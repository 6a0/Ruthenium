package net.vulkanmod.render.blockentity;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.CompiledRenderPipeline;
import com.mojang.renderpearl.api.pipeline.DepthStencilState;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import it.unimi.dsi.fastutil.ints.IntArrayList;
import it.unimi.dsi.fastutil.longs.Long2ObjectMap;
import it.unimi.dsi.fastutil.longs.Long2ObjectOpenHashMap;
import it.unimi.dsi.fastutil.longs.LongOpenHashSet;
import it.unimi.dsi.fastutil.objects.ObjectIterator;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.font.TextRenderable;
import net.minecraft.client.model.Model;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.BindGroupLayouts;
import net.minecraft.client.renderer.DynamicGpuData;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.ChestRenderer;
import net.minecraft.client.renderer.blockentity.DecoratedPotRenderer;
import net.minecraft.client.renderer.blockentity.HangingSignRenderer;
import net.minecraft.client.renderer.blockentity.ShulkerBoxRenderer;
import net.minecraft.client.renderer.blockentity.SkullBlockRenderer;
import net.minecraft.client.renderer.blockentity.StandingSignRenderer;
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.chunk.SectionMesh;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.rendertype.TextureTransform;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.core.BlockPos;
import net.minecraft.core.SectionPos;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FormattedText;
import net.minecraft.network.chat.Style;
import net.minecraft.resources.Identifier;
import net.minecraft.util.ARGB;
import net.minecraft.world.level.block.AbstractSkullBlock;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.ChestBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.level.block.entity.DecoratedPotBlockEntity;
import net.minecraft.world.level.block.entity.EnderChestBlockEntity;
import net.minecraft.world.level.block.entity.HangingSignBlockEntity;
import net.minecraft.world.level.block.entity.ShulkerBoxBlockEntity;
import net.minecraft.world.level.block.entity.SignBlockEntity;
import net.minecraft.world.level.block.entity.SignText;
import net.minecraft.world.level.block.entity.SignTextSlot;
import net.minecraft.world.level.block.entity.SkullBlockEntity;
import net.minecraft.world.level.block.entity.TrappedChestBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.ChestType;
import net.minecraft.world.phys.Vec3;
import net.vulkanmod.Initializer;
import net.vulkanmod.render.chunk.SectionFaceGroups;
import net.vulkanmod.interfaces.IdleTicker;
import net.vulkanmod.render.entity.ModelGeometryCache;
import net.vulkanmod.render.text.SignTextCache;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.joml.Vector4f;
import org.jspecify.annotations.Nullable;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.function.Consumer;

/**
 * Keeps the geometry of block entities at rest on the GPU across frames instead of extracting, submitting and
 * building it every frame.
 * <p>
 * Handled are the vanilla classes and renderers of chests (lid closed and staying closed), shulker boxes (closed),
 * decorated pots (not wobbling), skulls (not animating, no player skin) and signs (no glowing or obfuscated text).
 * Per render section, their vertices are built once, relative to the section origin, by running the real renderer
 * into a {@link ModelSubmitCapture}: every submitted model (pots submit their parts as models) is written with all
 * faces, through {@link ModelGeometryCache} for rest pose models and with vanilla's model rendering otherwise, and
 * every submitted text the way {@code TextFeatureRenderer} writes it, into one vertex buffer per render type. A block
 * entity is only kept if all its submits are models in a cutout or solid entity pipeline with the default texture
 * transform and no outline, or opaque polygon offset text without background or outline: text vanilla itself draws
 * with the solid features, where the order doesn't matter. The extraction then skips those block entities, and the
 * batches are drawn right after the world's solid features with copies of vanilla's entity and text pipelines
 * ({@link #PIPELINES}) whose vertex shader adds {@code ModelOffset} (section origin minus camera) to the position;
 * the render type's textures and its layering transform (the z offset of skulls) are applied as vanilla does.
 * Buffers of pipelines that cull back faces are sorted by facing and plane ({@link SectionFaceGroups#sortByPlanes}),
 * and a frame draws only the quads whose plane the eye can be in front of (switch {@code modelBackFaceCulling}); the
 * GPU would cull the others after running their vertex shader, which is most of the batches' GPU cost.
 * <p>
 * A batch is rebuilt when the section's compiled mesh changes (block entity list), when the section or a neighbour
 * is marked dirty (block and light changes, which is where the light comes from), on block events of chests,
 * shulker boxes and pots (lids, wobbles), on block entity data updates and sign text changes, when breaking progress
 * changes, and when a left out block entity comes to rest. Everything is dropped when fonts reload or the text
 * filtering setting changes. Batches of sections only partly within the view distance aren't drawn; their
 * block entities take the vanilla path that frame.
 * <p>
 * Only used from the render thread, except {@link #markDirty} and {@link #clear}.
 */
