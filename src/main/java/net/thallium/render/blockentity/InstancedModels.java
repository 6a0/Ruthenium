package net.thallium.render.blockentity;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.renderpearl.api.GpuFormat;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.CompiledRenderPipeline;
import com.mojang.renderpearl.api.pipeline.DepthStencilState;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.minecraft.client.model.Model;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.object.armorstand.ArmorStandModel;
import net.minecraft.client.renderer.BindGroupLayouts;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.EnchantTableRenderer;
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.blockentity.state.EnchantTableRenderState;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.client.renderer.texture.UvMapping;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.Identifier;
import net.minecraft.util.LightCoordsUtil;
import net.minecraft.util.Mth;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.EnchantingTableBlockEntity;
import net.thallium.Initializer;
import net.thallium.interfaces.CachedBlockEntityLight;
import net.thallium.render.entity.ModelCulling;
import net.thallium.render.entity.ModelGeometryCache;
import org.joml.Matrix4f;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Set;

/**
 * Draws models whose geometry only differs by their pose as GPU instances: the closed books of enchanting tables,
 * which spin and bob every frame (~5k of them facing the chest cube), and armor stands posed alike (the ~190 in
 * the benchmark all are).
 * <p>
 * The model is captured once in model space ({@link ModelGeometryCache#modelSpaceKey}; all faces, exact repeats
 * left out) into a vertex buffer. Each frame the extraction runs the real renderer into a
 * {@link ModelSubmitCapture} instead of submitting it, and stores the captured pose (camera-relative, rotation
 * only) and light as one instance. Once a closed book has been captured, the others skip the extraction and the
 * capture: their pose and light are computed from the block entity like the renderer does ({@link #tryAddClosedBook}).
 * Armor stands are taken from their prepared model submits instead: after
 * {@code setupAnim}, the transforms of all their parts ({@link #snapshot}) key the group, whose mesh is the
 * model rendered once with the identity pose. The instances are drawn after the world's solid features with
 * instanced copies of vanilla's entity pipelines, whose vertex shader applies the instance's pose to the position
 * and normal. The only difference to vanilla is rounding: the normal is quantized before the transform instead of
 * after it.
 * <p>
 * Only used from the render thread, except {@link #clear}.
 */
public final class InstancedModels {
    public static final VertexFormat INSTANCE_FORMAT = VertexFormat.builder(1)
            .addAttribute("InstanceRow0", GpuFormat.RGBA32_FLOAT)
            .addAttribute("InstanceRow1", GpuFormat.RGBA32_FLOAT)
            .addAttribute("InstanceRow2", GpuFormat.RGBA32_FLOAT)
            .addAttribute("InstanceLight", GpuFormat.RG16_SINT)
            .build();

    /** Instanced copies of vanilla's entity_solid, entity_cutout and entity_cutout_cull */
    public static final RenderPipeline SOLID = instancedPipeline("entity_solid", false, false, true);
    public static final RenderPipeline CUTOUT = instancedPipeline("entity_cutout", true, true, false);
    public static final RenderPipeline CUTOUT_CULL = instancedPipeline("entity_cutout_cull", true, false, true);
    public static final List<RenderPipeline> PIPELINES = List.of(SOLID, CUTOUT, CUTOUT_CULL);

    /** Models drawn from their prepared submits when their parts are posed like an existing group */
    private static final Set<Class<?>> POSED_MODELS = Set.of(ArmorStandModel.class);
    private static final int MAX_GROUPS_PER_MODEL = 32;

    private static final int INSTANCE_SIZE = INSTANCE_FORMAT.getVertexSize();
    /** Tolerance of the rotation test on the pose's linear part */
    private static final float ROTATION_TOLERANCE = 1.0e-4f;

