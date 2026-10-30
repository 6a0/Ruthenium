package net.ruthenium.render.blockentity;

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
import net.minecraft.client.renderer.blockentity.BellRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.ChestRenderer;
import net.minecraft.client.renderer.blockentity.CopperGolemStatueBlockRenderer;
import net.minecraft.client.renderer.blockentity.DecoratedPotRenderer;
import net.minecraft.client.renderer.blockentity.HangingSignRenderer;
import net.minecraft.client.renderer.blockentity.LecternRenderer;
import net.minecraft.client.renderer.blockentity.ShulkerBoxRenderer;
import net.minecraft.client.renderer.blockentity.SkullBlockRenderer;
import net.minecraft.client.renderer.blockentity.StandingSignRenderer;
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.blockentity.state.SignRenderState;
import net.minecraft.client.renderer.chunk.SectionMesh;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.util.LightCoordsUtil;
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
import net.minecraft.world.level.block.entity.BellBlockEntity;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.level.block.entity.CopperGolemStatueBlockEntity;
import net.minecraft.world.level.block.entity.DecoratedPotBlockEntity;
import net.minecraft.world.level.block.entity.EnderChestBlockEntity;
import net.minecraft.world.level.block.entity.HangingSignBlockEntity;
import net.minecraft.world.level.block.entity.LecternBlockEntity;
import net.minecraft.world.level.block.entity.ShulkerBoxBlockEntity;
import net.minecraft.world.level.block.entity.SignBlockEntity;
import net.minecraft.world.level.block.entity.SignText;
import net.minecraft.world.level.block.entity.SignTextSlot;
import net.minecraft.world.level.block.entity.SkullBlockEntity;
import net.minecraft.world.level.block.entity.TrappedChestBlockEntity;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.ChestType;
import net.minecraft.world.phys.Vec3;
import net.ruthenium.Initializer;
import net.ruthenium.mixin.render.blockentity.FrustumAccessor;
import net.ruthenium.render.chunk.SectionFaceGroups;
import net.ruthenium.interfaces.IdleTicker;
import net.ruthenium.mixin.render.text.AbstractSignRendererAccessor;
import net.ruthenium.mixin.render.vertex.BufferBuilderAccessor;
import net.ruthenium.render.entity.ModelGeometryCache;
import net.ruthenium.render.text.SignTextCache;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.joml.Vector4f;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
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
 * decorated pots (not wobbling), skulls (not animating, no player skin), signs (no obfuscated text), lecterns, bells
 * (not swinging) and copper golem statues.
 * Per render section, their vertices are built once, relative to the section origin, by running the real renderer
 * into a {@link ModelSubmitCapture}: every submitted model (pots submit their parts as models) is written with all
 * faces, through {@link ModelGeometryCache} for rest pose models and with vanilla's model rendering otherwise, and
 * every submitted text the way {@code TextFeatureRenderer} writes it, into one vertex buffer per render type. A block
 * entity is only kept if all its submits are models in a cutout or solid entity pipeline with the default texture
 * transform and no outline, or opaque polygon offset text without background or outline: text vanilla itself draws
 * with the solid features, where the order doesn't matter. Glowing sign text is full bright and drawn with an outline
 * (8 copies without polygon offset, behind the text by depth) if it is black, or while the camera entity is within 16
 * blocks of the sign or scoping ({@code AbstractSignRenderer.isOutlineVisible}). Signs whose outline depends on the
 * camera are built without it into separate draws ({@link Batch#signs}), and each frame those that vanilla would
 * outline take the vanilla path while the draws leave out their quads. The extraction then skips those block entities, and the
 * batches are drawn right after the world's solid features with copies of vanilla's entity and text pipelines
 * ({@link #PIPELINES}) whose vertex shader adds {@code ModelOffset} (section origin minus camera) to the position;
 * the render type's textures and its layering transform (the z offset of skulls) are applied as vanilla does.
 * Buffers of pipelines that cull back faces are sorted by facing and plane ({@link SectionFaceGroups#sortByPlanes}),
 * and a frame draws only the quads whose plane the eye can be in front of (switch {@code modelBackFaceCulling}); the
 * GPU would cull the others after running their vertex shader, which is most of the batches' GPU cost.
 * <p>
 * A batch is rebuilt when the section's compiled mesh changes (block entity list), when the section or a neighbour
 * is marked dirty (block and light changes, which is where the light comes from), on block events of chests,
 * shulker boxes, pots and bells (lids, wobbles, swings), on block entity data updates and sign text changes, when breaking progress
 * changes, and when a left out block entity comes to rest. Everything is dropped when fonts reload or the text
 * filtering setting changes. Batches of sections only partly within the view distance aren't drawn; their
 * block entities take the vanilla path that frame.
 * <p>
 * With {@code retainedCellCulling} the draws of the opaque entity pipelines are split by the 8x8x8 block cell of their
 * block entity, each with the bounds of its vertices, and a cell whose bounds (plus half a block) lie outside the frame's
 * cull frustum isn't drawn: a section is drawn whenever any of it is in view, and its block entities outside the view
 * only cost vertex work. The cells keep the render type order; only the order across block entities of one render type
 * changes, which shows only where two of them draw the same depth. A part's cells are drawn facing group by facing group,
 * as one section-wide buffer sorted by planes draws them, and within a group nearest cell first: drawn cell after cell in
 * index order, occluded fragments of dense sections were shaded (the block entity scene's retained draws took 448 instead
 * of 287 us of GPU per frame).
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
    /** Text without polygon offset: the outline of glowing sign text */
    public static final RenderPipeline TEXT_NORMAL = retainedTextPipeline(RenderPipelines.TEXT, "text");
    public static final RenderPipeline TEXT_GRAYSCALE_NORMAL = retainedTextPipeline(RenderPipelines.TEXT_GRAYSCALE, "text_grayscale");
    public static final List<RenderPipeline> PIPELINES = List.of(SOLID, CUTOUT_CULL, CUTOUT, TEXT, TEXT_GRAYSCALE, TEXT_NORMAL,
            TEXT_GRAYSCALE_NORMAL);

    /** Block entities rebuilt per frame at most; sections over budget take the vanilla path until they are built */
    private static final int REBUILD_BUDGET = 2048;
    /**
     * Batches unused for this many frames and seconds are freed, or for this many frames alone when their section is
     * beyond {@link #EVICT_DISTANCE} blocks past their view distance. A frame count alone freed the batches behind the
     * camera after ~1 s at high frame rates, so turning around rebuilt them (a section of ~2k chests takes ~7-10 ms).
     */
    private static final int EVICT_AFTER_FRAMES = 1200;
    private static final long EVICT_AFTER_NANOS = 30_000_000_000L;
    private static final double EVICT_DISTANCE = 64.0;
    private static long frameNanos;
    private static final Vector4f WHITE = new Vector4f(1.0F, 1.0F, 1.0F, 1.0F);
    private static final Matrix4f IDENTITY = new Matrix4f();
    private static final Draw[] NO_DRAWS = new Draw[0];
    private static final BlockEntity[] NO_BLOCK_ENTITIES = new BlockEntity[0];
    private static final BlockState[] NO_STATES = new BlockState[0];
    private static final int[] NO_INTS = new int[0];
    private static final boolean[] NO_BOOLEANS = new boolean[0];
    private static final int NO_LIGHT = Integer.MIN_VALUE;
    /** AbstractSignRenderer.OUTLINE_RENDER_DISTANCE plus a block of margin, for skipping batches without a sign in reach */
    private static final double OUTLINE_REACH_SQ = 17.0 * 17.0;
    private static final int[] RUNS = new int[SectionFaceGroups.MAX_PLANE_RUNS * 2];

    private static final int NOT_CANDIDATE = 0;
    private static final int CANDIDATE = 1;
    private static final int MOVING = 2;

    private static final Long2ObjectOpenHashMap<Batch> BATCHES = new Long2ObjectOpenHashMap<>();
    /** Sections marked dirty, possibly from other threads; guarded by itself, like the next one */
    private static final LongOpenHashSet PENDING_DIRTY = new LongOpenHashSet();
    /** Sections whose blocks or light changed */
    private static final LongOpenHashSet PENDING_CHANGED = new LongOpenHashSet();
    private static volatile boolean clearPending;

    private static final List<Batch> VISIBLE = new ArrayList<>();
    /** This frame's cull frustum, for {@link #isCulled} */
    private static @Nullable Frustum cullFrustum;
    /** {@code retainedCellCulling} when the batches were built */
    private static boolean builtWithCells = true;
    /** Per draw of {@link #prepareDraws}, in its order: culled this frame */
    private static boolean[] culled = new boolean[0];
    /** Per draw of {@link #prepareDraws}: the index of its transform in {@link #transforms} */
    private static int[] transformOf = new int[0];
    /** {@link #draw}'s bindings, and its order of a part's cells */
    private static @Nullable RenderPipeline boundPipeline;
    private static @Nullable RenderType boundType;
    private static @Nullable GpuBufferSlice boundTransform;
    private static @Nullable GpuBuffer boundBuffer;
    private static final int[] FIRST_QUAD = new int[1];
    private static final int[] CELL_ORDER = new int[8];
    private static final float[] CELL_DISTANCES = new float[8];
    private static final Map<RenderType, PreparedRenderType> PREPARED = new IdentityHashMap<>();
    private static final Map<RenderType, Matrix4f> MODEL_VIEWS = new IdentityHashMap<>();
    private static GpuBufferSlice[] transforms = new GpuBufferSlice[0];
    private static int drawCount;

    private static final ModelSubmitCapture CAPTURE = new ModelSubmitCapture();
    private static final PoseStack POSE_STACK = new PoseStack();
    private static final PoseStack WRITE_POSE = new PoseStack();
    /** Vertex builders of the batch being built, one per render type */
    private static final PartSet PARTS = new PartSet();
    /** Vertex builders of the batch's signs whose outline depends on the camera, in build order */
    private static final PartSet SIGN_PARTS = new PartSet();
    /** List indices of the signs written into {@link #SIGN_PARTS} */
    private static final IntArrayList SIGN_INDICES = new IntArrayList();
    private static @Nullable Boolean pipelineAvailable;
    /**
     * Glyphs of the block entity being built and the index of the text each belongs to, shifted left by one, with the
     * low bit set for outline glyphs (drawn without polygon offset)
     */
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
                .withVertexShader(Identifier.fromNamespaceAndPath("ruthenium", "core/retained_entity"))
                .withFragmentShader(Identifier.withDefaultNamespace("core/entity"))
                .withBindGroupLayout(BindGroupLayouts.SAMPLER0)
                .withVertexBinding(0, DefaultVertexFormat.ENTITY)
                .withPrimitiveTopology(PrimitiveTopology.QUADS)
                .withDepthStencilState(DepthStencilState.DEFAULT)
                .withBindGroupLayout(BindGroupLayouts.LIGHTING)
                .withBindGroupLayout(BindGroupLayouts.SAMPLER2)
                .withLocation(Identifier.fromNamespaceAndPath("ruthenium", "pipeline/retained_" + name));
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
                .withLocation(Identifier.fromNamespaceAndPath("ruthenium", "pipeline/retained_" + name))
                .withVertexShader(Identifier.fromNamespaceAndPath("ruthenium", "core/retained_text"))
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
        } else if (vanilla == RenderPipelines.TEXT) {
            return TEXT_NORMAL;
        } else if (vanilla == RenderPipelines.TEXT_GRAYSCALE) {
            return TEXT_GRAYSCALE_NORMAL;
        } else if (vanilla == RenderPipelines.ENTITY_CUTOUT_CULL) {
            return CUTOUT_CULL;
        } else if (vanilla == RenderPipelines.ENTITY_CUTOUT || vanilla == RenderPipelines.ENTITY_CUTOUT_Z_OFFSET) {
            return CUTOUT;
        } else if (vanilla == RenderPipelines.ENTITY_SOLID || vanilla == RenderPipelines.ENTITY_SOLID_Z_OFFSET_FORWARD) {
            return SOLID;
        }
        return null;
    }

    /**
     * @param faces    the quads grouped by facing, for pipelines that cull back faces
     * @param signEnds for draws of {@link Batch#signDraws}: the quad count after each of {@link Batch#signs}
     */
    private record Draw(RenderPipeline pipeline, RenderType renderType, GpuBuffer buffer, int indexCount,
                        @Nullable SectionFaceGroups faces, int @Nullable [] signEnds, float[] bounds) {
    }

    /** {@code retainedCellCulling}: entity model geometry is split into the 8 cells of 8x8x8 blocks of its section */
    private static final int CELLS = 8;
    /** Bounds that are never culled (geometry whose format doesn't start with the position) */
    private static final float[] NO_BOUNDS = new float[0];
    /** Added on every side of a draw's vertex bounds before the frustum test */
    private static final float CELL_CULL_MARGIN = 0.5f;

    private static final class Part {
        final ByteBufferBuilder bytes = new ByteBufferBuilder(1 << 16);
        @Nullable RenderType renderType;
        @Nullable RenderPipeline pipeline;
        @Nullable BufferBuilder builder;
        /** In {@link #SIGN_PARTS}: the quad count after each sign written so far */
        final IntArrayList signEnds = new IntArrayList();
        /** With {@code retainedCellCulling}, an entity pipeline's geometry by cell of its block entity ({@link #cellBuilder}) */
        boolean split;
        final ByteBufferBuilder[] cellBytes = new ByteBufferBuilder[CELLS];
        final @Nullable BufferBuilder[] cellBuilders = new BufferBuilder[CELLS];
    }

    /** The builder of a part for a block entity in the given cell of the section */
    private static BufferBuilder cellBuilder(Part part, int cell) {
        if (!part.split) {
            return part.builder;
        }
        BufferBuilder builder = part.cellBuilders[cell];
        if (builder == null) {
            if (part.cellBytes[cell] == null) {
                part.cellBytes[cell] = new ByteBufferBuilder(1 << 14);
            }
            builder = part.cellBuilders[cell] = new BufferBuilder(part.cellBytes[cell], PrimitiveTopology.QUADS, part.renderType.format());
        }
        return builder;
    }

    /** One vertex builder per render type; the first {@link #count} are in use */
    private static final class PartSet {
        final List<Part> parts = new ArrayList<>();
        int count;
    }

    private static final class Batch {
        final int originX, originY, originZ;
        @Nullable SectionMesh mesh;
        /** Must be rebuilt (a block entity's content changed) */
        boolean dirty;
        /** Blocks or light of the section changed; rebuilt only if {@link #isUnchanged} finds a difference */
        boolean blocksChanged;
        /** What the batch was built from: the block entity list, their block states and the light members were lit with */
        BlockEntity[] built = NO_BLOCK_ENTITIES;
        BlockState[] states = NO_STATES;
        int[] lights = NO_INTS;
        /** The light at the other half of a member double chest, or {@link #NO_LIGHT} */
        int[] otherLights = NO_INTS;
        /** Whether the other half of a member double chest is in another section, whose changes concern this batch too */
        boolean linked;
        boolean[] members = new boolean[0];
        int memberCount;
        /** Block entities left out because they (or a chest's other half) move */
        List<BlockEntity> moving = List.of();
        Draw[] draws = NO_DRAWS;
        /** Members whose glowing text vanilla outlines near the camera (list indices), built without the outline */
        int[] signs = NO_INTS;
        /** The draws of {@link #signs}, quads in sign order */
        Draw[] signDraws = NO_DRAWS;
        /** Which of {@link #signs} take the vanilla path this frame, if {@link #anyNear} */
        boolean[] signsNear = NO_BOOLEANS;
        boolean anyNear;
        /** {@link #members} without the signs near the camera */
        boolean[] frameMembers = NO_BOOLEANS;
        double viewDistance;
        int lastFrame;
        long lastNanos;

        Batch(BlockPos origin) {
            this.originX = origin.getX();
            this.originY = origin.getY();
            this.originZ = origin.getZ();
        }

        void close() {
            for (Draw draw : this.draws) {
                draw.buffer.close();
            }
            for (Draw draw : this.signDraws) {
                draw.buffer.close();
            }
            this.draws = NO_DRAWS;
            this.signDraws = NO_DRAWS;
            this.signs = NO_INTS;
            this.anyNear = false;
            this.memberCount = 0;
        }
    }

    public static void markDirty(int sectionX, int sectionY, int sectionZ) {
        synchronized (PENDING_DIRTY) {
            PENDING_DIRTY.add(SectionPos.asLong(sectionX, sectionY, sectionZ));
        }
    }

    /**
     * Blocks or light of the section changed ({@code LevelExtractor.setSectionDirty}): its batch, and the batches of
     * neighbours holding a double chest whose other half may be in it, are checked against what they were built from
     * and rebuilt if anything differs
     */
    public static void markBlocksChanged(int sectionX, int sectionY, int sectionZ) {
        synchronized (PENDING_DIRTY) {
            PENDING_CHANGED.add(SectionPos.asLong(sectionX, sectionY, sectionZ));
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
    public static void beginFrame(Vec3 cameraPos, @Nullable Frustum frustum) {
        cullFrustum = frustum;
        frame++;
        frameNanos = System.nanoTime();
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

        // Batches are built split by cell or not (retainedCellCulling): rebuilt when it changes, also by reflection
        boolean cells = Initializer.CONFIG.retainedCellCulling;
        if (cells != builtWithCells) {
            builtWithCells = cells;
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
            if (!PENDING_CHANGED.isEmpty()) {
                PENDING_CHANGED.forEach((long key) -> {
                    int x = SectionPos.x(key), y = SectionPos.y(key), z = SectionPos.z(key);
                    Batch batch = BATCHES.get(key);
                    if (batch != null) {
                        batch.blocksChanged = true;
                    }
                    markLinkedChanged(x - 1, y, z);
                    markLinkedChanged(x + 1, y, z);
                    markLinkedChanged(x, y - 1, z);
                    markLinkedChanged(x, y + 1, z);
                    markLinkedChanged(x, y, z - 1);
                    markLinkedChanged(x, y, z + 1);
                });
                PENDING_CHANGED.clear();
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

    private static void markLinkedChanged(int x, int y, int z) {
        Batch batch = BATCHES.get(SectionPos.asLong(x, y, z));
        if (batch != null && batch.linked) {
            batch.blocksChanged = true;
        }
    }

    /**
     * Whether a batch whose section's blocks or light changed (or whose section was compiled again) would be built
     * the same: the same block entity objects in the same order, with the same block states, and its members lit the
     * same (vanilla's light at the block entity, and for double chests also at the other half, which
     * {@code BrightnessCombiner} combines with it). Everything else a build reads is block entity data, whose changes
     * mark the batch {@link Batch#dirty}.
     */
    private static boolean isUnchanged(Batch batch, List<BlockEntity> blockEntities, ClientLevel level) {
        BlockEntity[] built = batch.built;
        int size = blockEntities.size();
        if (size != built.length || batch.members.length != size) {
            return false;
        }
        BlockState[] states = batch.states;
        int[] lights = batch.lights;
        int[] otherLights = batch.otherLights;
        boolean[] members = batch.members;
        for (int i = 0; i < size; i++) {
            BlockEntity blockEntity = blockEntities.get(i);
            if (blockEntity != built[i] || blockEntity.getBlockState() != states[i]) {
                return false;
            }
            if (members[i]) {
                BlockPos pos = blockEntity.getBlockPos();
                if (LightCoordsUtil.getLightCoords(level, pos) != lights[i]) {
                    return false;
                }
                if (otherLights[i] != NO_LIGHT && LightCoordsUtil.getLightCoords(level, otherHalf(pos, states[i])) != otherLights[i]) {
                    return false;
                }
            }
        }
        return true;
    }

    /** The other half of a double chest, where {@code ChestBlock.combine} looks for it */
    private static BlockPos otherHalf(BlockPos pos, BlockState state) {
        return pos.relative(ChestBlock.getConnectedDirection(state));
    }

    private static boolean isFar(Batch batch) {
        double dx = batch.originX + 8.0 - camX, dy = batch.originY + 8.0 - camY, dz = batch.originZ + 8.0 - camZ;
        double limit = Math.max(batch.viewDistance, 0.0) + EVICT_DISTANCE;
        return dx * dx + dy * dy + dz * dz > limit * limit;
    }

    private static void evict() {
        ObjectIterator<Long2ObjectMap.Entry<Batch>> iterator = BATCHES.long2ObjectEntrySet().fastIterator();
        while (iterator.hasNext()) {
            Batch batch = iterator.next().getValue();
            if (frame - batch.lastFrame > EVICT_AFTER_FRAMES
                    && (frameNanos - batch.lastNanos > EVICT_AFTER_NANOS || isFar(batch))) {
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
        batch.lastNanos = frameNanos;

        if (!batch.dirty && batch.mesh == mesh) {
            for (BlockEntity moving : batch.moving) {
                if (candidate(moving, level) == CANDIDATE) {
                    batch.dirty = true;
                    break;
                }
            }
        }

        if (!batch.dirty && (batch.blocksChanged || batch.mesh != mesh) && batch.mesh != null
                && Initializer.CONFIG.retainedChangeChecks && isUnchanged(batch, blockEntities, level)) {
            // A block or light change that doesn't concern the batch, or the section compiled again with the same block entities
            batch.mesh = mesh;
            batch.blocksChanged = false;
        }
        if (batch.dirty || batch.blocksChanged || batch.mesh != mesh) {
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
            return batch.signs.length == 0 ? batch.members : frameMembers(batch);
        } else if (near >= limit) {
            // None would be rendered
            return batch.members;
        }
        return null;
    }

    private static double sq(double v) {
        return v * v;
    }

    /**
     * The members the extraction skips this frame: all but the {@link Batch#signs} whose outline vanilla draws this
     * frame ({@code AbstractSignRenderer.isOutlineVisible}, which the extraction would call), which take the vanilla
     * path while {@link #draw} leaves out their quads
     */
    private static boolean[] frameMembers(Batch batch) {
        int[] signs = batch.signs;
        boolean[] near = batch.signsNear;
        boolean anyNear = false;
        if (mayShowOutlines(batch)) {
            BlockEntity[] built = batch.built;
            for (int k = 0; k < signs.length; k++) {
                near[k] = AbstractSignRendererAccessor.invokeIsOutlineVisible(built[signs[k]].getBlockPos());
                anyNear |= near[k];
            }
        }
        batch.anyNear = anyNear;
        if (!anyNear) {
            return batch.members;
        }
        boolean[] members = batch.frameMembers;
        if (members.length != batch.members.length) {
            members = batch.frameMembers = new boolean[batch.members.length];
        }
        System.arraycopy(batch.members, 0, members, 0, members.length);
        for (int k = 0; k < signs.length; k++) {
            if (near[k]) {
                members[signs[k]] = false;
            }
        }
        return members;
    }

    /** False if no sign of the batch can be outlined: no scoping and every block centre beyond reach of the camera entity */
    private static boolean mayShowOutlines(Batch batch) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.player != null && minecraft.options.getCameraType().isFirstPerson() && minecraft.player.isScoping()) {
            return true;
        }
        Entity camera = minecraft.getCameraEntity();
        if (camera == null) {
            return false;
        }
        double x = camera.getX(), y = camera.getY(), z = camera.getZ();
        double minX = batch.originX + 0.5, minY = batch.originY + 0.5, minZ = batch.originZ + 0.5;
        double dx = Math.max(0.0, Math.max(minX - x, x - (minX + 15.0)));
        double dy = Math.max(0.0, Math.max(minY - y, y - (minY + 15.0)));
        double dz = Math.max(0.0, Math.max(minZ - z, z - (minZ + 15.0)));
        return dx * dx + dy * dy + dz * dz < OUTLINE_REACH_SQ;
    }

    /** Whether vanilla draws an outline around a side of the sign only near the camera: glowing text that isn't black */
    private static boolean isOutlineDependent(SignRenderState state) {
        return isOutlineDependent(state.frontText) || isOutlineDependent(state.backText);
    }

    private static boolean isOutlineDependent(@Nullable SignText text) {
        return text != null && text.hasGlowingText() && text.getColor().getTextColor() != DyeColor.BLACK.getTextColor();
    }

    private static int candidate(BlockEntity blockEntity, ClientLevel level) {
        Class<?> type = blockEntity.getClass();
        if (type == ChestBlockEntity.class || type == TrappedChestBlockEntity.class || type == EnderChestBlockEntity.class) {
            if (blockEntity.isRemoved() || !((IdleTicker) blockEntity).ruthenium$isTickIdle()) {
                return MOVING;
            }

            BlockState state = blockEntity.getBlockState();
            if (state.hasProperty(ChestBlock.TYPE) && state.getValue(ChestBlock.TYPE) != ChestType.SINGLE) {
                BlockEntity other = level.getBlockEntity(blockEntity.getBlockPos().relative(ChestBlock.getConnectedDirection(state)));
                if (other != null && !(other instanceof IdleTicker idle && idle.ruthenium$isTickIdle())) {
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
        } else if (type == LecternBlockEntity.class || type == CopperGolemStatueBlockEntity.class) {
            // Posed by their block state alone (book on the lectern, statue pose, facing, oxidation)
            return blockEntity.isRemoved() ? MOVING : CANDIDATE;
        } else if (type == BellBlockEntity.class) {
            // A rung bell swings for 50 ticks
            return blockEntity.isRemoved() || ((BellBlockEntity) blockEntity).shaking ? MOVING : CANDIDATE;
        } else if (type == SignBlockEntity.class || type == HangingSignBlockEntity.class) {
            // Obfuscated text changes every frame
            SignBlockEntity sign = (SignBlockEntity) blockEntity;
            SignText front = sign.getText(SignTextSlot.FRONT);
            SignText back = sign.getText(SignTextSlot.BACK);
            if (!Initializer.CONFIG.retainedGlowingText && (front.hasGlowingText() || back.hasGlowingText())
                    || hasObfuscated(front) || hasObfuscated(back)) {
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
                || type == SkullBlockRenderer.class || type == StandingSignRenderer.class || type == HangingSignRenderer.class
                || type == LecternRenderer.class || type == BellRenderer.class || type == CopperGolemStatueBlockRenderer.class;
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static void build(Batch batch, SectionMesh mesh, List<BlockEntity> blockEntities, ClientLevel level,
                              BlockEntityRenderDispatcher dispatcher, float partialTick, Vec3 cameraPos, CameraRenderState cameraState) {
        batch.close();
        batch.mesh = mesh;
        batch.dirty = false;
        batch.blocksChanged = false;
        batch.moving = List.of();

        int size = blockEntities.size();
        boolean[] members = batch.members.length == size ? batch.members : new boolean[size];
        Arrays.fill(members, false);
        batch.members = members;
        BlockEntity[] built = batch.built.length == size ? batch.built : new BlockEntity[size];
        BlockState[] states = batch.states.length == size ? batch.states : new BlockState[size];
        int[] lights = batch.lights.length == size ? batch.lights : new int[size];
        int[] otherLights = batch.otherLights.length == size ? batch.otherLights : new int[size];
        batch.built = built;
        batch.states = states;
        batch.lights = lights;
        batch.otherLights = otherLights;
        boolean linked = false;

        List<BlockEntity> moving = null;
        int count = 0;
        double viewDistance = -1.0;

        try {
            for (int i = 0; i < size; i++) {
                BlockEntity blockEntity = blockEntities.get(i);
                built[i] = blockEntity;
                states[i] = blockEntity.getBlockState();
                otherLights[i] = NO_LIGHT;
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
                boolean outlineDependent = false;
                if (state instanceof SignRenderState sign) {
                    // Built without the outline of the camera's reach; frameMembers hands such signs to vanilla while it shows
                    outlineDependent = isOutlineDependent(sign);
                    sign.drawOutline = false;
                }

                POSE_STACK.pushPose();
                POSE_STACK.translate((float) (pos.getX() - batch.originX), (float) (pos.getY() - batch.originY), (float) (pos.getZ() - batch.originZ));
                CAPTURE.reset();
                renderer.submit(state, POSE_STACK, CAPTURE, cameraState);
                POSE_STACK.popPose();

                if (!CAPTURE.isModelsAndText() || !isRetainable(CAPTURE)) {
                    continue;
                }
                PartSet parts = outlineDependent ? SIGN_PARTS : PARTS;
                int cell = (pos.getX() - batch.originX) >> 3 & 1 | ((pos.getY() - batch.originY) >> 3 & 1) << 1
                        | ((pos.getZ() - batch.originZ) >> 3 & 1) << 2;
                for (int m = 0; m < CAPTURE.modelCount; m++) {
                    ModelSubmitCapture.ModelRecord model = CAPTURE.model(m);
                    write(model, cellBuilder(part(parts, model.renderType), cell));
                }
                // TextFeatureRenderer's glyph renderer
                for (int g = 0; g < GLYPHS.size(); g++) {
                    TextRenderable glyph = GLYPHS.get(g);
                    int entry = GLYPH_TEXTS.getInt(g);
                    ModelSubmitCapture.TextRecord text = CAPTURE.text(entry >> 1);
                    glyph.render(text.pose, part(parts, glyph.renderType(glyphMode(entry))).builder, text.lightCoords, false);
                }
                if (outlineDependent) {
                    SIGN_INDICES.add(i);
                    for (int p = 0; p < SIGN_PARTS.count; p++) {
                        Part part = SIGN_PARTS.parts.get(p);
                        part.signEnds.add(((BufferBuilderAccessor) part.builder).ruthenium$getVertices() / 4);
                    }
                }

                viewDistance = distance;
                members[i] = true;
                count++;
                lights[i] = LightCoordsUtil.getLightCoords(level, pos);
                BlockState blockState = states[i];
                if (blockEntity instanceof ChestBlockEntity && blockState.hasProperty(ChestBlock.TYPE)
                        && blockState.getValue(ChestBlock.TYPE) != ChestType.SINGLE) {
                    BlockPos other = otherHalf(pos, blockState);
                    otherLights[i] = LightCoordsUtil.getLightCoords(level, other);
                    linked |= SectionPos.blockToSectionCoord(other.getX()) != SectionPos.blockToSectionCoord(pos.getX())
                            || SectionPos.blockToSectionCoord(other.getZ()) != SectionPos.blockToSectionCoord(pos.getZ());
                }
            }
        } finally {
            batch.draws = finishParts(PARTS);
            batch.signDraws = finishParts(SIGN_PARTS);
            batch.signs = SIGN_INDICES.isEmpty() ? NO_INTS : SIGN_INDICES.toIntArray();
            batch.signsNear = batch.signsNear.length == batch.signs.length ? batch.signsNear : new boolean[batch.signs.length];
            batch.anyNear = false;
            SIGN_INDICES.clear();
            GLYPHS.clear();
            GLYPH_TEXTS.clear();
        }

        rebuiltThisFrame += count;
        batch.linked = linked;
        batch.moving = moving != null ? moving : List.of();
        batch.viewDistance = viewDistance;
        if (count > 0 && batch.draws.length == 0 && batch.signDraws.length == 0) {
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
            // SubmitNodeCollection.canRenderAsSolid
            if (text.displayMode != Font.DisplayMode.POLYGON_OFFSET || ARGB.alpha(text.color) != 255
                    || ARGB.alpha(text.backgroundColor) != 0) {
                return false;
            }
            if (text.outlineColor == 0) {
                glyphText = t << 1;
                SignTextCache.prepareText(font, text.string, text.x, text.y, text.color, text.dropShadow, false, text.backgroundColor)
                        .visit(GLYPH_COLLECTOR);
            } else {
                // TextFeatureRenderer.renderText: the outline without polygon offset, then the text without shadow or background
                glyphText = t << 1 | 1;
                SignTextCache.prepare8xTextOutline(font, text.string, text.x, text.y, text.outlineColor).visit(GLYPH_COLLECTOR);
                glyphText = t << 1;
                SignTextCache.prepareText(font, text.string, text.x, text.y, text.color, false, false, 0).visit(GLYPH_COLLECTOR);
            }
        }
        for (int g = 0; g < GLYPHS.size(); g++) {
            if (retainedPipeline(GLYPHS.get(g).renderType(glyphMode(GLYPH_TEXTS.getInt(g)))) == null) {
                return false;
            }
        }
        return true;
    }

    private static Font.DisplayMode glyphMode(int glyphText) {
        return (glyphText & 1) != 0 ? Font.DisplayMode.NORMAL : Font.DisplayMode.POLYGON_OFFSET;
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

    private static Part part(PartSet set, RenderType renderType) {
        for (int i = 0; i < set.count; i++) {
            Part part = set.parts.get(i);
            if (part.renderType == renderType) {
                return part;
            }
        }
        if (set.count == set.parts.size()) {
            set.parts.add(new Part());
        }
        Part part = set.parts.get(set.count++);
        part.renderType = renderType;
        part.pipeline = retainedPipeline(renderType);
        part.builder = new BufferBuilder(part.bytes, PrimitiveTopology.QUADS, renderType.format());
        part.signEnds.clear();
        // Only the opaque entity pipelines are split: their order across block entities only matters where two draw the same depth
        part.split = set == PARTS && Initializer.CONFIG.retainedCellCulling
                && (part.pipeline == SOLID || part.pipeline == CUTOUT || part.pipeline == CUTOUT_CULL);
        if (set == SIGN_PARTS) {
            // No quads for the signs before this one
            part.signEnds.size(SIGN_INDICES.size());
        }
        return part;
    }

    /**
     * Uploads the parts' vertices, one draw per render type, and releases the builders. Sign parts keep their order
     * (sign by sign) and are drawn without the per plane culling.
     */
    private static Draw[] finishParts(PartSet set) {
        Draw[] draws = NO_DRAWS;
        int drawn = 0;
        boolean signs = set == SIGN_PARTS;
        for (int i = 0; i < set.count; i++) {
            Part part = set.parts.get(i);
            if (part.split) {
                // One draw per cell with geometry, in cell order; the part's own builder took nothing
                for (int cell = 0; cell < CELLS; cell++) {
                    BufferBuilder builder = part.cellBuilders[cell];
                    part.cellBuilders[cell] = null;
                    Draw draw = builder != null ? finishDraw(part, builder, false) : null;
                    if (draw != null) {
                        if (drawn == draws.length) {
                            draws = Arrays.copyOf(draws, Math.max(set.count, drawn * 2));
                        }
                        draws[drawn++] = draw;
                    }
                }
                try (MeshData unused = part.builder.build()) {
                }
            } else {
                Draw draw = finishDraw(part, part.builder, signs);
                if (draw != null) {
                    if (drawn == draws.length) {
                        draws = Arrays.copyOf(draws, Math.max(set.count, drawn * 2));
                    }
                    draws[drawn++] = draw;
                }
            }
            part.builder = null;
            part.renderType = null;
            part.pipeline = null;
            part.signEnds.clear();
        }
        set.count = 0;
        return drawn == draws.length ? draws : Arrays.copyOf(draws, drawn);
    }

    /** Uploads one builder's vertices of a part as a draw, or null if it has none */
    private static @Nullable Draw finishDraw(Part part, BufferBuilder builder, boolean signs) {
        try (MeshData data = builder.build()) {
            if (data == null) {
                return null;
            }
            // Reorders the vertices into the buffer that is uploaded
            SectionFaceGroups faces = null;
            ByteBuffer sorted = null;
            if (!signs && part.pipeline.isCull()) {
                sorted = MemoryUtil.memAlloc(data.vertexBuffer().remaining());
                faces = SectionFaceGroups.sortByPlanes(data, sorted);
            }
            GpuBuffer buffer;
            try {
                buffer = RenderSystem.getDevice().createBuffer(() -> "Ruthenium retained block entities",
                        GpuBuffer.USAGE_VERTEX, faces != null ? sorted : data.vertexBuffer());
            } finally {
                if (sorted != null) {
                    MemoryUtil.memFree(sorted);
                }
            }
            return new Draw(part.pipeline, part.renderType, buffer, data.drawState().vertexCount() / 4 * 6, faces,
                    signs ? part.signEnds.toIntArray() : null, part.split ? bounds(data, part.renderType.format()) : NO_BOUNDS);
        }
    }

    /** The batch-relative bounds of a mesh's positions (min x, y, z, max x, y, z), or {@link #NO_BOUNDS} */
    private static float[] bounds(MeshData data, VertexFormat format) {
        var position = format.getElement("Position");
        if (position == null || position.offset() != 0) {
            return NO_BOUNDS;
        }
        ByteBuffer vertices = data.vertexBuffer();
        long base = MemoryUtil.memAddress(vertices);
        int stride = format.getVertexSize();
        int count = data.drawState().vertexCount();
        float minX = Float.POSITIVE_INFINITY, minY = Float.POSITIVE_INFINITY, minZ = Float.POSITIVE_INFINITY;
        float maxX = Float.NEGATIVE_INFINITY, maxY = Float.NEGATIVE_INFINITY, maxZ = Float.NEGATIVE_INFINITY;
        for (int v = 0; v < count; v++) {
            long p = base + (long) v * stride;
            float x = MemoryUtil.memGetFloat(p), y = MemoryUtil.memGetFloat(p + 4), z = MemoryUtil.memGetFloat(p + 8);
            if (!Float.isFinite(x) || !Float.isFinite(y) || !Float.isFinite(z)) {
                return NO_BOUNDS;
            }
            minX = Math.min(minX, x);
            minY = Math.min(minY, y);
            minZ = Math.min(minZ, z);
            maxX = Math.max(maxX, x);
            maxY = Math.max(maxY, y);
            maxZ = Math.max(maxZ, z);
        }
        return new float[]{minX, minY, minZ, maxX, maxY, maxZ};
    }

    /** Whether a draw's geometry lies entirely outside the frame's cull frustum */
    private static boolean isCulled(Batch batch, Draw draw) {
        float[] b = draw.bounds;
        Frustum frustum = cullFrustum;
        if (b.length == 0 || frustum == null || !Initializer.CONFIG.retainedCellCulling) {
            return false;
        }
        int result = ((FrustumAccessor) frustum).invokeCubeInFrustum(
                batch.originX + b[0] - CELL_CULL_MARGIN, batch.originY + b[1] - CELL_CULL_MARGIN, batch.originZ + b[2] - CELL_CULL_MARGIN,
                batch.originX + b[3] + CELL_CULL_MARGIN, batch.originY + b[4] + CELL_CULL_MARGIN, batch.originZ + b[5] + CELL_CULL_MARGIN);
        return result != -2 && result != -1;
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
            count += batch.draws.length + batch.signDraws.length;
        }
        if (count == 0) {
            return;
        }

        Matrix4f modelView = RenderSystem.getModelViewMatrixCopy();
        DynamicGpuData.Transform[] data = new DynamicGpuData.Transform[count];
        int maxIndexCount = 0;
        int i = 0;
        if (culled.length < count) {
            culled = new boolean[Math.max(count, culled.length * 2)];
        }
        if (transformOf.length < count) {
            transformOf = new int[Math.max(count, transformOf.length * 2)];
        }
        int d = 0;
        for (Batch batch : VISIBLE) {
            RenderType lastType = null;
            for (int set = 0; set < 2; set++) {
                for (Draw draw : set == 0 ? batch.draws : batch.signDraws) {
                    boolean skip = culled[d] = isCulled(batch, draw);
                    if (skip) {
                        d++;
                        continue;
                    }
                    // The cells of a part share their batch's transform (and the draws then keep its binding)
                    if (draw.renderType == lastType) {
                        transformOf[d++] = i - 1;
                        maxIndexCount = Math.max(maxIndexCount, draw.indexCount);
                        continue;
                    }
                    lastType = draw.renderType;
                    transformOf[d++] = i;
                    // Fresh vectors: the uniform storage compares against the last record it wrote
                    Vector3f offset = new Vector3f((float) (batch.originX - camX), (float) (batch.originY - camY), (float) (batch.originZ - camZ));
                    data[i++] = new DynamicGpuData.Transform(modelView(draw.renderType, modelView), WHITE, offset, IDENTITY);
                    maxIndexCount = Math.max(maxIndexCount, draw.indexCount);
                    PREPARED.computeIfAbsent(draw.renderType, RenderType::prepare);
                }
            }
        }
        transforms = i == 0 ? new GpuBufferSlice[0]
                : RenderSystem.getDynamicUniforms().writeTransforms(i == count ? data : Arrays.copyOf(data, i));
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

        renderPass.pushDebugGroup(() -> "Ruthenium retained block entities");
        RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS);
        boundPipeline = null;
        boundType = null;
        boundTransform = null;
        boundBuffer = null;
        boolean faceCulling = Initializer.CONFIG.modelBackFaceCulling;
        int d = 0;
        for (Batch batch : VISIBLE) {
            float eyeX = (float) (camX - batch.originX), eyeY = (float) (camY - batch.originY), eyeZ = (float) (camZ - batch.originZ);
            for (int set = 0; set < 2; set++) {
                Draw[] draws = set == 0 ? batch.draws : batch.signDraws;
                for (int k = 0; k < draws.length; ) {
                    // A part's cells (consecutive draws of one render type with bounds) nearest to the eye first: the cells'
                    // order otherwise breaks the near to far order the planes gave the whole section, and occluded fragments
                    // are shaded
                    int end = k + 1;
                    if (draws[k].bounds.length != 0) {
                        while (end < draws.length && draws[end].renderType == draws[k].renderType && draws[end].bounds.length != 0) {
                            end++;
                        }
                    }
                    int n = end - k;
                    for (int j = 0; j < n; j++) {
                        int index = k + j;
                        float distance = n == 1 ? 0.0F : centerDistance(draws[index].bounds, eyeX, eyeY, eyeZ);
                        int at = j;
                        while (at > 0 && CELL_DISTANCES[at - 1] > distance) {
                            CELL_ORDER[at] = CELL_ORDER[at - 1];
                            CELL_DISTANCES[at] = CELL_DISTANCES[at - 1];
                            at--;
                        }
                        CELL_ORDER[at] = index;
                        CELL_DISTANCES[at] = distance;
                    }
                    if (n > 1 && faceCulling && draws[k].faces != null) {
                        // Group by group, as one section-wide buffer sorted by planes draws them, each group's cells near to far
                        for (int g = 0; g < SectionFaceGroups.GROUP_COUNT; g++) {
                            for (int j = 0; j < n; j++) {
                                int index = CELL_ORDER[j];
                                Draw draw = draws[index];
                                if (culled[d + index]) {
                                    continue;
                                }
                                int quads = draw.faces.visibleGroupQuads(g, eyeX, eyeY, eyeZ, FIRST_QUAD);
                                if (quads > 0 && bind(renderPass, indices, draw, transforms[transformOf[d + index]])) {
                                    renderPass.drawIndexed(quads * 6, 1, 0, FIRST_QUAD[0] * 4, 0);
                                }
                            }
                        }
                    } else {
                        for (int j = 0; j < n; j++) {
                            int index = CELL_ORDER[j];
                            if (!culled[d + index]) {
                                drawOne(renderPass, indices, batch, draws[index], transforms[transformOf[d + index]], faceCulling, eyeX, eyeY, eyeZ);
                            }
                        }
                    }
                    k = end;
                }
                d += draws.length;
            }
        }
        renderPass.popDebugGroup();
    }

    /** The squared distance from the eye to the center of a draw's bounds */
    private static float centerDistance(float[] bounds, float eyeX, float eyeY, float eyeZ) {
        float dx = (bounds[0] + bounds[3]) * 0.5F - eyeX, dy = (bounds[1] + bounds[4]) * 0.5F - eyeY, dz = (bounds[2] + bounds[5]) * 0.5F - eyeZ;
        return dx * dx + dy * dy + dz * dz;
    }

    private static void drawOne(RenderPass renderPass, RenderSystem.AutoStorageIndexBuffer indices, Batch batch, Draw draw,
                                GpuBufferSlice transform, boolean faceCulling, float eyeX, float eyeY, float eyeZ) {
        if (!bind(renderPass, indices, draw, transform)) {
            return;
        }
        if (draw.signEnds != null && batch.anyNear) {
            drawSignsNotNear(renderPass, draw.signEnds, batch.signsNear);
        } else if (faceCulling && draw.faces != null) {
            // Quads that face away from the eye would be culled by the pipeline
            int runs = draw.faces.visiblePlaneRuns(eyeX, eyeY, eyeZ, RUNS);
            for (int r = 0; r < runs; r++) {
                renderPass.drawIndexed(RUNS[r * 2 + 1] * 6, 1, 0, RUNS[r * 2] * 4, 0);
            }
        } else {
            renderPass.drawIndexed(draw.indexCount, 1, 0, 0, 0);
        }
    }

    /** Binds what a draw needs (only what changed); false if its pipeline is not compiled */
    private static boolean bind(RenderPass renderPass, RenderSystem.AutoStorageIndexBuffer indices, Draw draw, GpuBufferSlice transform) {
        if (draw.pipeline != boundPipeline) {
            CompiledRenderPipeline pipeline = RenderSystem.getCompiledPipelineNullable(draw.pipeline);
            if (pipeline == null) {
                return false;
            }
            boundPipeline = draw.pipeline;
            boundType = null;
            boundTransform = null;
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
        if (transform != boundTransform) {
            boundTransform = transform;
            renderPass.setUniform("DynamicTransforms", transform);
        }
        if (draw.buffer != boundBuffer) {
            boundBuffer = draw.buffer;
            renderPass.setVertexBuffer(0, draw.buffer.slice());
        }
        return true;
    }

    /** Draws the quads of the signs that stay retained this frame, in runs of consecutive signs */
    private static void drawSignsNotNear(RenderPass renderPass, int[] signEnds, boolean[] near) {
        int start = 0;
        int runStart = -1;
        for (int k = 0; k < signEnds.length; k++) {
            if (!near[k]) {
                if (runStart < 0) {
                    runStart = start;
                }
            } else if (runStart >= 0) {
                drawQuads(renderPass, runStart, start - runStart);
                runStart = -1;
            }
            start = signEnds[k];
        }
        if (runStart >= 0) {
            drawQuads(renderPass, runStart, start - runStart);
        }
    }

    private static void drawQuads(RenderPass renderPass, int first, int count) {
        if (count > 0) {
            renderPass.drawIndexed(count * 6, 1, 0, first * 4, 0);
        }
    }
}