public final class RetainedBlockEntities {
    public static final RenderPipeline SOLID = retainedPipeline("entity_solid", false, false, true);
    public static final RenderPipeline CUTOUT_CULL = retainedPipeline("entity_cutout_cull", true, false, true);
    public static final RenderPipeline CUTOUT = retainedPipeline("entity_cutout", true, true, false);
    public static final RenderPipeline TEXT = retainedTextPipeline(RenderPipelines.TEXT_POLYGON_OFFSET, "text_polygon_offset");
    public static final RenderPipeline TEXT_GRAYSCALE = retainedTextPipeline(RenderPipelines.TEXT_GRAYSCALE_POLYGON_OFFSET,
            "text_grayscale_polygon_offset");
    public static final List<RenderPipeline> PIPELINES = List.of(SOLID, CUTOUT_CULL, CUTOUT, TEXT, TEXT_GRAYSCALE);

    /** Block entities rebuilt per frame at most; sections over budget take the vanilla path until they are built */
    private static final int REBUILD_BUDGET = 2048;
    private static final int EVICT_AFTER_FRAMES = 1200;
    private static final Vector4f WHITE = new Vector4f(1.0F, 1.0F, 1.0F, 1.0F);
    private static final Matrix4f IDENTITY = new Matrix4f();
    private static final Draw[] NO_DRAWS = new Draw[0];
    private static final int[] RUNS = new int[SectionFaceGroups.MAX_PLANE_RUNS * 2];

    private static final int NOT_CANDIDATE = 0;
    private static final int CANDIDATE = 1;
    private static final int MOVING = 2;

    private static final Long2ObjectOpenHashMap<Batch> BATCHES = new Long2ObjectOpenHashMap<>();
    /** Sections marked dirty, possibly from other threads; guarded by itself */
    private static final LongOpenHashSet PENDING_DIRTY = new LongOpenHashSet();
    private static volatile boolean clearPending;

    private static final List<Batch> VISIBLE = new ArrayList<>();
    private static final Map<RenderType, PreparedRenderType> PREPARED = new IdentityHashMap<>();
    private static final Map<RenderType, Matrix4f> MODEL_VIEWS = new IdentityHashMap<>();
    private static GpuBufferSlice[] transforms = new GpuBufferSlice[0];
    private static int drawCount;

    private static final ModelSubmitCapture CAPTURE = new ModelSubmitCapture();
    private static final PoseStack POSE_STACK = new PoseStack();
    private static final PoseStack WRITE_POSE = new PoseStack();
    /** Vertex builders of the batch being built, one per render type; the first {@link #partCount} are in use */
    private static final List<Part> PARTS = new ArrayList<>();
    private static int partCount;
    private static @Nullable Boolean pipelineAvailable;
    /** Glyphs of the block entity being built and the index of the text each belongs to */
    private static final List<TextRenderable> GLYPHS = new ArrayList<>();
    private static final IntArrayList GLYPH_TEXTS = new IntArrayList();
    private static int glyphText;
    private static final Font.GlyphVisitor GLYPH_COLLECTOR = new Font.GlyphVisitor() {
        @Override
        public void acceptRenderable(TextRenderable renderable) {
            GLYPHS.add(renderable);
            GLYPH_TEXTS.add(glyphText);
        }
    };
    private static boolean textFiltering;
    private static int frame;
    private static int rebuiltThisFrame;
    private static double camX, camY, camZ;

    private RetainedBlockEntities() {
    }