    private static final List<Group> GROUPS = new ArrayList<>();
    private static float[] snapshotScratch = new float[0];
    private static final ModelSubmitCapture CAPTURE = new ModelSubmitCapture();
    private static final PoseStack POSE_STACK = new PoseStack();
    private static final PoseStack.Pose IDENTITY_POSE = new PoseStack().last();
    /** The group of closed books and the renderer that submitted it, for {@link #tryAddClosedBook} */
    private static @Nullable Group closedBookGroup;
    private static @Nullable EnchantTableRenderer closedBookRenderer;
    /** The enchanting table renderer of this frame, looked up on the first table */
    private static @Nullable BlockEntityRenderer<?, ?> frameTableRenderer;
    private static final Matrix4f BOOK_POSE = new Matrix4f();
    /**
     * The sine and cosine {@code Matrix4f.rotateZ} computes for the book's constant 80 degree tilt (with JOML's own
     * functions, so the same values); on a matrix that isn't a pure translation it then calls {@code rotateTowardsXY}
     */
    private static final float BOOK_TILT = 80.0F * (float) (Math.PI / 180.0);
    private static final float BOOK_TILT_SIN = org.joml.Math.sin(BOOK_TILT);
    private static final float BOOK_TILT_COS = org.joml.Math.cosFromSin(BOOK_TILT_SIN, BOOK_TILT);
    /**
     * Bumped whenever a section is marked dirty (block and light changes), which invalidates the light cached in
     * enchanting tables ({@link CachedBlockEntityLight}); 0 means not cached
     */
    private static volatile int lightGeneration = 1;
    private static int frameLightStamp;
    private static volatile boolean clearPending;
    private static @Nullable Boolean pipelineAvailable;
    private static @Nullable GpuBufferSlice transform;
    private static boolean drawPending;

    private InstancedModels() {
    }

    private static RenderPipeline instancedPipeline(String name, boolean cutout, boolean perFaceLighting, boolean cull) {
        RenderPipeline.Builder builder = RenderPipeline.builder()
                .withBindGroupLayout(BindGroupLayouts.GLOBALS)
                .withBindGroupLayout(BindGroupLayouts.PROJECTION)
                .withBindGroupLayout(BindGroupLayouts.DYNAMIC_TRANSFORMS)
                .withBindGroupLayout(BindGroupLayouts.FOG)
                .withVertexShader(Identifier.fromNamespaceAndPath("thallium", "core/instanced_entity"))
                .withFragmentShader(Identifier.withDefaultNamespace("core/entity"))
                .withBindGroupLayout(BindGroupLayouts.SAMPLER0)
                .withVertexBinding(0, DefaultVertexFormat.ENTITY)
                .withVertexBinding(1, INSTANCE_FORMAT)
                .withPrimitiveTopology(PrimitiveTopology.QUADS)
                .withDepthStencilState(DepthStencilState.DEFAULT)
                .withBindGroupLayout(BindGroupLayouts.LIGHTING)
                .withBindGroupLayout(BindGroupLayouts.SAMPLER2)
                .withLocation(Identifier.fromNamespaceAndPath("thallium", "pipeline/instanced_" + name));
        if (cutout) {
            builder.withShaderDefine("ALPHA_CUTOUT", 0.1F);
        }
        if (perFaceLighting) {
            builder.withShaderDefine("PER_FACE_LIGHTING");
        }
        return builder.withBindGroupLayout(BindGroupLayouts.SAMPLER1)
                .withCull(cull)
                .withColorTargetState(ColorTargetState.DEFAULT)
                .build();
    }

    private static @Nullable RenderPipeline instancedPipeline(RenderPipeline vanilla) {
        if (vanilla == RenderPipelines.ENTITY_SOLID) {
            return SOLID;
        } else if (vanilla == RenderPipelines.ENTITY_CUTOUT) {
            return CUTOUT;
        } else if (vanilla == RenderPipelines.ENTITY_CUTOUT_CULL) {
            return CUTOUT_CULL;
        }
        return null;
    }

    private static final class Group {
        final Model<?> model;
        /** A {@link ModelGeometryCache#modelSpaceKey}, or the part transforms ({@link #snapshot}) */
        final Object key;
        final RenderPipeline pipeline;
        final RenderType renderType;
        final @Nullable UvMapping uvMapping;
        final int overlay;
        final int color;
        @Nullable GpuBuffer mesh;
        int indexCount;
        boolean failed;

        ByteBuffer instances = MemoryUtil.memAlloc(64 * 52);
        int count;
        @Nullable MappableRingBuffer ring;
        @Nullable GpuBufferSlice instanceSlice;
        @Nullable PreparedRenderType prepared;

