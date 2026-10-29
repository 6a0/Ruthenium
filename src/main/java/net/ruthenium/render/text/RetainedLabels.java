package net.ruthenium.render.text;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.systems.ScissorState;
import com.mojang.blaze3d.vertex.CompactVectorArray;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.blaze3d.vertex.VertexSorting;
import com.mojang.renderpearl.api.GpuFormat;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.device.DeviceInfo;
import com.mojang.renderpearl.api.pipeline.BindGroupLayout;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.CompiledRenderPipeline;
import com.mojang.renderpearl.api.pipeline.IndexType;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.renderpearl.api.pipeline.UniformType;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.font.TextRenderable;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.StagedVertexBuffer;
import net.minecraft.client.renderer.feature.TextFeatureRenderer;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.resources.Identifier;
import net.minecraft.util.ARGB;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.ExtendedVertexBuilder;
import net.ruthenium.interfaces.LabelDrawHolder;
import net.ruthenium.interfaces.PosedDrawGroup;
import net.ruthenium.interfaces.PosedDrawHolder;
import net.ruthenium.render.entity.ModelCulling;
import net.ruthenium.render.vertex.DistanceSorting;
import net.ruthenium.render.vertex.SortOrderCache;
import org.joml.Matrix4f;
import org.joml.Matrix4fc;
import org.joml.Vector3f;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * Labels (name tags, text display lines, any text whose prepared glyphs {@link SignTextCache} keeps) drawn from a mesh of
 * their glyph quads in label space and a per-frame pose, instead of writing their vertices every frame
 * ({@code retainedLabels}).
 * <p>
 * A label's glyphs are the same quads every frame, under the submit's pose (camera rotation, scale, translation). Each
 * kept prepared text ({@link SignTextCache.KeptText}) gets a mesh once (from its second frame on): the quads vanilla writes,
 * recorded in label space (the positions vanilla hands to {@code addVertex(pose, x, y, z)}), with their UV and color, in a
 * shared GPU buffer (the arena), each vertex tagged with the label's region. Each frame only the region's pose rows and
 * light are written (a texel buffer the vertex shader reads, {@code label_text.vsh}).
 * <p>
 * Exactness: the draws are vanilla's. Each run of glyphs of one render type picks the group's draw exactly as
 * {@code Group.getVertexBuilder} would (joining the last draw, or {@code getOrAddDraw}: merged, new, strictly ordered,
 * whatever the group does), and the label is added to that draw in place of its vertices, so draws, their order and the
 * quad order within each draw are vanilla's. The draw's indices list its labels' quads in submission order; a draw sorted
 * on upload ({@code text_polygon_offset}) sorts them with its own sorting from centroids computed on the CPU with the same
 * JOML {@code transformPosition} of vertices 0 and 2 and the same mean that {@code MeshData.decodeQuadCentroids} reads.
 * Positions are computed on the GPU in JOML's operation order ({@code precise}), so they are bit-identical to the
 * written ones; UV, color and light are the same values. A draw holds either vanilla vertices or labels: a label whose
 * draw already has vertices is written as vanilla does, and a vanilla write joining a label draw writes its labels first
 * ({@link #materialize}).
 * <p>
 * Only in the world's features (not the hand or GUI), without improved transparency, for glyphs of
 * {@code BakedSheetGlyph} and render types of vanilla's six world text pipelines; obfuscated text is never kept
 * ({@link SignTextCache}), and outlined text (glowing signs) and standalone backgrounds take vanilla's path. Render thread
 * only.
 */
public final class RetainedLabels {
    public static final VertexFormat MESH_FORMAT = VertexFormat.builder(0)
            .addAttribute("Position", GpuFormat.RGB32_FLOAT)
            .addAttribute("UV0", GpuFormat.RG32_FLOAT)
            .addAttribute("Color", GpuFormat.RGBA8_UNORM)
            .addAttribute("LabelRegion", GpuFormat.RG16_SINT)
            .build();
    private static final int VERTEX_SIZE = MESH_FORMAT.getVertexSize();
    /** Regions per arena: four RGBA32F texels each, within the 65536 texels every texel buffer supports */
    private static final int MAX_REGIONS = 16384;
    private static final int REGION_SIZE = 64;
    /** Quads per label at most (longer texts take vanilla's path) */
    private static final int MAX_LABEL_QUADS = 2048;
    private static final int MIN_ARENA_VERTICES = 1 << 16;
    private static final int MAX_ARENA_VERTICES = 1 << 22;
    private static final BindGroupLayout POSES = BindGroupLayout.builder()
            .withUniform("LabelPoses", UniformType.TEXEL_BUFFER, GpuFormat.RGBA32_FLOAT).build();

    /** Vanilla's world text pipelines and their copies with the label vertex shader */
    private static final Map<RenderPipeline, RenderPipeline> COPIES = new IdentityHashMap<>();
    public static final List<RenderPipeline> PIPELINES = new ArrayList<>();

    static {
        copy(RenderPipelines.TEXT, "text");
        copy(RenderPipelines.TEXT_GRAYSCALE, "text_grayscale");
        copy(RenderPipelines.TEXT_POLYGON_OFFSET, "text_polygon_offset");
        copy(RenderPipelines.TEXT_GRAYSCALE_POLYGON_OFFSET, "text_grayscale_polygon_offset");
        copy(RenderPipelines.TEXT_SEE_THROUGH, "text_see_through");
        copy(RenderPipelines.TEXT_GRAYSCALE_SEE_THROUGH, "text_grayscale_see_through");
    }

    /** The glyph classes whose quads depend only on the glyph instance ({@code BakedSheetGlyph}'s glyphs and effects) */
    private static final @Nullable Class<?> GLYPH_CLASS = classOrNull("net.minecraft.client.gui.font.glyphs.BakedSheetGlyph$GlyphInstance");
    private static final @Nullable Class<?> EFFECT_CLASS = classOrNull("net.minecraft.client.gui.font.glyphs.BakedSheetGlyph$EffectInstance");
    private static final Object FAILED = new Object();
    private static final Run[] NO_RUNS = new Run[0];

    private static boolean clearPending;
    private static @Nullable Boolean pipelinesAvailable;
    private static final Map<RenderPipeline, CompiledRenderPipeline> COMPILED = new IdentityHashMap<>();
    /** Whether this frame's world features may take label draws */
    private static boolean frameActive;
    private static int frame;
    private static @Nullable PosedDrawGroup preparingGroup;

    /** The arena: label meshes of the current generation, appended; replaced when full */
    private static @Nullable GpuBuffer arena;
    private static int arenaGeneration;
    private static int arenaCapacity;
    private static int arenaVertices;
    private static int arenaRegions;
    private static boolean arenaFull;
    /** Vertices of the meshes drawn (or to be drawn) recently and this frame, which size the next arena */
    private static int liveVertices;
    private static int frameVertices;
    private static int frameMeshes;
    /** This frame's new meshes, written into the arena from {@link #pendingStart} at the upload */
    private static ByteBuffer pending = MemoryUtil.memAlloc(1 << 16);
    private static int pendingStart;
    private static int pendingSize;

    /** This frame's labels: pose, light and mesh per label submit */
    private static final List<Matrix4f> LABEL_POSES = new ArrayList<>();
    private static Mesh[] labelMeshes = new Mesh[64];
    private static int[] labelLights = new int[64];
    private static int labelCount;
    private static final List<LabelDraw> DRAWS = new ArrayList<>();
    private static int drawCount;
    /** The regions' pose rows and light (the texel buffer's layout) */
    private static @Nullable ByteBuffer poseBytes;
    private static int maxRegion;
    private static ByteBuffer indexBytes = MemoryUtil.memAlloc(1 << 16);
    private static int indexSize;
    private static @Nullable MappableRingBuffer poseRing;
    private static @Nullable MappableRingBuffer indexRing;
    private static ByteBuffer commandBytes = MemoryUtil.memAlloc(1 << 12);
    private static int commandSize;
    private static @Nullable MappableRingBuffer commandRing;
    private static @Nullable Boolean multiDrawIndirect;
    private static int maxIndirectDraws;
    private static @Nullable GpuBuffer framePoses;
    private static @Nullable GpuBuffer frameIndices;
    private static @Nullable GpuBuffer frameCommands;
    private static @Nullable GpuBuffer frameArena;
    private static final SortOrderCache SORT_ORDERS = new SortOrderCache();
    private static boolean sortOrdersUsed;
    private static final Vector3f SCRATCH = new Vector3f();
    private static final Recorder RECORDER = new Recorder();
    private static final boolean INLINE_TRANSFORM = checkInlineTransform();

    private RetainedLabels() {
    }

    private static @Nullable Class<?> classOrNull(String name) {
        try {
            return Class.forName(name, false, RetainedLabels.class.getClassLoader());
        } catch (ClassNotFoundException e) {
            return null;
        }
    }

    private static void copy(RenderPipeline vanilla, String name) {
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
        RenderPipeline copy = RenderPipeline.builder(snippet)
                .withLocation(Identifier.fromNamespaceAndPath("ruthenium", "pipeline/label_" + name))
                .withVertexShader(Identifier.fromNamespaceAndPath("ruthenium", "core/label_text"))
                .withVertexBinding(0, MESH_FORMAT)
                .withBindGroupLayout(POSES)
                .build();
        COPIES.put(vanilla, copy);
        PIPELINES.add(copy);
    }

    /** A run of a label's glyphs with one render type: one draw selection, as vanilla's per-glyph selections */
    record Run(RenderType renderType, int firstQuad, int quads) {
    }

    /** A kept text's quads in label space, and where they are in the arena */
    static final class Mesh {
        final int quads;
        /** Per vertex: x, y, z as handed to addVertex(pose, ...); u, v; ARGB color */
        final float[] positions;
        final float[] uvs;
        final int[] colors;
        final TextRenderable[] renderables;
        /** The quad each renderable's quads end at */
        final int[] ends;
        final Run[][] runs = new Run[Font.DisplayMode.values().length][];
        int arenaGeneration = -1;
        int baseVertex;
        int region;
        int usedFrame = -1;

        Mesh(int quads, float[] positions, float[] uvs, int[] colors, TextRenderable[] renderables, int[] ends) {
            this.quads = quads;
            this.positions = positions;
            this.uvs = uvs;
            this.colors = colors;
            this.renderables = renderables;
            this.ends = ends;
        }

        Run[] runs(Font.DisplayMode mode) {
            Run[] runs = this.runs[mode.ordinal()];
            if (runs == null) {
                runs = this.runs[mode.ordinal()] = buildRuns(mode);
            }
            return runs;
        }

        private Run[] buildRuns(Font.DisplayMode mode) {
            List<Run> runs = new ArrayList<>();
            RenderType current = null;
            int start = 0;
            int quad = 0;
            for (int i = 0; i < this.renderables.length; i++) {
                RenderType renderType = this.renderables[i].renderType(mode);
                if (!COPIES.containsKey(renderType.pipeline()) || !renderType.canConsolidateConsecutiveGeometry()) {
                    return NO_RUNS;
                }
                if (renderType != current) {
                    if (current != null) {
                        runs.add(new Run(current, start, quad - start));
                    }
                    current = renderType;
                    start = quad;
                }
                quad = this.ends[i];
            }
            if (current != null) {
                runs.add(new Run(current, start, quad - start));
            }
            return runs.isEmpty() ? NO_RUNS : runs.toArray(new Run[0]);
        }
    }

    /** A group draw whose geometry is labels: each use is a run of a label (its quads, under the label's pose) */
    public static final class LabelDraw {
        StagedVertexBuffer.@Nullable Draw draw;
        @Nullable PreparedRenderType prepared;
        @Nullable VertexSorting sorting;
        int[] useLabel = new int[16];
        int[] useFirstQuad = new int[16];
        int[] useQuads = new int[16];
        int uses;
        int quads;
        int firstIndex;
        int indexCount;
        IndexType indexType = IndexType.INT;
        int baseVertex;
        int firstCommand;
        int commandCount;
        @Nullable CompactVectorArray centroids;
        int[] quadVertices = new int[0];
        /** The last frame's sort of this draw slot: its inputs (uses, their meshes, places and poses) and its order */
        @Nullable VertexSorting sortedWith;
        Mesh[] sortedMeshes = new Mesh[0];
        int[] sortedUses = new int[0];
        float[] sortedPoses = new float[0];
        int sortedCount = -1;
        int @Nullable [] sortedOrder;

        /** Whether this frame's sort inputs are bit for bit last frame's: then so are the centroids and the order */
        boolean sameSortInputs(VertexSorting sorting) {
            if (this.sortedOrder == null || this.sortedWith != sorting || this.sortedCount != this.uses) {
                return false;
            }
            for (int u = 0; u < this.uses; u++) {
                int label = this.useLabel[u];
                Mesh mesh = labelMeshes[label];
                if (this.sortedMeshes[u] != mesh || this.sortedUses[u * 3] != this.useFirstQuad[u] || this.sortedUses[u * 3 + 1] != this.useQuads[u]
                        || this.sortedUses[u * 3 + 2] != mesh.baseVertex || !samePose(LABEL_POSES.get(label), this.sortedPoses, u * 13)) {
                    return false;
                }
            }
            return true;
        }

        void keepSortInputs(VertexSorting sorting, int[] order) {
            int n = this.uses;
            if (this.sortedMeshes.length < n) {
                this.sortedMeshes = new Mesh[Math.max(n, this.sortedMeshes.length * 2)];
                this.sortedUses = new int[this.sortedMeshes.length * 3];
                this.sortedPoses = new float[this.sortedMeshes.length * 13];
            }
            for (int u = 0; u < n; u++) {
                int label = this.useLabel[u];
                Mesh mesh = labelMeshes[label];
                this.sortedMeshes[u] = mesh;
                this.sortedUses[u * 3] = this.useFirstQuad[u];
                this.sortedUses[u * 3 + 1] = this.useQuads[u];
                this.sortedUses[u * 3 + 2] = mesh.baseVertex;
                keepPose(LABEL_POSES.get(label), this.sortedPoses, u * 13);
            }
            Arrays.fill(this.sortedMeshes, n, this.sortedMeshes.length, null);
            this.sortedCount = n;
            this.sortedWith = sorting;
            this.sortedOrder = order == this.sortedOrder ? order : Arrays.copyOf(order, order.length);
        }

        void reset(StagedVertexBuffer.Draw draw, PreparedRenderType prepared, @Nullable VertexSorting sorting) {
            this.draw = draw;
            this.prepared = prepared;
            this.sorting = sorting;
            this.uses = 0;
            this.quads = 0;
            this.indexCount = 0;
            this.commandCount = 0;
        }

        void clear() {
            this.draw = null;
            this.prepared = null;
            this.sorting = null;
            this.uses = 0;
            this.quads = 0;
            this.indexCount = 0;
        }

        void add(int label, int firstQuad, int quads) {
            int n = this.uses;
            if (n == this.useLabel.length) {
                this.useLabel = Arrays.copyOf(this.useLabel, n * 2);
                this.useFirstQuad = Arrays.copyOf(this.useFirstQuad, n * 2);
                this.useQuads = Arrays.copyOf(this.useQuads, n * 2);
            }
            this.useLabel[n] = label;
            this.useFirstQuad[n] = firstQuad;
            this.useQuads[n] = quads;
            this.uses = n + 1;
            this.quads += quads;
        }
    }

    /**
     * Records a label's quads: the label-space position each vertex is handed with its pose, and its color and UV.
     * Light, overlay and normals are not part of text vertices' kept data (the light is the submit's, the others
     * aren't in the text formats).
     */
    private static final class Recorder implements VertexConsumer {
        float[] positions = new float[256 * 3];
        float[] uvs = new float[256 * 2];
        int[] colors = new int[256];
        int vertices;
        boolean failed;

        void reset() {
            this.vertices = 0;
            this.failed = false;
        }

        @Override
        public VertexConsumer addVertex(Matrix4fc pose, float x, float y, float z) {
            int n = this.vertices;
            if (n == this.colors.length) {
                this.positions = Arrays.copyOf(this.positions, n * 6);
                this.uvs = Arrays.copyOf(this.uvs, n * 4);
                this.colors = Arrays.copyOf(this.colors, n * 2);
            }
            this.positions[n * 3] = x;
            this.positions[n * 3 + 1] = y;
            this.positions[n * 3 + 2] = z;
            this.uvs[n * 2] = 0.0F;
            this.uvs[n * 2 + 1] = 0.0F;
            this.colors[n] = 0;
            this.vertices = n + 1;
            return this;
        }

        @Override
        public VertexConsumer addVertex(float x, float y, float z) {
            // A vertex without its label-space position
            this.failed = true;
            return this;
        }

        @Override
        public VertexConsumer setColor(int r, int g, int b, int a) {
            return this.setColor(ARGB.color(a, r, g, b));
        }

        @Override
        public VertexConsumer setColor(int color) {
            if (this.vertices == 0) {
                this.failed = true;
            } else {
                this.colors[this.vertices - 1] = color;
            }
            return this;
        }

        @Override
        public VertexConsumer setUv(float u, float v) {
            if (this.vertices == 0) {
                this.failed = true;
            } else {
                this.uvs[(this.vertices - 1) * 2] = u;
                this.uvs[(this.vertices - 1) * 2 + 1] = v;
            }
            return this;
        }

        @Override
        public VertexConsumer setUv1(int u, int v) {
            return this;
        }

        @Override
        public VertexConsumer setUv2(int u, int v) {
            return this;
        }

        @Override
        public VertexConsumer setUv3(float u, float v) {
            return this;
        }

        @Override
        public VertexConsumer setNormal(float x, float y, float z) {
            return this;
        }

        @Override
        public VertexConsumer setLineWidth(float width) {
            return this;
        }
    }

    /** Drops every mesh's arena placement at the start of the next frame (resource reload, switch toggled) */
    public static void clear() {
        clearPending = true;
    }

    /** Called before the world's features are prepared */
    public static void beginFrame() {
        frame++;
        if (clearPending) {
            clearPending = false;
            pipelinesAvailable = null;
            COMPILED.clear();
            SORT_ORDERS.clear();
            arenaFull = true;
        }
        if (arenaFull) {
            // A new arena; meshes are placed again when next used
            arenaFull = false;
            if (arena != null) {
                arena.close();
                arena = null;
            }
            arenaGeneration++;
            arenaVertices = 0;
            arenaRegions = 0;
            arenaCapacity = 0;
        }
        for (int i = 0; i < drawCount; i++) {
            DRAWS.get(i).clear();
        }
        drawCount = 0;
        Arrays.fill(labelMeshes, 0, labelCount, null);
        labelCount = 0;
        indexSize = 0;
        maxRegion = -1;
        pendingStart = arenaVertices;
        pendingSize = 0;
        liveVertices = Math.max(frameVertices, liveVertices / 2);
        frameVertices = 0;
        frameMeshes = 0;
        frameActive = Initializer.CONFIG.retainedLabels && GLYPH_CLASS != null && EFFECT_CLASS != null
                && !Minecraft.getInstance().gameRenderer.useImprovedTransparency() && arePipelinesAvailable();
    }

    /** A feature renderer group starts preparing */
    public static void beginGroup(PosedDrawGroup group) {
        preparingGroup = group;
    }

    private static boolean arePipelinesAvailable() {
        Boolean available = pipelinesAvailable;
        if (available == null) {
            available = true;
            for (RenderPipeline copy : PIPELINES) {
                if (RenderSystem.getCompiledPipelineNullable(copy) == null) {
                    available = false;
                    break;
                }
            }
            pipelinesAvailable = available;
        }
        return available;
    }

    /**
     * {@code TextFeatureRenderer.renderText} visiting a prepared text with the glyph renderer: draws it as a retained
     * label and returns true, or returns false for vanilla's visit
     */
    public static boolean submit(SignTextCache.KeptText text, TextFeatureRenderer.GlyphRenderer renderer) {
        PosedDrawGroup group = preparingGroup;
        if (!frameActive || group == null || !ModelCulling.isWorldPass() || !Initializer.CONFIG.retainedLabels) {
            return false;
        }
        Mesh mesh = meshOf(text);
        if (mesh == null) {
            return false;
        }
        Run[] runs = mesh.runs(renderer.displayMode);
        if (runs.length == 0) {
            return false;
        }

        Matrix4f pose = renderer.pose;
        int light = renderer.lightCoords;
        // A mesh drawn twice in a frame (one region, one pose) is written as vanilla does the second time
        boolean retain = mesh.usedFrame != frame && place(mesh);
        int label = -1;
        for (Run run : runs) {
            RenderType renderType = run.renderType();
            if (retain) {
                // The draw Group.getVertexBuilder would write this run into
                StagedVertexBuffer.Draw draw = group.ruthenium$joiningDraw(renderType);
                if (draw == null) {
                    draw = group.ruthenium$addDraw(renderType);
                }
                LabelDraw labelDraw = ((LabelDrawHolder) draw).ruthenium$labelDraw();
                if (labelDraw == null && !group.ruthenium$hasVertices(draw) && ((PosedDrawHolder) draw).ruthenium$poseDraw() == null) {
                    labelDraw = addLabelDraw(draw, group.ruthenium$preparedTypeOf(draw));
                }
                if (labelDraw != null) {
                    if (label == -1) {
                        label = addLabel(mesh, pose, light);
                    }
                    labelDraw.add(label, run.firstQuad(), run.quads());
                    continue;
                }
            }
            // The draw has vertices: written as vanilla writes them (the draw is the one just selected)
            write(group.ruthenium$vertexBuilder(renderType), mesh, run.firstQuad(), run.quads(), pose, light);
        }
        return true;
    }

    private static @Nullable Mesh meshOf(SignTextCache.KeptText text) {
        Object label = text.label;
        if (label instanceof Mesh mesh) {
            return mesh;
        }
        if (label == FAILED) {
            return null;
        }
        // From the second frame a text is drawn in: texts drawn once (fading opacity, changing scores) stay vanilla's
        if (text.firstFrame == -1 || text.firstFrame == frame) {
            text.firstFrame = frame;
            return null;
        }
        Mesh mesh = build(text);
        text.label = mesh != null ? mesh : FAILED;
        return mesh;
    }

    private static @Nullable Mesh build(SignTextCache.KeptText text) {
        Recorder recorder = RECORDER;
        recorder.reset();
        List<TextRenderable> renderables = new ArrayList<>();
        int[][] ends = {new int[16]};
        Matrix4f identity = new Matrix4f();
        text.visit(new Font.GlyphVisitor() {
            @Override
            public void acceptRenderable(TextRenderable renderable) {
                Class<?> type = renderable.getClass();
                if (type != GLYPH_CLASS && type != EFFECT_CLASS) {
                    recorder.failed = true;
                    return;
                }
                renderable.render(identity, recorder, 0, false);
                int n = renderables.size();
                if (n == ends[0].length) {
                    ends[0] = Arrays.copyOf(ends[0], n * 2);
                }
                ends[0][n] = recorder.vertices / 4;
                renderables.add(renderable);
                if (recorder.vertices % 4 != 0) {
                    recorder.failed = true;
                }
            }
        });
        int vertices = recorder.vertices;
        if (recorder.failed || vertices == 0 || vertices % 4 != 0 || vertices / 4 > MAX_LABEL_QUADS) {
            return null;
        }
        return new Mesh(vertices / 4, Arrays.copyOf(recorder.positions, vertices * 3), Arrays.copyOf(recorder.uvs, vertices * 2),
                Arrays.copyOf(recorder.colors, vertices), renderables.toArray(new TextRenderable[0]),
                Arrays.copyOf(ends[0], renderables.size()));
    }

    /** Places the mesh in the arena (its vertices go up with this frame's upload); false if the arena is full */
    private static boolean place(Mesh mesh) {
        int vertices = mesh.quads * 4;
        frameVertices += vertices;
        frameMeshes++;
        if (mesh.arenaGeneration != arenaGeneration) {
            if (arena == null) {
                arenaCapacity = Math.min(MAX_ARENA_VERTICES, Math.max(MIN_ARENA_VERTICES, Integer.highestOneBit(Math.max(liveVertices, 1) * 4)));
                arena = RenderSystem.getDevice().createBuffer(() -> "Ruthenium retained labels", GpuBuffer.USAGE_VERTEX | GpuBuffer.USAGE_COPY_DST,
                        (long) arenaCapacity * VERTEX_SIZE);
            }
            if (arenaVertices + vertices > arenaCapacity || arenaRegions >= MAX_REGIONS) {
                // A new arena next frame, sized for twice the labels drawn (meshes no longer drawn fill an arena over time,
                // and more labels may be drawn than the arena was sized for), unless even that couldn't hold them; until
                // then such labels are written as vanilla does
                if (frameVertices * 2 <= MAX_ARENA_VERTICES && frameMeshes < MAX_REGIONS / 2) {
                    arenaFull = true;
                }
                return false;
            }
            mesh.arenaGeneration = arenaGeneration;
            mesh.baseVertex = arenaVertices;
            mesh.region = arenaRegions++;
            arenaVertices += vertices;
            writeMesh(mesh);
        }
        mesh.usedFrame = frame;
        return true;
    }

    private static void writeMesh(Mesh mesh) {
        int vertices = mesh.quads * 4;
        int bytes = vertices * VERTEX_SIZE;
        if (pending.capacity() < pendingSize + bytes) {
            pending = MemoryUtil.memRealloc(pending, Math.max(pending.capacity() * 2, pendingSize + bytes));
        }
        long ptr = MemoryUtil.memAddress0(pending) + pendingSize;
        float[] positions = mesh.positions;
        float[] uvs = mesh.uvs;
        int[] colors = mesh.colors;
        int region = mesh.region;
        for (int v = 0; v < vertices; v++, ptr += VERTEX_SIZE) {
            MemoryUtil.memPutFloat(ptr, positions[v * 3]);
            MemoryUtil.memPutFloat(ptr + 4, positions[v * 3 + 1]);
            MemoryUtil.memPutFloat(ptr + 8, positions[v * 3 + 2]);
            MemoryUtil.memPutFloat(ptr + 12, uvs[v * 2]);
            MemoryUtil.memPutFloat(ptr + 16, uvs[v * 2 + 1]);
            // As BakedSheetGlyph's write (ABGR, i.e. RGBA bytes)
            MemoryUtil.memPutInt(ptr + 20, ARGB.toABGR(colors[v]));
            MemoryUtil.memPutShort(ptr + 24, (short) region);
            MemoryUtil.memPutShort(ptr + 26, (short) 0);
        }
        pendingSize += bytes;
    }

    private static LabelDraw addLabelDraw(StagedVertexBuffer.Draw draw, PreparedRenderType prepared) {
        if (drawCount == DRAWS.size()) {
            DRAWS.add(new LabelDraw());
        }
        LabelDraw labelDraw = DRAWS.get(drawCount++);
        labelDraw.reset(draw, prepared, ((PosedDrawHolder) draw).ruthenium$quadSorting());
        ((LabelDrawHolder) draw).ruthenium$setLabelDraw(labelDraw);
        return labelDraw;
    }

    /** This frame's label: its pose (as vanilla's glyph renderer holds it) and light, and its region's texels */
    private static int addLabel(Mesh mesh, Matrix4f pose, int light) {
        int n = labelCount;
        if (n == labelMeshes.length) {
            labelMeshes = Arrays.copyOf(labelMeshes, n * 2);
            labelLights = Arrays.copyOf(labelLights, n * 2);
        }
        if (n == LABEL_POSES.size()) {
            LABEL_POSES.add(new Matrix4f());
        }
        LABEL_POSES.get(n).set(pose);
        labelMeshes[n] = mesh;
        labelLights[n] = light;
        labelCount = n + 1;

        ByteBuffer bytes = poseBytes;
        if (bytes == null) {
            bytes = poseBytes = MemoryUtil.memAlloc(MAX_REGIONS * REGION_SIZE);
        }
        long ptr = MemoryUtil.memAddress0(bytes) + (long) mesh.region * REGION_SIZE;
        putRow(ptr, pose.m00(), pose.m10(), pose.m20(), pose.m30());
        putRow(ptr + 16, pose.m01(), pose.m11(), pose.m21(), pose.m31());
        putRow(ptr + 32, pose.m02(), pose.m12(), pose.m22(), pose.m32());
        // The light as the text format's UV2 (two signed shorts)
        putRow(ptr + 48, (short) light, (short) (light >>> 16), 0.0F, 0.0F);
        maxRegion = Math.max(maxRegion, mesh.region);
        return n;
    }

    /** The pose's elements that {@code transformPosition} reads, and its properties (which pick JOML's path) */
    private static void keepPose(Matrix4f pose, float[] out, int at) {
        out[at] = pose.m00();
        out[at + 1] = pose.m01();
        out[at + 2] = pose.m02();
        out[at + 3] = pose.m10();
        out[at + 4] = pose.m11();
        out[at + 5] = pose.m12();
        out[at + 6] = pose.m20();
        out[at + 7] = pose.m21();
        out[at + 8] = pose.m22();
        out[at + 9] = pose.m30();
        out[at + 10] = pose.m31();
        out[at + 11] = pose.m32();
        out[at + 12] = Float.intBitsToFloat(pose.properties());
    }

    private static boolean samePose(Matrix4f pose, float[] kept, int at) {
        return same(pose.m30(), kept[at + 9]) && same(pose.m31(), kept[at + 10]) && same(pose.m32(), kept[at + 11])
                && same(pose.m00(), kept[at]) && same(pose.m01(), kept[at + 1]) && same(pose.m02(), kept[at + 2])
                && same(pose.m10(), kept[at + 3]) && same(pose.m11(), kept[at + 4]) && same(pose.m12(), kept[at + 5])
                && same(pose.m20(), kept[at + 6]) && same(pose.m21(), kept[at + 7]) && same(pose.m22(), kept[at + 8])
                && pose.properties() == Float.floatToRawIntBits(kept[at + 12]);
    }

    private static boolean same(float a, float b) {
        return Float.floatToRawIntBits(a) == Float.floatToRawIntBits(b);
    }

    private static void putRow(long ptr, float x, float y, float z, float w) {
        MemoryUtil.memPutFloat(ptr, x);
        MemoryUtil.memPutFloat(ptr + 4, y);
        MemoryUtil.memPutFloat(ptr + 8, z);
        MemoryUtil.memPutFloat(ptr + 12, w);
    }

    /**
     * Writes a run of a mesh's quads into a vertex builder, as {@code BakedSheetGlyph} writes them (directly into the text
     * formats, as {@code BakedSheetGlyphM} does, else through the vertex consumer's calls)
     */
    private static void write(VertexConsumer builder, Mesh mesh, int firstQuad, int quads, Matrix4fc pose, int light) {
        float[] positions = mesh.positions;
        float[] uvs = mesh.uvs;
        int[] colors = mesh.colors;
        int end = (firstQuad + quads) * 4;
        boolean withLight = true;
        ExtendedVertexBuilder direct = ExtendedVertexBuilder.of(builder, DefaultVertexFormat.POSITION_TEX_LIGHTMAP_COLOR);
        if (direct == null) {
            direct = ExtendedVertexBuilder.of(builder, DefaultVertexFormat.POSITION_TEX_COLOR);
            withLight = false;
        }
        if (direct != null) {
            int vertexSize = withLight ? 28 : 24;
            Vector3f scratch = SCRATCH;
            for (int v = firstQuad * 4; v < end; v += 4) {
                long ptr = direct.reserveVertices(4);
                for (int i = v; i < v + 4; i++, ptr += vertexSize) {
                    pose.transformPosition(positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2], scratch);
                    MemoryUtil.memPutFloat(ptr, scratch.x());
                    MemoryUtil.memPutFloat(ptr + 4, scratch.y());
                    MemoryUtil.memPutFloat(ptr + 8, scratch.z());
                    MemoryUtil.memPutFloat(ptr + 12, uvs[i * 2]);
                    MemoryUtil.memPutFloat(ptr + 16, uvs[i * 2 + 1]);
                    if (withLight) {
                        MemoryUtil.memPutInt(ptr + 20, light);
                        MemoryUtil.memPutInt(ptr + 24, ARGB.toABGR(colors[i]));
                    } else {
                        MemoryUtil.memPutInt(ptr + 20, ARGB.toABGR(colors[i]));
                    }
                }
            }
            return;
        }
        for (int v = firstQuad * 4; v < end; v++) {
            builder.addVertex(pose, positions[v * 3], positions[v * 3 + 1], positions[v * 3 + 2])
                    .setColor(colors[v]).setUv(uvs[v * 2], uvs[v * 2 + 1]).setLight(light);
        }
    }

    /**
     * A vanilla write is about to join a label draw: its labels are written into the draw first, as vanilla would have
     * written them, and the draw is a vanilla draw from then on
     */
    public static void materialize(LabelDraw labelDraw, VertexConsumer builder) {
        StagedVertexBuffer.Draw draw = labelDraw.draw;
        if (draw != null) {
            ((LabelDrawHolder) draw).ruthenium$setLabelDraw(null);
        }
        for (int i = 0; i < labelDraw.uses; i++) {
            int label = labelDraw.useLabel[i];
            write(builder, labelMeshes[label], labelDraw.useFirstQuad[i], labelDraw.useQuads[i], LABEL_POSES.get(label), labelLights[label]);
        }
        labelDraw.uses = 0;
        labelDraw.quads = 0;
    }

    /** Sorts this frame's label draws and uploads the new meshes, the regions' poses and the indices */
    public static void upload() {
        framePoses = null;
        frameIndices = null;
        frameCommands = null;
        frameArena = null;
        COMPILED.clear();
        if (poseRing != null) {
            // Fences the previous frame's use of the current buffers
            poseRing.rotate();
        }
        if (indexRing != null) {
            indexRing.rotate();
        }
        if (commandRing != null) {
            commandRing.rotate();
        }
        GpuBuffer arenaBuffer = arena;
        if (pendingSize > 0 && arenaBuffer != null) {
            ByteBuffer data = MemoryUtil.memByteBuffer(MemoryUtil.memAddress0(pending), pendingSize);
            RenderSystem.getDevice().createCommandEncoder().writeToBuffer(arenaBuffer.slice((long) pendingStart * VERTEX_SIZE, pendingSize), data);
        }
        pendingStart = arenaVertices;
        pendingSize = 0;
        if (drawCount == 0 || arenaBuffer == null) {
            return;
        }

        boolean reuseOrder = Initializer.CONFIG.reuseSortOrder;
        if (reuseOrder) {
            SORT_ORDERS.beginUpload();
            sortOrdersUsed = true;
        } else if (sortOrdersUsed) {
            SORT_ORDERS.clear();
            sortOrdersUsed = false;
        }
        boolean indirect = multiDrawIndirect();
        indexSize = 0;
        commandSize = 0;
        int largestRange = 0;
        for (int d = 0; d < drawCount; d++) {
            LabelDraw draw = DRAWS.get(d);
            int quads = draw.quads;
            draw.indexCount = 0;
            draw.commandCount = 0;
            if (draw.uses == 0 || quads == 0) {
                continue;
            }
            VertexSorting sorting = draw.sorting;
            if (sorting == null && indirect) {
                // Ranges of the arena in write order, drawn with the sequential quad index buffer: consecutive uses of
                // meshes placed one after the other are one range
                largestRange = Math.max(largestRange, writeCommands(draw));
            } else if (sorting == null) {
                writeIndices(draw, null);
            } else {
                if (draw.quadVertices.length < quads) {
                    draw.quadVertices = new int[Math.max(quads, draw.quadVertices.length * 2)];
                }
                int[] order;
                if (draw.sameSortInputs(sorting)) {
                    // Nothing that the centroids depend on moved: last frame's order (and quad vertices) stand
                    order = draw.sortedOrder;
                } else {
                    order = sort(draw, sorting, draw.quadVertices, reuseOrder);
                    draw.keepSortInputs(sorting, order);
                    order = draw.sortedOrder;
                }
                writeIndices(draw, order);
            }
        }
        if ((indexSize == 0 && commandSize == 0) || poseBytes == null || maxRegion < 0) {
            return;
        }
        if (largestRange > 0) {
            // Grown here, outside the render pass, as StagedVertexBuffer.upload does for its draws
            RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS).getBuffer(largestRange * 6);
        }

        if (poseRing == null) {
            poseRing = new MappableRingBuffer(() -> "Ruthenium label poses", GpuBuffer.USAGE_UNIFORM_TEXEL_BUFFER | GpuBuffer.USAGE_MAP_WRITE,
                    MAX_REGIONS * REGION_SIZE);
        }
        GpuBuffer poses = poseRing.currentBuffer();
        int poseSize = (maxRegion + 1) * REGION_SIZE;
        try (GpuBufferSlice.MappedView view = poses.slice(0, poseSize).map(false, true)) {
            MemoryUtil.memCopy(MemoryUtil.memAddress0(poseBytes), MemoryUtil.memAddress(view.data()), poseSize);
        }
        if (indexSize > 0) {
            if (indexRing == null || indexRing.size() < indexSize) {
                if (indexRing != null) {
                    indexRing.close();
                }
                indexRing = new MappableRingBuffer(() -> "Ruthenium label indices", GpuBuffer.USAGE_INDEX | GpuBuffer.USAGE_MAP_WRITE,
                        Math.max(indexSize * 2, 1 << 16));
            }
            GpuBuffer indices = indexRing.currentBuffer();
            try (GpuBufferSlice.MappedView view = indices.slice(0, indexSize).map(false, true)) {
                MemoryUtil.memCopy(MemoryUtil.memAddress0(indexBytes), MemoryUtil.memAddress(view.data()), indexSize);
            }
            frameIndices = indices;
        }
        if (commandSize > 0) {
            if (commandRing == null || commandRing.size() < commandSize) {
                if (commandRing != null) {
                    commandRing.close();
                }
                commandRing = new MappableRingBuffer(() -> "Ruthenium label draw commands",
                        GpuBuffer.USAGE_INDIRECT_PARAMETERS | GpuBuffer.USAGE_MAP_WRITE, Math.max(commandSize * 2, 1 << 14));
            }
            GpuBuffer commands = commandRing.currentBuffer();
            try (GpuBufferSlice.MappedView view = commands.slice(0, commandSize).map(false, true)) {
                MemoryUtil.memCopy(MemoryUtil.memAddress0(commandBytes), MemoryUtil.memAddress(view.data()), commandSize);
            }
            frameCommands = commands;
        }
        framePoses = poses;
        frameArena = arenaBuffer;
    }

    /** Whether unsorted label draws are drawn with indirect multi-draws (as vanilla's terrain decides it) */
    private static boolean multiDrawIndirect() {
        Boolean available = multiDrawIndirect;
        if (available == null) {
            DeviceInfo info = RenderSystem.getDevice().getDeviceInfo();
            available = info.features().drawIndirect() && info.features().multiDrawIndirect() && info.limits().maxDrawIndirectDrawCount() > 1
                    && !info.hintsAndWorkarounds().multiDrawIndirectHasKnownIssues();
            maxIndirectDraws = info.limits().maxDrawIndirectDrawCount();
            multiDrawIndirect = available;
        }
        return available;
    }

    /** An unsorted draw's ranges as indirect commands; returns the largest range's quads */
    private static int writeCommands(LabelDraw draw) {
        int largest = 0;
        int start = -1;
        int end = -1;
        int commands = 0;
        int first = commandSize;
        for (int u = 0; u <= draw.uses; u++) {
            int vertex = -1;
            int vertices = 0;
            if (u < draw.uses) {
                vertex = labelMeshes[draw.useLabel[u]].baseVertex + draw.useFirstQuad[u] * 4;
                vertices = draw.useQuads[u] * 4;
                if (vertex == end) {
                    end += vertices;
                    continue;
                }
            }
            if (start != -1) {
                int quads = (end - start) / 4;
                if (commandBytes.capacity() < commandSize + 20) {
                    commandBytes = MemoryUtil.memRealloc(commandBytes, commandBytes.capacity() * 2);
                }
                long ptr = MemoryUtil.memAddress0(commandBytes) + commandSize;
                // VkDrawIndexedIndirectCommand: index count, instance count, first index, vertex offset, first instance
                MemoryUtil.memPutInt(ptr, quads * 6);
                MemoryUtil.memPutInt(ptr + 4, 1);
                MemoryUtil.memPutInt(ptr + 8, 0);
                MemoryUtil.memPutInt(ptr + 12, start);
                MemoryUtil.memPutInt(ptr + 16, 0);
                commandSize += 20;
                commands++;
                largest = Math.max(largest, quads);
            }
            start = vertex;
            end = vertex + vertices;
        }
        draw.firstCommand = first;
        draw.commandCount = commands;
        return largest;
    }

    /**
     * A draw's quads as indices into the arena, in this order (the sorted quads) or in write order: 16 bit relative to the
     * draw's first vertex when its quads span fewer than 65536 vertices
     */
    private static void writeIndices(LabelDraw draw, int @Nullable [] order) {
        int quads = draw.quads;
        int min = Integer.MAX_VALUE;
        int max = 0;
        for (int u = 0; u < draw.uses; u++) {
            int vertex = labelMeshes[draw.useLabel[u]].baseVertex + draw.useFirstQuad[u] * 4;
            min = Math.min(min, vertex);
            max = Math.max(max, vertex + draw.useQuads[u] * 4);
        }
        boolean small = max - min <= 65536;
        int indexBytesPerQuad = small ? 12 : 24;
        int bytes = quads * indexBytesPerQuad;
        if (indexBytes.capacity() < indexSize + bytes + 4) {
            indexBytes = MemoryUtil.memRealloc(indexBytes, Math.max(indexBytes.capacity() * 2, indexSize + bytes + 4));
        }
        long ptr = MemoryUtil.memAddress0(indexBytes) + indexSize;
        int base = small ? min : 0;
        if (order == null) {
            // The draw's quads in write order, as the sequential index buffer lists them
            for (int u = 0; u < draw.uses; u++) {
                int vertex = labelMeshes[draw.useLabel[u]].baseVertex + draw.useFirstQuad[u] * 4 - base;
                int end = vertex + draw.useQuads[u] * 4;
                for (; vertex < end; vertex += 4, ptr += indexBytesPerQuad) {
                    putQuad(ptr, vertex, small);
                }
            }
        } else {
            int[] vertices = draw.quadVertices;
            for (int q : order) {
                putQuad(ptr, vertices[q] - base, small);
                ptr += indexBytesPerQuad;
            }
        }
        draw.indexType = small ? IndexType.SHORT : IndexType.INT;
        draw.baseVertex = base;
        draw.firstIndex = indexSize / (small ? 2 : 4);
        draw.indexCount = quads * 6;
        // Every draw's indices start at a multiple of 4 bytes
        indexSize = (indexSize + bytes + 3) & ~3;
    }

    private static void putQuad(long ptr, int base, boolean small) {
        if (small) {
            MemoryUtil.memPutShort(ptr, (short) base);
            MemoryUtil.memPutShort(ptr + 2, (short) (base + 1));
            MemoryUtil.memPutShort(ptr + 4, (short) (base + 2));
            MemoryUtil.memPutShort(ptr + 6, (short) (base + 2));
            MemoryUtil.memPutShort(ptr + 8, (short) (base + 3));
            MemoryUtil.memPutShort(ptr + 10, (short) base);
        } else {
            MemoryUtil.memPutInt(ptr, base);
            MemoryUtil.memPutInt(ptr + 4, base + 1);
            MemoryUtil.memPutInt(ptr + 8, base + 2);
            MemoryUtil.memPutInt(ptr + 12, base + 2);
            MemoryUtil.memPutInt(ptr + 16, base + 3);
            MemoryUtil.memPutInt(ptr + 20, base);
        }
    }

    /**
     * The draw's quad order: what vanilla's upload sorts from the centroids it decodes from the written vertices (the
     * same floats: vertices 0 and 2 through the label's pose, then their mean)
     */
    private static int[] sort(LabelDraw draw, VertexSorting sorting, int[] vertices, boolean reuseOrder) {
        int quads = draw.quads;
        CompactVectorArray centroids = draw.centroids;
        if (centroids == null || centroids.size() != quads) {
            centroids = draw.centroids = new CompactVectorArray(quads);
        }
        Vector3f scratch = SCRATCH;
        int q = 0;
        for (int u = 0; u < draw.uses; u++) {
            int label = draw.useLabel[u];
            Mesh mesh = labelMeshes[label];
            Matrix4f pose = LABEL_POSES.get(label);
            float[] positions = mesh.positions;
            int first = draw.useFirstQuad[u];
            int end = first + draw.useQuads[u];
            int vertex = mesh.baseVertex + first * 4;
            if (INLINE_TRANSFORM && (pose.properties() & (Matrix4fc.PROPERTY_IDENTITY | Matrix4fc.PROPERTY_TRANSLATION)) == 0) {
                // JOML's generic transformPosition (Vector3f.mulPositionGeneric), checked at startup to give the same bits
                float m00 = pose.m00(), m01 = pose.m01(), m02 = pose.m02();
                float m10 = pose.m10(), m11 = pose.m11(), m12 = pose.m12();
                float m20 = pose.m20(), m21 = pose.m21(), m22 = pose.m22();
                float m30 = pose.m30(), m31 = pose.m31(), m32 = pose.m32();
                for (int quad = first; quad < end; quad++, q++, vertex += 4) {
                    int a = quad * 12;
                    int b = a + 6;
                    float ax = positions[a], ay = positions[a + 1], az = positions[a + 2];
                    float bx = positions[b], by = positions[b + 1], bz = positions[b + 2];
                    float x0 = m00 * ax + (m10 * ay + (m20 * az + m30));
                    float y0 = m01 * ax + (m11 * ay + (m21 * az + m31));
                    float z0 = m02 * ax + (m12 * ay + (m22 * az + m32));
                    float x1 = m00 * bx + (m10 * by + (m20 * bz + m30));
                    float y1 = m01 * bx + (m11 * by + (m21 * bz + m31));
                    float z1 = m02 * bx + (m12 * by + (m22 * bz + m32));
                    centroids.set(q, (x0 + x1) / 2.0F, (y0 + y1) / 2.0F, (z0 + z1) / 2.0F);
                    vertices[q] = vertex;
                }
            } else {
                for (int quad = first; quad < end; quad++, q++, vertex += 4) {
                    int a = quad * 12;
                    int b = a + 6;
                    pose.transformPosition(positions[a], positions[a + 1], positions[a + 2], scratch);
                    float x0 = scratch.x(), y0 = scratch.y(), z0 = scratch.z();
                    pose.transformPosition(positions[b], positions[b + 1], positions[b + 2], scratch);
                    centroids.set(q, (x0 + scratch.x()) / 2.0F, (y0 + scratch.y()) / 2.0F, (z0 + scratch.z()) / 2.0F);
                    vertices[q] = vertex;
                }
            }
        }
        if (reuseOrder && sorting instanceof DistanceSorting.Sorter sorter) {
            return SORT_ORDERS.sort(sorter, centroids);
        }
        return sorting.sort(centroids);
    }

    /** Whether the inlined transform gives JOML's bits (no fused multiply-add configured, the generic path's order) */
    private static boolean checkInlineTransform() {
        java.util.Random random = new java.util.Random(23);
        Matrix4f m = new Matrix4f();
        Vector3f out = new Vector3f();
        for (int i = 0; i < 4096; i++) {
            m.set(random.nextFloat() * 2 - 1, random.nextFloat() * 2 - 1, random.nextFloat() * 2 - 1, 0.0F,
                    random.nextFloat() * 2 - 1, random.nextFloat() * 2 - 1, random.nextFloat() * 2 - 1, 0.0F,
                    random.nextFloat() * 2 - 1, random.nextFloat() * 2 - 1, random.nextFloat() * 2 - 1, 0.0F,
                    random.nextFloat() * 64 - 32, random.nextFloat() * 64 - 32, random.nextFloat() * 64 - 32, 1.0F);
            float x = random.nextFloat() * 200 - 100, y = random.nextFloat() * 200 - 100, z = random.nextFloat() - 0.5F;
            m.transformPosition(x, y, z, out);
            float ix = m.m00() * x + (m.m10() * y + (m.m20() * z + m.m30()));
            float iy = m.m01() * x + (m.m11() * y + (m.m21() * z + m.m31()));
            float iz = m.m02() * x + (m.m12() * y + (m.m22() * z + m.m32()));
            if (Float.floatToRawIntBits(ix) != Float.floatToRawIntBits(out.x) || Float.floatToRawIntBits(iy) != Float.floatToRawIntBits(out.y)
                    || Float.floatToRawIntBits(iz) != Float.floatToRawIntBits(out.z)) {
                return false;
            }
        }
        return true;
    }

    /** Draws a label draw in its group draw's place ({@code PreparedRenderType.draw} with the arena and the indices) */
    public static void draw(RenderPass renderPass, LabelDraw draw) {
        GpuBuffer poses = framePoses;
        GpuBuffer arenaBuffer = frameArena;
        PreparedRenderType prepared = draw.prepared;
        if (draw.uses == 0 || (draw.indexCount == 0 && draw.commandCount == 0) || poses == null || arenaBuffer == null || prepared == null) {
            return;
        }
        GpuBuffer indices = frameIndices;
        GpuBuffer commands = frameCommands;
        if (draw.commandCount > 0 ? commands == null : indices == null) {
            return;
        }
        CompiledRenderPipeline pipeline = COMPILED.get(prepared.pipeline());
        if (pipeline == null) {
            RenderPipeline copy = COPIES.get(prepared.pipeline());
            pipeline = copy != null ? RenderSystem.getCompiledPipelineNullable(copy) : null;
            if (pipeline == null) {
                return;
            }
            COMPILED.put(prepared.pipeline(), pipeline);
        }
        renderPass.setPipeline(pipeline);
        ScissorState scissor = prepared.scissorState();
        if (scissor.enabled()) {
            renderPass.enableScissor(scissor.x(), scissor.y(), scissor.width(), scissor.height());
        }
        RenderSystem.bindDefaultUniforms(renderPass);
        renderPass.setUniform("DynamicTransforms", prepared.dynamicTransforms());
        renderPass.setVertexBuffer(0, arenaBuffer.slice());
        for (PreparedRenderType.Texture texture : prepared.textures()) {
            renderPass.setUniform(texture.name(), texture.textureView(), texture.sampler());
        }
        renderPass.setUniform("LabelPoses", poses);
        if (draw.commandCount > 0) {
            RenderSystem.AutoStorageIndexBuffer sequential = RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS);
            renderPass.setIndexBuffer(sequential.getBuffer(), sequential.type());
            int max = Math.max(1, maxIndirectDraws);
            for (int done = 0; done < draw.commandCount; done += max) {
                int count = Math.min(max, draw.commandCount - done);
                renderPass.drawIndexedIndirect(commands.slice(draw.firstCommand + done * 20L, count * 20L), count);
            }
        } else {
            renderPass.setIndexBuffer(indices, draw.indexType);
            renderPass.drawIndexed(draw.indexCount, 1, draw.firstIndex, draw.baseVertex, 0);
        }
    }
}
