package net.vulkanmod.render.blockentity;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.CompiledRenderPipeline;
import com.mojang.renderpearl.api.pipeline.DepthStencilState;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import it.unimi.dsi.fastutil.longs.Long2ObjectMap;
import it.unimi.dsi.fastutil.longs.Long2ObjectOpenHashMap;
import it.unimi.dsi.fastutil.longs.LongOpenHashSet;
import it.unimi.dsi.fastutil.objects.ObjectIterator;
import net.minecraft.client.model.Model;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.BindGroupLayouts;
import net.minecraft.client.renderer.DynamicGpuData;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.ChestRenderer;
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.chunk.SectionMesh;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.core.BlockPos;
import net.minecraft.core.SectionPos;
import net.minecraft.resources.Identifier;
import net.minecraft.world.level.block.ChestBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.level.block.entity.EnderChestBlockEntity;
import net.minecraft.world.level.block.entity.TrappedChestBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.ChestType;
import net.minecraft.world.phys.Vec3;
import net.vulkanmod.Initializer;
import net.vulkanmod.interfaces.IdleTicker;
import net.vulkanmod.render.entity.ModelGeometryCache;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.joml.Vector4f;
import org.jspecify.annotations.Nullable;

import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

/**
 * Keeps the geometry of closed chests on the GPU across frames instead of extracting, submitting and building it
 * every frame.
 * <p>
 * Per render section, the vertices of all chests that are at rest (vanilla chest classes and renderer, lid
 * closed and staying closed, no breaking progress, one cacheable model in {@code entity_cutout_cull}) are built
 * once, relative to the section origin, by running the real renderer into a {@link ModelSubmitCapture} and writing
 * the captured model with {@link ModelGeometryCache} (all faces; the GPU culls back faces as before). The
 * extraction then skips those chests, and the batch is drawn right after the world's solid features with
 * {@link #PIPELINE}: vanilla's entity_cutout_cull with a vertex shader that adds {@code ModelOffset} (section
 * origin minus camera) to the position.
 * <p>
 * A batch is rebuilt when the section's compiled mesh changes (block entity list), when the section or a neighbour
 * is marked dirty (block and light changes, which is where a chest's light comes from), when a chest's lid starts
 * to move (block events), when breaking progress changes, and when a left out chest comes to rest. Batches of
 * sections only partly within the chest view distance aren't drawn; their chests take the vanilla path that frame.
 * <p>
 * Only used from the render thread, except {@link #markDirty} and {@link #clear}.
 */
public final class RetainedChests {
    public static final RenderPipeline PIPELINE = RenderPipeline.builder()
            .withBindGroupLayout(BindGroupLayouts.GLOBALS)
            .withBindGroupLayout(BindGroupLayouts.PROJECTION)
            .withBindGroupLayout(BindGroupLayouts.DYNAMIC_TRANSFORMS)
            .withBindGroupLayout(BindGroupLayouts.FOG)
            .withVertexShader(Identifier.fromNamespaceAndPath("vulkanmod", "core/retained_entity"))
            .withFragmentShader(Identifier.withDefaultNamespace("core/entity"))
            .withBindGroupLayout(BindGroupLayouts.SAMPLER0)
            .withVertexBinding(0, DefaultVertexFormat.ENTITY)
            .withPrimitiveTopology(PrimitiveTopology.QUADS)
            .withDepthStencilState(DepthStencilState.DEFAULT)
            .withBindGroupLayout(BindGroupLayouts.LIGHTING)
            .withBindGroupLayout(BindGroupLayouts.SAMPLER2)
            .withLocation(Identifier.fromNamespaceAndPath("vulkanmod", "pipeline/retained_entity_cutout_cull"))
            .withShaderDefine("ALPHA_CUTOUT", 0.1F)
            .withBindGroupLayout(BindGroupLayouts.SAMPLER1)
            .withColorTargetState(ColorTargetState.DEFAULT)
            .build();

    /** Chests rebuilt per frame at most; sections over budget take the vanilla path until they are built */
    private static final int REBUILD_BUDGET = 2048;
    private static final int EVICT_AFTER_FRAMES = 1200;
    private static final Vector4f WHITE = new Vector4f(1.0F, 1.0F, 1.0F, 1.0F);
    private static final Matrix4f IDENTITY = new Matrix4f();

    private static final int NOT_CANDIDATE = 0;
    private static final int CANDIDATE = 1;
    private static final int MOVING = 2;

    private static final Long2ObjectOpenHashMap<Batch> BATCHES = new Long2ObjectOpenHashMap<>();
    /** Sections marked dirty, possibly from other threads; guarded by itself */
    private static final LongOpenHashSet PENDING_DIRTY = new LongOpenHashSet();
    private static volatile boolean clearPending;