        Group(Model<?> model, Object key, RenderPipeline pipeline, RenderType renderType, @Nullable UvMapping uvMapping, int overlay, int color) {
            this.model = model;
            this.key = key;
            this.pipeline = pipeline;
            this.renderType = renderType;
            this.uvMapping = uvMapping;
            this.overlay = overlay;
            this.color = color;
        }

        boolean matches(Model<?> model, Object key, RenderType renderType, @Nullable UvMapping uvMapping, int overlay, int color) {
            return this.model == model && this.renderType == renderType && this.uvMapping == uvMapping
                    && this.overlay == overlay && this.color == color
                    && (this.key == key || this.key instanceof float[] a && key instanceof float[] b && Arrays.equals(a, b));
        }

        void close() {
            if (this.mesh != null) {
                this.mesh.close();
                this.mesh = null;
            }
            if (this.ring != null) {
                this.ring.close();
                this.ring = null;
            }
            MemoryUtil.memFree(this.instances);
        }
    }

    /** Drops every group at the start of the next frame (level change, resource reload) */
    public static void clear() {
        clearPending = true;
        invalidateLight();
    }

    /** A section was marked dirty: block entity light may have changed */
    public static void invalidateLight() {
        int next = lightGeneration + 1;
        lightGeneration = next == 0 ? 1 : next;
    }

    public static void beginFrame() {
        if (clearPending) {
            clearPending = false;
            pipelineAvailable = null;
            GROUPS.forEach(Group::close);
            GROUPS.clear();
            closedBookGroup = null;
            closedBookRenderer = null;
        }
        frameTableRenderer = null;
        frameLightStamp = lightGeneration;
        for (Group group : GROUPS) {
            group.count = 0;
        }
    }

    /**
     * Adds the closed book of an enchanting table as an instance straight from the block entity, without a render
     * state and a captured submit: the pose and light are what {@link EnchantTableRenderer} extracts and submits
     * (the same float operations in the same order), and the group is the one the first closed book captured
     * through {@link #tryAdd(BlockEntityRenderDispatcher, BlockEntityRenderState, double, double, double, CameraRenderState)}.
     * Only for vanilla's block entity and renderer classes, a book that is shut ({@code open} and {@code oOpen} 0,
     * so the interpolated openness is 0) and no breaking progress.
     *
     * @return false if the block entity must be extracted as usual
     */
    public static boolean tryAddClosedBook(BlockEntityRenderDispatcher dispatcher, EnchantingTableBlockEntity table, float partialTicks,
                                           double camX, double camY, double camZ) {
        if (table.open != 0.0f || table.oOpen != 0.0f || table.getClass() != EnchantingTableBlockEntity.class
                || closedBookGroup == null || !Initializer.CONFIG.instancedModels) {
            return false;
        }
        BlockEntityRenderer<?, ?> renderer = frameTableRenderer;
        if (renderer == null) {
            renderer = frameTableRenderer = dispatcher.getRenderer(table);
        }
        Level level = table.getLevel();
        BlockPos pos = table.getBlockPos();
        if (renderer != closedBookRenderer || level == null) {
            return false;
        }
        // The block state check and the light only change with the blocks and light around the table
        CachedBlockEntityLight cache = (CachedBlockEntityLight) table;
        int light;
        if (cache.thallium$getLightStamp() == frameLightStamp) {
            light = cache.thallium$getLight();
        } else {
            if (!table.getType().isValid(table.getBlockState())) {
                return false;
            }
            light = LightCoordsUtil.getLightCoords(level, pos);
            cache.thallium$setLight(frameLightStamp, light);
        }
        // BlockEntityRenderer.shouldRender
        double dx = pos.getX() + 0.5 - camX;
        double dy = pos.getY() + 0.5 - camY;
        double dz = pos.getZ() + 0.5 - camZ;
        if (!(dx * dx + dy * dy + dz * dz < 64.0 * 64.0)) {
            return false;
        }

        // EnchantTableRenderer.extractRenderState
        float time = table.time + partialTicks;
        float or = table.rot - table.oRot;
        while (or >= (float) Math.PI) {
            or -= (float) (Math.PI * 2);
        }
        while (or < (float) -Math.PI) {
            or += (float) (Math.PI * 2);
        }
        float yRot = table.oRot + or * partialTicks;

        // The extraction's translation, then EnchantTableRenderer.submit
        Matrix4f m = BOOK_POSE.identity();
        m.translate((float) (pos.getX() - camX), (float) (pos.getY() - camY), (float) (pos.getZ() - camZ));
        m.translate(0.5F, 0.75F, 0.5F);
        m.translate(0.0F, 0.1F + Mth.sin(time * 0.1F) * 0.01F, 0.0F);
        m.rotateY(-yRot);
        if ((m.properties() & (Matrix4f.PROPERTY_IDENTITY | Matrix4f.PROPERTY_TRANSLATION)) == 0) {
            // What rotateZ(BOOK_TILT) does on such a matrix, without computing the sine and cosine again
            m.rotateTowardsXY(BOOK_TILT_SIN, BOOK_TILT_COS);
        } else {
            m.rotateZ(BOOK_TILT);
        }
        addInstance(closedBookGroup, m, light);
        return true;
    }