    private static RenderPipeline retainedPipeline(String name, boolean cutout, boolean perFaceLighting, boolean cull) {
        RenderPipeline.Builder builder = RenderPipeline.builder()
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
                .withLocation(Identifier.fromNamespaceAndPath("vulkanmod", "pipeline/retained_" + name));
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

    /** Vanilla's world text pipeline with only the vertex shader replaced */
    private static RenderPipeline retainedTextPipeline(RenderPipeline vanilla, String name) {
        List<ColorTargetState> colorTargets = vanilla.getColorTargetStates();
        List<VertexFormat> vertexFormats = vanilla.getVertexFormatBindings();
        RenderPipeline.Snippet snippet = new RenderPipeline.Snippet(
                vanilla.getShaders(),
                Optional.of(vanilla.getShaderDefines()),
                Optional.of(List.copyOf(vanilla.getBindGroupLayouts())),
                colorTargets.toArray(new ColorTargetState[0]),
                colorTargets.size(),
                Optional.ofNullable(vanilla.getDepthStencilState()),
                Optional.of(vanilla.getPolygonMode()),
                Optional.of(vanilla.isCull()),
                vertexFormats.toArray(new VertexFormat[0]),
                Optional.of(vanilla.getPrimitiveTopology()),
                vanilla.pushConstantSize());
        return RenderPipeline.builder(snippet)
                .withLocation(Identifier.fromNamespaceAndPath("vulkanmod", "pipeline/retained_" + name))
                .withVertexShader(Identifier.fromNamespaceAndPath("vulkanmod", "core/retained_text"))
                .build();
    }

    /**
     * The retained copy of the render type's pipeline, or null if it can't be retained. The z offset pipelines only
     * differ from the plain ones by name; their offset is the render type's layering transform.
     */
    private static @Nullable RenderPipeline retainedPipeline(RenderType renderType) {
        if (renderType.state.textureTransform != TextureTransform.DEFAULT_TEXTURING) {
            return null;
        }
        RenderPipeline vanilla = renderType.pipeline();
        if (vanilla == RenderPipelines.TEXT_POLYGON_OFFSET) {
            return TEXT;
        } else if (vanilla == RenderPipelines.TEXT_GRAYSCALE_POLYGON_OFFSET) {
            return TEXT_GRAYSCALE;
        } else if (vanilla == RenderPipelines.ENTITY_CUTOUT_CULL) {
            return CUTOUT_CULL;
        } else if (vanilla == RenderPipelines.ENTITY_CUTOUT || vanilla == RenderPipelines.ENTITY_CUTOUT_Z_OFFSET) {
            return CUTOUT;
        } else if (vanilla == RenderPipelines.ENTITY_SOLID || vanilla == RenderPipelines.ENTITY_SOLID_Z_OFFSET_FORWARD) {
            return SOLID;
        }
        return null;
    }

    /** @param faces the quads grouped by facing, for pipelines that cull back faces */
    private record Draw(RenderPipeline pipeline, RenderType renderType, GpuBuffer buffer, int indexCount,
                        @Nullable SectionFaceGroups faces) {
    }

    private static final class Part {
        final ByteBufferBuilder bytes = new ByteBufferBuilder(1 << 16);
        @Nullable RenderType renderType;
        @Nullable RenderPipeline pipeline;
        @Nullable BufferBuilder builder;
    }

    private static final class Batch {
        final int originX, originY, originZ;
        @Nullable SectionMesh mesh;
        boolean dirty;
        boolean[] members = new boolean[0];
        int memberCount;
        /** Block entities left out because they (or a chest's other half) move */
        List<BlockEntity> moving = List.of();
        Draw[] draws = NO_DRAWS;
        double viewDistance;
        int lastFrame;

        Batch(BlockPos origin) {
            this.originX = origin.getX();
            this.originY = origin.getY();
            this.originZ = origin.getZ();
        }

        void close() {
            for (Draw draw : this.draws) {
                draw.buffer.close();
            }
            this.draws = NO_DRAWS;
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

        // Sign lines are resolved with this setting when they are submitted
        boolean filtering = Minecraft.getInstance().isTextFilteringEnabled();
        if (filtering != textFiltering) {
            textFiltering = filtering;
            clearPending = true;
        }

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

        // BlockEntityRenderer.shouldRender: block centres within the view distance of the camera
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
        if (type == ChestBlockEntity.class || type == TrappedChestBlockEntity.class || type == EnderChestBlockEntity.class) {
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
        } else if (type == ShulkerBoxBlockEntity.class) {
            ShulkerBoxBlockEntity box = (ShulkerBoxBlockEntity) blockEntity;
            return blockEntity.isRemoved() || !box.isClosed() || box.getProgress(0.0F) != 0.0F || box.getProgress(1.0F) != 0.0F
                    ? MOVING : CANDIDATE;
        } else if (type == DecoratedPotBlockEntity.class) {
            // DecoratedPotRenderer wobbles while (game time - start + partial tick) / duration is within [0, 1]
            DecoratedPotBlockEntity pot = (DecoratedPotBlockEntity) blockEntity;
            DecoratedPotBlockEntity.WobbleStyle wobble = pot.lastWobbleStyle;
            return blockEntity.isRemoved() || wobble != null && level.getGameTime() - pot.wobbleStartedAtTick <= wobble.duration
                    ? MOVING : CANDIDATE;
        } else if (type == SkullBlockEntity.class) {
            SkullBlockEntity skull = (SkullBlockEntity) blockEntity;
            if (skull.getOwnerProfile() != null) {
                // Player skins are resolved asynchronously, which changes the render type
                return NOT_CANDIDATE;
            }
            if (blockEntity.isRemoved() || skull.getAnimation(0.0F) != skull.getAnimation(1.0F)) {
                return MOVING;
            }
            // Powered dragon and piglin heads start animating on their next tick, without an event
            BlockState state = blockEntity.getBlockState();
            if (state.getValueOrElse(AbstractSkullBlock.POWERED, false) && (state.is(Blocks.DRAGON_HEAD) || state.is(Blocks.DRAGON_WALL_HEAD)
                    || state.is(Blocks.PIGLIN_HEAD) || state.is(Blocks.PIGLIN_WALL_HEAD))) {
                return MOVING;
            }
            return CANDIDATE;
        } else if (type == SignBlockEntity.class || type == HangingSignBlockEntity.class) {
            // Glowing text draws an outline depending on the camera distance; obfuscated text changes every frame
            SignBlockEntity sign = (SignBlockEntity) blockEntity;
            SignText front = sign.getText(SignTextSlot.FRONT);
            SignText back = sign.getText(SignTextSlot.BACK);
            if (front.hasGlowingText() || back.hasGlowingText() || hasObfuscated(front) || hasObfuscated(back)) {
                return NOT_CANDIDATE;
            }
            return blockEntity.isRemoved() ? MOVING : CANDIDATE;
        }
        return NOT_CANDIDATE;
    }

    private static boolean hasObfuscated(SignText text) {
        return hasObfuscated(text.getMessages(false)) || hasObfuscated(text.getMessages(true));
    }

    private static boolean hasObfuscated(List<Component> messages) {
        for (Component message : messages) {
            if (message.visit((style, _) -> style.isObfuscated() ? FormattedText.STOP_ITERATION : Optional.empty(), Style.EMPTY)
                    .isPresent()) {
                return true;
            }
        }
        return false;
    }

    private static boolean isRetainedRenderer(BlockEntityRenderer<?, ?> renderer) {
        Class<?> type = renderer.getClass();
        return type == ChestRenderer.class || type == ShulkerBoxRenderer.class || type == DecoratedPotRenderer.class
                || type == SkullBlockRenderer.class || type == StandingSignRenderer.class || type == HangingSignRenderer.class;
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static void build(Batch batch, SectionMesh mesh, List<BlockEntity> blockEntities, ClientLevel level,
                              BlockEntityRenderDispatcher dispatcher, float partialTick, Vec3 cameraPos, CameraRenderState cameraState) {
        batch.close();
        batch.mesh = mesh;
        batch.dirty = false;
        batch.moving = List.of();

        int size = blockEntities.size();
        boolean[] members = batch.members.length == size ? batch.members : new boolean[size];
        Arrays.fill(members, false);
        batch.members = members;

        List<BlockEntity> moving = null;
        int count = 0;
        double viewDistance = -1.0;

        try {
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
                if (renderer == null || !isRetainedRenderer(renderer) || renderer.shouldRenderOffScreen()) {
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

                if (!CAPTURE.isModelsAndText() || !isRetainable(CAPTURE)) {
                    continue;
                }
                for (int m = 0; m < CAPTURE.modelCount; m++) {
                    ModelSubmitCapture.ModelRecord model = CAPTURE.model(m);
                    write(model, part(model.renderType).builder);
                }
                // TextFeatureRenderer's glyph renderer
                for (int g = 0; g < GLYPHS.size(); g++) {
                    TextRenderable glyph = GLYPHS.get(g);
                    ModelSubmitCapture.TextRecord text = CAPTURE.text(GLYPH_TEXTS.getInt(g));
                    glyph.render(text.pose, part(glyph.renderType(text.displayMode)).builder, text.lightCoords, false);
                }

                viewDistance = distance;
                members[i] = true;
                count++;
            }
        } finally {
            batch.draws = finishParts();
            GLYPHS.clear();
            GLYPH_TEXTS.clear();
        }

        rebuiltThisFrame += count;
        batch.moving = moving != null ? moving : List.of();
        batch.viewDistance = viewDistance;
        if (count > 0 && batch.draws.length == 0) {
            // Only empty models
            Arrays.fill(members, false);
            count = 0;
        }
        batch.memberCount = count;
    }

    /** Checks the captured submits and prepares the glyphs of the captured text into {@link #GLYPHS} */
    private static boolean isRetainable(ModelSubmitCapture capture) {
        for (int m = 0; m < capture.modelCount; m++) {
            ModelSubmitCapture.ModelRecord model = capture.model(m);
            if (model.outlineColor != 0 || model.renderType == null || retainedPipeline(model.renderType) == null) {
                return false;
            }
        }

        GLYPHS.clear();
        GLYPH_TEXTS.clear();
        Font font = Minecraft.getInstance().font;
        for (int t = 0; t < capture.textCount; t++) {
            ModelSubmitCapture.TextRecord text = capture.text(t);
            // SubmitNodeCollection.canRenderAsSolid, and no outline (TextFeatureRenderer would draw it separately)
            if (text.displayMode != Font.DisplayMode.POLYGON_OFFSET || ARGB.alpha(text.color) != 255
                    || ARGB.alpha(text.backgroundColor) != 0 || text.outlineColor != 0) {
                return false;
            }
            glyphText = t;
            SignTextCache.prepareText(font, text.string, text.x, text.y, text.color, text.dropShadow, false, text.backgroundColor)
                    .visit(GLYPH_COLLECTOR);
        }
        for (int g = 0; g < GLYPHS.size(); g++) {
            if (retainedPipeline(GLYPHS.get(g).renderType(Font.DisplayMode.POLYGON_OFFSET)) == null) {
                return false;
            }
        }
        return true;
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static void write(ModelSubmitCapture.ModelRecord record, VertexConsumer buffer) {
        Model model = record.model;
        // All faces: the pipeline culls
        if (ModelGeometryCache.isCacheable(record.state) && ModelGeometryCache.write(model, record.state, record.pose, buffer,
                record.lightCoords, record.overlayCoords, record.tintedColor, record.uvMapping, false)) {
            return;
        }

        // ModelFeatureRenderer's vanilla path
        WRITE_POSE.last().set(record.pose);
        model.setupAnim(record.state);
        model.renderToBuffer(WRITE_POSE, record.uvMapping != null ? record.uvMapping.wrap(buffer) : buffer, record.lightCoords,
                record.overlayCoords, record.tintedColor);
    }

    private static Part part(RenderType renderType) {
        for (int i = 0; i < partCount; i++) {
            Part part = PARTS.get(i);
            if (part.renderType == renderType) {
                return part;
            }
        }
        if (partCount == PARTS.size()) {
            PARTS.add(new Part());
        }
        Part part = PARTS.get(partCount++);
        part.renderType = renderType;
        part.pipeline = retainedPipeline(renderType);
        part.builder = new BufferBuilder(part.bytes, PrimitiveTopology.QUADS, renderType.format());
        return part;
    }

    /** Uploads the parts' vertices, one draw per render type, and releases the builders */
    private static Draw[] finishParts() {
        Draw[] draws = NO_DRAWS;
        int drawn = 0;
        for (int i = 0; i < partCount; i++) {
            Part part = PARTS.get(i);
            try (MeshData data = part.builder.build()) {
                if (data != null) {
                    if (draws.length == 0) {
                        draws = new Draw[partCount];
                    }
                    // Reorders the vertices before the upload
                    SectionFaceGroups faces = part.pipeline.isCull() ? SectionFaceGroups.sortByPlanes(data) : null;
                    GpuBuffer buffer = RenderSystem.getDevice().createBuffer(() -> "VulkanMod retained block entities",
                            GpuBuffer.USAGE_VERTEX, data.vertexBuffer());
                    draws[drawn++] = new Draw(part.pipeline, part.renderType, buffer, data.drawState().vertexCount() / 4 * 6, faces);
                }
            }
            part.builder = null;
            part.renderType = null;
            part.pipeline = null;
        }
        partCount = 0;
        return drawn == draws.length ? draws : Arrays.copyOf(draws, drawn);
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

    /** Writes the draws' transforms while the frame is prepared (after the features, before the passes) */
    public static void prepareDraws() {
        drawCount = 0;
        PREPARED.clear();
        MODEL_VIEWS.clear();
        int count = 0;
        for (Batch batch : VISIBLE) {
            count += batch.draws.length;
        }
        if (count == 0) {
            return;
        }

        Matrix4f modelView = RenderSystem.getModelViewMatrixCopy();
        DynamicGpuData.Transform[] data = new DynamicGpuData.Transform[count];
        int maxIndexCount = 0;
        int i = 0;
        for (Batch batch : VISIBLE) {
            for (Draw draw : batch.draws) {
                // Fresh vectors: the uniform storage compares against the last record it wrote
                Vector3f offset = new Vector3f((float) (batch.originX - camX), (float) (batch.originY - camY), (float) (batch.originZ - camZ));
                data[i++] = new DynamicGpuData.Transform(modelView(draw.renderType, modelView), WHITE, offset, IDENTITY);
                maxIndexCount = Math.max(maxIndexCount, draw.indexCount);
                PREPARED.computeIfAbsent(draw.renderType, RenderType::prepare);
            }
        }
        transforms = RenderSystem.getDynamicUniforms().writeTransforms(data);
        RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS).getBuffer(maxIndexCount);
        drawCount = count;
    }

    /** The model view matrix with the render type's layering transform, as {@link RenderType#prepare} writes it */
    private static Matrix4f modelView(RenderType renderType, Matrix4f modelView) {
        Consumer<Matrix4f> modifier = renderType.state.layeringTransform.getModifier();
        if (modifier == null) {
            return modelView;
        }
        return MODEL_VIEWS.computeIfAbsent(renderType, _ -> {
            Matrix4f layered = new Matrix4f(modelView);
            modifier.accept(layered);
            return layered;
        });
    }

    /** Draws the batches into the world's solid pass */
    public static void draw(RenderPass renderPass) {
        if (drawCount == 0) {
            return;
        }

        renderPass.pushDebugGroup(() -> "VulkanMod retained block entities");
        RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS);
        RenderPipeline boundPipeline = null;
        RenderType boundType = null;
        boolean faceCulling = Initializer.CONFIG.modelBackFaceCulling;
        int i = 0;
        for (Batch batch : VISIBLE) {
            float eyeX = (float) (camX - batch.originX), eyeY = (float) (camY - batch.originY), eyeZ = (float) (camZ - batch.originZ);
            for (Draw draw : batch.draws) {
                GpuBufferSlice transform = transforms[i++];
                if (draw.pipeline != boundPipeline) {
                    CompiledRenderPipeline pipeline = RenderSystem.getCompiledPipelineNullable(draw.pipeline);
                    if (pipeline == null) {
                        continue;
                    }
                    boundPipeline = draw.pipeline;
                    boundType = null;
                    renderPass.setPipeline(pipeline);
                    RenderSystem.bindDefaultUniforms(renderPass);
                    renderPass.setIndexBuffer(indices.getBuffer(), indices.type());
                }
                if (draw.renderType != boundType) {
                    boundType = draw.renderType;
                    for (PreparedRenderType.Texture texture : PREPARED.get(boundType).textures()) {
                        renderPass.setUniform(texture.name(), texture.textureView(), texture.sampler());
                    }
                }
                renderPass.setUniform("DynamicTransforms", transform);
                renderPass.setVertexBuffer(0, draw.buffer.slice());
                if (faceCulling && draw.faces != null) {
                    // Quads that face away from the eye would be culled by the pipeline
                    int runs = draw.faces.visiblePlaneRuns(eyeX, eyeY, eyeZ, RUNS);
                    for (int r = 0; r < runs; r++) {
                        renderPass.drawIndexed(RUNS[r * 2 + 1] * 6, 1, 0, RUNS[r * 2] * 4, 0);
                    }
                } else {
                    renderPass.drawIndexed(draw.indexCount, 1, 0, 0, 0);
                }
            }
        }
        renderPass.popDebugGroup();
    }
}