    private static final List<Batch> VISIBLE = new ArrayList<>();
    private static final Map<RenderType, PreparedRenderType> PREPARED = new IdentityHashMap<>();
    private static GpuBufferSlice[] transforms = new GpuBufferSlice[0];
    private static int drawCount;

    private static final ModelSubmitCapture CAPTURE = new ModelSubmitCapture();
    private static final PoseStack POSE_STACK = new PoseStack();
    private static @Nullable ByteBufferBuilder bytes;
    private static @Nullable Boolean pipelineAvailable;
    private static int frame;
    private static int rebuiltThisFrame;
    private static double camX, camY, camZ;

    private RetainedChests() {
    }

    private static final class Batch {
        final int originX, originY, originZ;
        @Nullable SectionMesh mesh;
        boolean dirty;
        boolean[] members = new boolean[0];
        int memberCount;
        /** Chests left out because their lid (or their other half's lid) moves */
        List<BlockEntity> moving = List.of();
        @Nullable GpuBuffer buffer;
        int indexCount;
        @Nullable RenderType renderType;
        double viewDistance;
        int lastFrame;

        Batch(BlockPos origin) {
            this.originX = origin.getX();
            this.originY = origin.getY();
            this.originZ = origin.getZ();
        }

        void close() {
            if (this.buffer != null) {
                this.buffer.close();
                this.buffer = null;
            }
            this.memberCount = 0;
        }
    }

    public static void markDirty(int sectionX, int sectionY, int sectionZ) {
        synchronized (PENDING_DIRTY) {
            PENDING_DIRTY.add(SectionPos.asLong(sectionX, sectionY, sectionZ));
        }
    }

    public static void markDirty(BlockPos pos) {
        markDirty(SectionPos.blockToSectionCoord(pos.getX()), SectionPos.blockToSectionCoord(pos.getY()),
                SectionPos.blockToSectionCoord(pos.getZ()));
    }

    /** Drops every batch at the start of the next frame (level change, resource reload) */
    public static void clear() {
        clearPending = true;
    }

    /** Called before the visible block entities are extracted */
    public static void beginFrame(Vec3 cameraPos) {
        frame++;
        rebuiltThisFrame = 0;
        VISIBLE.clear();
        camX = cameraPos.x();
        camY = cameraPos.y();
        camZ = cameraPos.z();

        if (clearPending) {
            clearPending = false;
            pipelineAvailable = null;
            BATCHES.values().forEach(Batch::close);
            BATCHES.clear();
        }

        synchronized (PENDING_DIRTY) {
            if (!PENDING_DIRTY.isEmpty()) {
                // A chest's light can come from its other half in a neighbouring section
                PENDING_DIRTY.forEach((long key) -> {
                    int x = SectionPos.x(key), y = SectionPos.y(key), z = SectionPos.z(key);
                    markBatchDirty(x, y, z);
                    markBatchDirty(x - 1, y, z);
                    markBatchDirty(x + 1, y, z);
                    markBatchDirty(x, y - 1, z);
                    markBatchDirty(x, y + 1, z);
                    markBatchDirty(x, y, z - 1);
                    markBatchDirty(x, y, z + 1);
                });
                PENDING_DIRTY.clear();
            }
        }

        if ((frame & 255) == 0) {
            evict();
        }
    }

    private static void markBatchDirty(int x, int y, int z) {
        Batch batch = BATCHES.get(SectionPos.asLong(x, y, z));
        if (batch != null) {
            batch.dirty = true;
        }
    }

    private static void evict() {
        ObjectIterator<Long2ObjectMap.Entry<Batch>> iterator = BATCHES.long2ObjectEntrySet().fastIterator();
        while (iterator.hasNext()) {
            Batch batch = iterator.next().getValue();
            if (frame - batch.lastFrame > EVICT_AFTER_FRAMES) {
                batch.close();
                iterator.remove();
            }
        }
    }