    /**
     * Turns an extracted block entity into an instance instead of a render state to submit.
     *
     * @return false if the render state must be submitted as usual
     */
    @SuppressWarnings({"rawtypes", "unchecked"})
    public static boolean tryAdd(BlockEntityRenderDispatcher dispatcher, BlockEntityRenderState state, double camX, double camY,
                                 double camZ, CameraRenderState camera) {
        if (!Initializer.CONFIG.instancedModels || !(state instanceof EnchantTableRenderState) || state.breakProgress != null) {
            return false;
        }
        BlockEntityRenderer renderer = dispatcher.getRenderer(state);
        if (renderer == null || renderer.getClass() != EnchantTableRenderer.class || !isPipelineAvailable()) {
            return false;
        }

        BlockPos pos = state.blockPos;
        POSE_STACK.pushPose();
        POSE_STACK.translate(pos.getX() - camX, pos.getY() - camY, pos.getZ() - camZ);
        CAPTURE.reset();
        renderer.submit(state, POSE_STACK, CAPTURE, camera);
        POSE_STACK.popPose();
        if (!CAPTURE.isSingleModel()) {
            return false;
        }

        ModelSubmitCapture.ModelRecord capture = CAPTURE.model(0);
        Object key;
        RenderPipeline pipeline;
        if (capture.outlineColor != 0 || capture.renderType == null
                || (pipeline = instancedPipeline(capture.renderType.pipeline())) == null
                || (key = ModelGeometryCache.modelSpaceKey(capture.state)) == null || !isOrthogonal(capture.pose.pose())) {
            return false;
        }

        Group group = findGroup(capture.model, capture.state, key, pipeline, capture.renderType, capture.uvMapping,
                capture.overlayCoords, capture.tintedColor);
        if (group == null) {
            return false;
        }
        addInstance(group, capture.pose.pose(), capture.lightCoords);
        // Everything but the pose and light is a constant of the renderer
        closedBookGroup = group;
        closedBookRenderer = (EnchantTableRenderer) renderer;
        return true;
    }

    /**
     * Turns a prepared model submit into an instance when its model is one of {@link #POSED_MODELS} and its parts
     * end up posed like those of an existing group (or a new one).
     *
     * @return false if the model has to be written as usual
     */
    @SuppressWarnings({"rawtypes", "unchecked"})
    public static boolean tryAdd(ModelFeatureRenderer.Submit<?> submit) {
        Model model = submit.model();
        RenderPipeline pipeline;
        if (!Initializer.CONFIG.instancedModels || !POSED_MODELS.contains(model.getClass()) || submit.sheetedDecalPose() != null
                || (pipeline = instancedPipeline(submit.renderType().pipeline())) == null
                || !isOrthogonal(submit.pose().pose()) || !isPipelineAvailable()) {
            return false;
        }

        model.setupAnim(submit.state());
        float[] snapshot = snapshot(model);
        Group group = findGroup(model, submit.state(), snapshot, pipeline, submit.renderType(), submit.uvMapping(),
                submit.overlayCoords(), submit.tintedColor());
        if (group == null) {
            return false;
        }
        addInstance(group, submit.pose().pose(), submit.lightCoords());
        return true;
    }

    /** The transforms and visibility of every part, which (with the model) decide its model space geometry */
    private static float[] snapshot(Model<?> model) {
        List<ModelPart> parts = model.allParts();
        int size = parts.size() * 11;
        if (snapshotScratch.length != size) {
            snapshotScratch = new float[size];
        }
        float[] s = snapshotScratch;
        int o = 0;
        for (int i = 0; i < parts.size(); i++) {
            ModelPart part = parts.get(i);
            s[o] = part.x;
            s[o + 1] = part.y;
            s[o + 2] = part.z;
            s[o + 3] = part.xRot;
            s[o + 4] = part.yRot;
            s[o + 5] = part.zRot;
            s[o + 6] = part.xScale;
            s[o + 7] = part.yScale;
            s[o + 8] = part.zScale;
            s[o + 9] = part.visible ? 1.0f : 0.0f;
            s[o + 10] = part.skipDraw ? 1.0f : 0.0f;
            o += 11;
        }
        return s;
    }

    private static void addInstance(Group group, Matrix4f m, int lightCoords) {
        if (group.instances.capacity() < (group.count + 1) * INSTANCE_SIZE) {
            group.instances = MemoryUtil.memRealloc(group.instances, group.instances.capacity() * 2);
        }
        long ptr = MemoryUtil.memAddress0(group.instances) + (long) group.count * INSTANCE_SIZE;
        putRow(ptr, m.m00(), m.m10(), m.m20(), m.m30());
        putRow(ptr + 16, m.m01(), m.m11(), m.m21(), m.m31());
        putRow(ptr + 32, m.m02(), m.m12(), m.m22(), m.m32());
        MemoryUtil.memPutInt(ptr + 48, lightCoords);
        group.count++;
    }

    private static void putRow(long ptr, float x, float y, float z, float w) {
        MemoryUtil.memPutFloat(ptr, x);
        MemoryUtil.memPutFloat(ptr + 4, y);
        MemoryUtil.memPutFloat(ptr + 8, z);
        MemoryUtil.memPutFloat(ptr + 12, w);
    }