    /**
     * Prepares the section's batch during extraction.
     *
     * @return the block entities (by list index) that the extraction must skip, or null to extract all of them
     */
    public static boolean @Nullable [] section(BlockPos origin, SectionMesh mesh, List<BlockEntity> blockEntities, ClientLevel level,
                                                BlockEntityRenderDispatcher dispatcher, float partialTick, Vec3 cameraPos,
                                                CameraRenderState cameraState) {
        if (!Initializer.CONFIG.retainedChests || !isPipelineAvailable()) {
            return null;
        }

        long key = SectionPos.asLong(SectionPos.blockToSectionCoord(origin.getX()), SectionPos.blockToSectionCoord(origin.getY()),
                SectionPos.blockToSectionCoord(origin.getZ()));
        Batch batch = BATCHES.get(key);
        if (batch == null) {
            batch = new Batch(origin);
            batch.dirty = true;
            BATCHES.put(key, batch);
        }
        batch.lastFrame = frame;

        if (!batch.dirty && batch.mesh == mesh) {
            for (BlockEntity moving : batch.moving) {
                if (candidate(moving, level) == CANDIDATE) {
                    batch.dirty = true;
                    break;
                }
            }
        }

        if (batch.dirty || batch.mesh != mesh) {
            if (rebuiltThisFrame >= REBUILD_BUDGET) {
                return null;
            }
            build(batch, mesh, blockEntities, level, dispatcher, partialTick, cameraPos, cameraState);
        }

        if (batch.memberCount == 0) {
            return null;
        }

        // BlockEntityRenderer.shouldRender: chest centres within the view distance of the camera
        double minX = batch.originX + 0.5, minY = batch.originY + 0.5, minZ = batch.originZ + 0.5;
        double maxX = minX + 15.0, maxY = minY + 15.0, maxZ = minZ + 15.0;
        double far = sq(Math.max(camX - minX, maxX - camX)) + sq(Math.max(camY - minY, maxY - camY)) + sq(Math.max(camZ - minZ, maxZ - camZ));
        double near = sq(Math.max(0.0, Math.max(minX - camX, camX - maxX))) + sq(Math.max(0.0, Math.max(minY - camY, camY - maxY)))
                + sq(Math.max(0.0, Math.max(minZ - camZ, camZ - maxZ)));
        double limit = batch.viewDistance * batch.viewDistance;
        if (far < limit) {
            VISIBLE.add(batch);
            return batch.members;
        } else if (near >= limit) {
            // None would be rendered
            return batch.members;
        }
        return null;
    }

    private static double sq(double v) {
        return v * v;
    }