    /**
     * The shader transforms normals with the pose itself, which is only right for rotations and reflections
     * (vanilla's normal matrix is then the same; {@code PoseStack.scale(-1, -1, 1)} flips its signs too)
     */
    private static boolean isOrthogonal(Matrix4f m) {
        float c0 = m.m00() * m.m00() + m.m01() * m.m01() + m.m02() * m.m02();
        float c1 = m.m10() * m.m10() + m.m11() * m.m11() + m.m12() * m.m12();
        float c2 = m.m20() * m.m20() + m.m21() * m.m21() + m.m22() * m.m22();
        return Math.abs(c0 - 1.0f) <= ROTATION_TOLERANCE && Math.abs(c1 - 1.0f) <= ROTATION_TOLERANCE
                && Math.abs(c2 - 1.0f) <= ROTATION_TOLERANCE
                && Math.abs(m.m00() * m.m10() + m.m01() * m.m11() + m.m02() * m.m12()) <= ROTATION_TOLERANCE
                && Math.abs(m.m00() * m.m20() + m.m01() * m.m21() + m.m02() * m.m22()) <= ROTATION_TOLERANCE
                && Math.abs(m.m10() * m.m20() + m.m11() * m.m21() + m.m12() * m.m22()) <= ROTATION_TOLERANCE
                && m.m03() == 0.0f && m.m13() == 0.0f && m.m23() == 0.0f && m.m33() == 1.0f;
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static @Nullable Group findGroup(Model model, Object state, Object key, RenderPipeline pipeline, RenderType renderType,
                                             @Nullable UvMapping uvMapping, int overlay, int color) {
        int modelGroups = 0;
        for (Group group : GROUPS) {
            if (group.matches(model, key, renderType, uvMapping, overlay, color)) {
                return group.failed ? null : group;
            }
            if (group.model == model) {
                modelGroups++;
            }
        }
        if (modelGroups >= MAX_GROUPS_PER_MODEL) {
            return null;
        }

        Object groupKey = key instanceof float[] snapshot ? snapshot.clone() : key;
        Group group = new Group(model, groupKey, pipeline, renderType, uvMapping, overlay, color);
        GROUPS.add(group);

        try (ByteBufferBuilder bytes = new ByteBufferBuilder(1 << 12)) {
            BufferBuilder builder = new BufferBuilder(bytes, PrimitiveTopology.QUADS, DefaultVertexFormat.ENTITY);
            // Light comes from the instance; all faces, the pipeline culls
            boolean written;
            if (key instanceof float[]) {
                ModelCulling.endModel();
                try {
                    // The parts are posed already (the snapshot was taken after setupAnim)
                    model.root().render(new PoseStack(), uvMapping != null ? uvMapping.wrap(builder) : builder, 0, overlay, color);
                    written = true;
                } finally {
                    ModelCulling.beginModel(renderType);
                }
            } else {
                written = ModelGeometryCache.writeModelSpace(model, state, key, IDENTITY_POSE, builder, 0, overlay, color, uvMapping,
                        false, true);
            }
            try (MeshData data = written ? builder.build() : null) {
                if (data == null) {
                    group.failed = true;
                    return null;
                }
                group.mesh = RenderSystem.getDevice().createBuffer(() -> "Thallium instanced model", GpuBuffer.USAGE_VERTEX, data.vertexBuffer());
                group.indexCount = data.drawState().vertexCount() / 4 * 6;
            }
        }
        return group;
    }

    private static boolean isPipelineAvailable() {
        Boolean available = pipelineAvailable;
        if (available == null) {
            available = true;
            for (RenderPipeline pipeline : PIPELINES) {
                available &= RenderSystem.getCompiledPipelineNullable(pipeline) != null;
            }
            pipelineAvailable = available;
        }
        return available;
    }

    /** Uploads this frame's instances (after the features are prepared, before the passes) */
    public static void prepareDraws() {
        drawPending = false;
        int maxIndexCount = 0;
        for (Group group : GROUPS) {
            group.instanceSlice = null;
            if (group.ring != null) {
                // Fences the previous frame's use of the current buffer
                group.ring.rotate();
            }
            if (group.count == 0 || group.mesh == null) {
                continue;
            }

            int size = group.count * INSTANCE_SIZE;
            if (group.ring == null || group.ring.size() < size) {
                if (group.ring != null) {
                    group.ring.close();
                }
                int capacity = Math.max(size * 2, 64 * INSTANCE_SIZE);
                group.ring = new MappableRingBuffer(() -> "Thallium model instances",
                        GpuBuffer.USAGE_VERTEX | GpuBuffer.USAGE_MAP_WRITE, capacity);
            }

            GpuBuffer buffer = group.ring.currentBuffer();
            try (GpuBufferSlice.MappedView view = buffer.slice(0, size).map(false, true)) {
                MemoryUtil.memCopy(MemoryUtil.memAddress0(group.instances), MemoryUtil.memAddress(view.data()), size);
            }
            group.instanceSlice = buffer.slice(0, size);
            group.prepared = group.renderType.prepare();
            maxIndexCount = Math.max(maxIndexCount, group.indexCount);
            drawPending = true;
        }

        if (drawPending) {
            transform = RenderSystem.getDynamicUniforms().writeTransform(RenderSystem.getModelViewMatrixCopy());
            RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS).getBuffer(maxIndexCount);
        }
    }

    /** Draws the instances into the world's solid pass */
    public static void draw(RenderPass renderPass) {
        if (!drawPending) {
            return;
        }

        renderPass.pushDebugGroup(() -> "Thallium instanced models");
        RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS);

        for (Group group : GROUPS) {
            if (group.instanceSlice == null) {
                continue;
            }
            CompiledRenderPipeline pipeline = RenderSystem.getCompiledPipelineNullable(group.pipeline);
            if (pipeline == null) {
                continue;
            }
            renderPass.setPipeline(pipeline);
            RenderSystem.bindDefaultUniforms(renderPass);
            renderPass.setUniform("DynamicTransforms", transform);
            renderPass.setIndexBuffer(indices.getBuffer(), indices.type());
            for (PreparedRenderType.Texture texture : group.prepared.textures()) {
                renderPass.setUniform(texture.name(), texture.textureView(), texture.sampler());
            }
            renderPass.setVertexBuffer(0, group.mesh.slice());
            renderPass.setVertexBuffer(1, group.instanceSlice);
            renderPass.drawIndexed(group.indexCount, group.count, 0, 0, 0);
        }
        renderPass.popDebugGroup();
    }
}