    private static int candidate(BlockEntity blockEntity, ClientLevel level) {
        Class<?> type = blockEntity.getClass();
        if (type != ChestBlockEntity.class && type != TrappedChestBlockEntity.class && type != EnderChestBlockEntity.class) {
            return NOT_CANDIDATE;
        }
        if (blockEntity.isRemoved() || !((IdleTicker) blockEntity).vulkanmod$isTickIdle()) {
            return MOVING;
        }

        BlockState state = blockEntity.getBlockState();
        if (state.hasProperty(ChestBlock.TYPE) && state.getValue(ChestBlock.TYPE) != ChestType.SINGLE) {
            BlockEntity other = level.getBlockEntity(blockEntity.getBlockPos().relative(ChestBlock.getConnectedDirection(state)));
            if (other != null && !(other instanceof IdleTicker idle && idle.vulkanmod$isTickIdle())) {
                return MOVING;
            }
        }
        return CANDIDATE;
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static void build(Batch batch, SectionMesh mesh, List<BlockEntity> blockEntities, ClientLevel level,
                              BlockEntityRenderDispatcher dispatcher, float partialTick, Vec3 cameraPos, CameraRenderState cameraState) {
        batch.close();
        batch.mesh = mesh;
        batch.dirty = false;
        batch.renderType = null;
        batch.moving = List.of();

        int size = blockEntities.size();
        boolean[] members = batch.members.length == size ? batch.members : new boolean[size];
        java.util.Arrays.fill(members, false);
        batch.members = members;

        if (bytes == null) {
            bytes = new ByteBufferBuilder(1 << 16);
        }
        BufferBuilder builder = null;
        List<BlockEntity> moving = null;
        int count = 0;
        double viewDistance = -1.0;

        for (int i = 0; i < size; i++) {
            BlockEntity blockEntity = blockEntities.get(i);
            int candidate = candidate(blockEntity, level);
            if (candidate == MOVING) {
                if (moving == null) {
                    moving = new ArrayList<>();
                }
                moving.add(blockEntity);
                continue;
            } else if (candidate != CANDIDATE) {
                continue;
            }

            BlockPos pos = blockEntity.getBlockPos();
            if (!blockEntity.hasLevel() || !blockEntity.getType().isValid(blockEntity.getBlockState())
                    || level.destructionProgress().containsKey(pos.asLong())) {
                continue;
            }

            BlockEntityRenderer renderer = dispatcher.getRenderer(blockEntity);
            if (renderer == null || renderer.getClass() != ChestRenderer.class || renderer.shouldRenderOffScreen()) {
                continue;
            }
            double distance = renderer.getViewDistance();
            if (viewDistance >= 0.0 && distance != viewDistance) {
                continue;
            }

            BlockEntityRenderState state = renderer.createRenderState();
            renderer.extractRenderState(blockEntity, state, partialTick, cameraPos, null);

            POSE_STACK.pushPose();
            POSE_STACK.translate((float) (pos.getX() - batch.originX), (float) (pos.getY() - batch.originY), (float) (pos.getZ() - batch.originZ));
            CAPTURE.reset();
            renderer.submit(state, POSE_STACK, CAPTURE, cameraState);
            POSE_STACK.popPose();

            ModelSubmitCapture capture = CAPTURE;
            if (!capture.isSingleModel() || capture.outlineColor != 0 || capture.renderType == null
                    || capture.renderType.pipeline() != RenderPipelines.ENTITY_CUTOUT_CULL
                    || !ModelGeometryCache.isCacheable(capture.state)
                    || batch.renderType != null && batch.renderType != capture.renderType) {
                continue;
            }

            if (builder == null) {
                builder = new BufferBuilder(bytes, PrimitiveTopology.QUADS, DefaultVertexFormat.ENTITY);
            }
            if (!ModelGeometryCache.write((Model) capture.model, capture.state, capture.pose, builder, capture.lightCoords,
                    capture.overlayCoords, capture.tintedColor, capture.uvMapping, false)) {
                continue;
            }

            batch.renderType = capture.renderType;
            viewDistance = distance;
            members[i] = true;
            count++;
        }

        rebuiltThisFrame += count;
        batch.moving = moving != null ? moving : List.of();
        batch.viewDistance = viewDistance;
        if (count == 0 || builder == null) {
            if (builder != null) {
                try (MeshData data = builder.build()) {
                    // Nothing to keep
                }
            }
            return;
        }

        try (MeshData data = builder.build()) {
            if (data == null) {
                java.util.Arrays.fill(members, false);
                return;
            }
            batch.buffer = RenderSystem.getDevice().createBuffer(() -> "VulkanMod retained chests", GpuBuffer.USAGE_VERTEX, data.vertexBuffer());
            batch.indexCount = data.drawState().vertexCount() / 4 * 6;
            batch.memberCount = count;
        }
    }

    private static boolean isPipelineAvailable() {
        Boolean available = pipelineAvailable;
        if (available == null) {
            available = RenderSystem.getCompiledPipelineNullable(PIPELINE) != null;
            pipelineAvailable = available;
        }
        return available;
    }

    /** Writes the batches' transforms while the frame is prepared (after the features, before the passes) */
    public static void prepareDraws() {
        drawCount = 0;
        PREPARED.clear();
        int count = VISIBLE.size();
        if (count == 0) {
            return;
        }

        Matrix4f modelView = RenderSystem.getModelViewMatrixCopy();
        DynamicGpuData.Transform[] data = new DynamicGpuData.Transform[count];
        int maxIndexCount = 0;
        for (int i = 0; i < count; i++) {
            Batch batch = VISIBLE.get(i);
            // Fresh vectors: the uniform storage compares against the last record it wrote
            Vector3f offset = new Vector3f((float) (batch.originX - camX), (float) (batch.originY - camY), (float) (batch.originZ - camZ));
            data[i] = new DynamicGpuData.Transform(modelView, WHITE, offset, IDENTITY);
            maxIndexCount = Math.max(maxIndexCount, batch.indexCount);
            PREPARED.computeIfAbsent(batch.renderType, RenderType::prepare);
        }
        transforms = RenderSystem.getDynamicUniforms().writeTransforms(data);
        RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS).getBuffer(maxIndexCount);
        drawCount = count;
    }

    /** Draws the batches into the world's solid pass */
    public static void draw(RenderPass renderPass) {
        if (drawCount == 0) {
            return;
        }
        CompiledRenderPipeline pipeline = RenderSystem.getCompiledPipelineNullable(PIPELINE);
        if (pipeline == null) {
            return;
        }

        renderPass.pushDebugGroup(() -> "VulkanMod retained chests");
        renderPass.setPipeline(pipeline);
        RenderSystem.bindDefaultUniforms(renderPass);
        RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS);
        renderPass.setIndexBuffer(indices.getBuffer(), indices.type());

        RenderType boundType = null;
        for (int i = 0; i < drawCount; i++) {
            Batch batch = VISIBLE.get(i);
            if (batch.renderType != boundType) {
                boundType = batch.renderType;
                for (PreparedRenderType.Texture texture : PREPARED.get(boundType).textures()) {
                    renderPass.setUniform(texture.name(), texture.textureView(), texture.sampler());
                }
            }
            renderPass.setUniform("DynamicTransforms", transforms[i]);
            renderPass.setVertexBuffer(0, batch.buffer.slice());
            renderPass.drawIndexed(batch.indexCount, 1, 0, 0, 0);
        }
        renderPass.popDebugGroup();
    }
}
