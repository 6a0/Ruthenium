package net.ruthenium.render.entity;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.systems.ScissorState;
import com.mojang.blaze3d.vertex.CompactVectorArray;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.blaze3d.vertex.VertexSorting;
import com.mojang.renderpearl.api.GpuFormat;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.pipeline.BindGroupLayout;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.CompiledRenderPipeline;
import com.mojang.renderpearl.api.pipeline.IndexType;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.renderpearl.api.pipeline.UniformType;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.minecraft.client.Minecraft;
import net.minecraft.client.model.Model;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.StagedVertexBuffer;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.resources.Identifier;
import net.minecraft.util.ARGB;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.PoseLayoutHolder;
import net.ruthenium.interfaces.PosedDrawGroup;
import net.ruthenium.interfaces.PosedDrawHolder;
import net.ruthenium.interfaces.PosedPart;
import net.ruthenium.interfaces.TransparentCube;
import net.ruthenium.mixin.render.entity.model.PoseAccessor;
import net.ruthenium.render.vertex.DistanceSorting;
import net.ruthenium.render.vertex.SortOrderCache;
import org.joml.Matrix3f;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.joml.Vector3fc;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Optional;
import java.util.function.Function;

/**
 * Draws player bodies from a part-space mesh and per-frame part poses instead of writing their vertices every frame
 * ({@code playerPoseDraws}).
 * <p>
 * A player body ({@link PlayerModel}, {@code entity_translucent}) is a model submit of the translucent models phase:
 * a strictly ordered group whose draws are sorted per draw on upload, one draw per run of consecutive submits with
 * the same render type (in practice one per player, two players with the same skin next to each other share one).
 * Vanilla writes all 72 faces into the draw, decodes their centroids on upload and sorts them back to front. Here the
 * draw keeps its place in the group, but its geometry is a static mesh of the model's cubes in part space (per
 * {@link PlayerModel} instance: wide and slim models are different instances), with the member and part of each vertex;
 * each frame only the parts' poses (pose and normal matrix rows), light, overlay and color are written, into a uniform
 * buffer slice per draw that the vertex shader indexes ({@code player_pose.vsh}).
 * <p>
 * Exactness: the parts are posed by vanilla's {@code setupAnim} and part tree; {@code ModelPart.compile} hands each
 * written part's pose to {@link #capturePart} instead of writing its cubes. Positions are computed on the GPU in JOML's
 * operation order ({@code precise}), as {@link InstancedParts} does, so they are bit-identical. The quad order is
 * vanilla's: each face's centroid is computed on the CPU from the same corner transforms (JOML's
 * {@code transformPosition}) and the same mean of vertices 0 and 2 that {@code MeshData.decodeQuadCentroids} reads, the
 * faces are numbered in vanilla's write order (part tree order, cube order, polygon order, minus the faces
 * {@link TransparentFaces} skips), and the draw's own sorting sorts them ({@link SortOrderCache} start orders when
 * {@code reuseSortOrder} is on), so the index order is the one vanilla's upload writes. Visibility ({@code visible},
 * {@code skipDraw}, the outer layer toggles, spectators) only changes which faces are listed. Normals are transformed and
 * normalized in full precision on the GPU instead of being quantized to bytes (as with instanced parts).
 * <p>
 * Only for submits of the exact {@link PlayerModel} class whose written parts are the model's own (any other part, a
 * part written twice or any vertex written to the sink fails the capture), of vanilla's {@code entity_translucent}
 * pipeline, without UV mapping or decal, in a strictly ordered group of the world's features without improved
 * transparency; anything else takes vanilla's path. If a submit that takes vanilla's path would join a draw of posed
 * members (same render type, consecutive), those members are written first as vanilla would have ({@link #materialize}).
 * Render thread only.
 */
public final class PlayerPoseDraws {
    /** Members per draw (consecutive bodies of one skin) and part slots per member */
    public static final int MAX_MEMBERS = 8;
    public static final int MAX_SLOTS = 12;
    /** Uniform rows per slot: three pose rows and three normal rows */
    private static final int SLOT_ROWS = 6;
    private static final int HEADER_SIZE = 32;
    private static final int MEMBER_SIZE = HEADER_SIZE + MAX_SLOTS * SLOT_ROWS * 16;
    /** The size of the uniform block the shader declares */
    private static final int BLOCK_SIZE = MAX_MEMBERS * MEMBER_SIZE;

    public static final VertexFormat MESH_FORMAT = VertexFormat.builder(0)
            .addAttribute("Position", GpuFormat.RGB32_FLOAT)
            .addAttribute("UV0", GpuFormat.RG32_FLOAT)
            .addAttribute("Normal", GpuFormat.RGB32_FLOAT)
            .addAttribute("PartIndex", GpuFormat.RG16_SINT)
            .build();
    private static final int VERTEX_SIZE = MESH_FORMAT.getVertexSize();
    private static final BindGroupLayout POSES = BindGroupLayout.builder().withUniform("PlayerPoses", UniformType.UNIFORM_BUFFER).build();

    /** Vanilla's entity_translucent with the posed vertex shader */
    public static final RenderPipeline PIPELINE = copy(RenderPipelines.ENTITY_TRANSLUCENT, "entity_translucent");
    public static final List<RenderPipeline> PIPELINES = List.of(PIPELINE);

    private static final List<Layout> LAYOUTS = new ArrayList<>();
    /** Layouts kept at most; models created over and over (by mods) take vanilla's path beyond this */
    private static final int MAX_LAYOUTS = 64;
    /** Consecutive failed captures after which a model takes vanilla's path until the next clear */
    private static final int MAX_FAILURES = 64;
    private static final Object NO_LAYOUT = new Object();
    private static int generation = 1;
    private static boolean clearPending;
    private static @Nullable Boolean pipelineAvailable;

    /** Whether this frame's world features may take posed draws */
    private static boolean frameActive;
    private static int uniformAlignment = 256;
    private static @Nullable CompiledRenderPipeline framePipeline;
    /** The group being prepared ({@code RenderTypeFeatureRenderer$Group} construction) */
    private static @Nullable PosedDrawGroup preparingGroup;

    private static final List<PoseDraw> DRAWS = new ArrayList<>();
    private static int drawCount;
    /** This frame's member blocks (the shader's uniform layout) and indices */
    private static ByteBuffer poseBytes = MemoryUtil.memAlloc(1 << 16);
    private static int poseSize;
    private static ByteBuffer indexBytes = MemoryUtil.memAlloc(1 << 14);
    private static int indexSize;
    private static @Nullable MappableRingBuffer poseRing;
    private static @Nullable MappableRingBuffer indexRing;
    private static @Nullable GpuBuffer framePoses;
    private static @Nullable GpuBuffer frameIndices;
    private static final SortOrderCache SORT_ORDERS = new SortOrderCache();
    private static boolean sortOrdersUsed;
    private static CompactVectorArray[] centroidArrays = new CompactVectorArray[0];

    /** Capture of one member: its layout, draw, member index and uniform block */
    private static boolean capturing;
    private static boolean captureFailed;
    private static @Nullable Layout captureLayout;
    private static @Nullable PoseDraw captureDraw;
    private static int captureVertexBase;
    private static int captureSeen;
    private static long captureRows;
    private static TransparentFaces.@Nullable TextureAlpha captureAlpha;
    private static final Vector3f SCRATCH = new Vector3f();
    private static float[] transformed = new float[24 * 3];
    private static final CaptureSink SINK = new CaptureSink();
    private static final PoseStack MATERIALIZE_POSE = new PoseStack();

    private PlayerPoseDraws() {
    }

    private static RenderPipeline copy(RenderPipeline vanilla, String name) {
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
                .withLocation(Identifier.fromNamespaceAndPath("ruthenium", "pipeline/player_pose_" + name))
                .withVertexShader(Identifier.fromNamespaceAndPath("ruthenium", "core/player_pose"))
                .withVertexBinding(0, MESH_FORMAT)
                .withBindGroupLayout(POSES)
                .withShaderDefine("POSE_MEMBERS", MAX_MEMBERS)
                .withShaderDefine("POSE_SLOTS", MAX_SLOTS)
                .build();
    }

    /**
     * A model's cubes in part space: one slot per part with cubes, the cube faces numbered in slot, cube and polygon
     * order, and a mesh of {@link #MAX_MEMBERS} copies (one per member of a draw)
     */
    static final class Layout {
        final int generation;
        final int slotCount;
        final List<?>[] cubes;
        final CubeInfo[][] cubeInfos;
        /** Vertices of one copy */
        final int vertexCount;
        @Nullable GpuBuffer mesh;
        @Nullable GpuBufferSlice meshSlice;
        int failures;

        Layout(int generation, int slotCount, List<?>[] cubes, CubeInfo[][] cubeInfos, int vertexCount) {
            this.generation = generation;
            this.slotCount = slotCount;
            this.cubes = cubes;
            this.cubeInfos = cubeInfos;
            this.vertexCount = vertexCount;
        }
    }

    /** A cube's distinct corners and the corners of each polygon's vertices 0 and 2 (offsets into the corners) */
    static final class CubeInfo {
        final TransparentCube cube;
        final int quadBase;
        final int polygons;
        final int allPolygons;
        final float[] corners;
        final int[] first;
        final int[] third;

        CubeInfo(TransparentCube cube, int quadBase, int polygons, float[] corners, int[] first, int[] third) {
            this.cube = cube;
            this.quadBase = quadBase;
            this.polygons = polygons;
            this.allPolygons = (1 << polygons) - 1;
            this.corners = corners;
            this.first = first;
            this.third = third;
        }
    }

    /** A posed draw: the members (consecutive submits) of one group draw and their faces */
    public static final class PoseDraw {
        @Nullable Layout layout;
        @Nullable PreparedRenderType prepared;
        @Nullable VertexSorting sorting;
        StagedVertexBuffer.@Nullable Draw draw;
        final ModelFeatureRenderer.Submit<?>[] submits = new ModelFeatureRenderer.Submit<?>[MAX_MEMBERS];
        int members;
        int uboOffset;
        int quadCount;
        float[] centroids = new float[3 * 72];
        int[] quadVertices = new int[72];
        int firstIndex;
        int indexCount;

        void reset(Layout layout, int uboOffset) {
            this.layout = layout;
            this.prepared = null;
            this.sorting = null;
            this.draw = null;
            this.members = 0;
            this.uboOffset = uboOffset;
            this.quadCount = 0;
            this.indexCount = 0;
        }

        void clear() {
            this.layout = null;
            this.prepared = null;
            this.sorting = null;
            this.draw = null;
            Arrays.fill(this.submits, null);
            this.members = 0;
        }

        void addQuad(int vertex, float x, float y, float z) {
            int n = this.quadCount;
            if (n == this.quadVertices.length) {
                this.quadVertices = Arrays.copyOf(this.quadVertices, n * 2);
                this.centroids = Arrays.copyOf(this.centroids, n * 6);
            }
            this.quadVertices[n] = vertex;
            this.centroids[n * 3] = x;
            this.centroids[n * 3 + 1] = y;
            this.centroids[n * 3 + 2] = z;
            this.quadCount = n + 1;
        }
    }

    /** Takes the vertices a mod writes while the parts are captured: any write fails the capture */
    private static final class CaptureSink implements VertexConsumer {
        boolean written;

        @Override
        public VertexConsumer addVertex(float x, float y, float z) {
            this.written = true;
            return this;
        }

        @Override
        public VertexConsumer setColor(int r, int g, int b, int a) {
            this.written = true;
            return this;
        }

        @Override
        public VertexConsumer setColor(int color) {
            this.written = true;
            return this;
        }

        @Override
        public VertexConsumer setUv(float u, float v) {
            this.written = true;
            return this;
        }

        @Override
        public VertexConsumer setUv1(int u, int v) {
            this.written = true;
            return this;
        }

        @Override
        public VertexConsumer setUv2(int u, int v) {
            this.written = true;
            return this;
        }

        @Override
        public VertexConsumer setUv3(float u, float v) {
            this.written = true;
            return this;
        }

        @Override
        public VertexConsumer setNormal(float x, float y, float z) {
            this.written = true;
            return this;
        }

        @Override
        public VertexConsumer setLineWidth(float width) {
            this.written = true;
            return this;
        }
    }

    /** Drops every layout at the start of the next frame (resource reload, switch toggled) */
    public static void clear() {
        clearPending = true;
    }

    /** Called before the world's features are prepared */
    public static void beginFrame() {
        if (clearPending) {
            clearPending = false;
            pipelineAvailable = null;
            for (Layout layout : LAYOUTS) {
                if (layout.mesh != null) {
                    layout.mesh.close();
                }
            }
            LAYOUTS.clear();
            generation++;
            SORT_ORDERS.clear();
        }
        for (int i = 0; i < drawCount; i++) {
            DRAWS.get(i).clear();
        }
        drawCount = 0;
        poseSize = 0;
        indexSize = 0;
        frameActive = Initializer.CONFIG.playerPoseDraws && !Minecraft.getInstance().gameRenderer.useImprovedTransparency() && isPipelineAvailable();
        uniformAlignment = RenderSystem.getDevice().getDeviceInfo().limits().minUniformOffsetAlignment();
    }

    /** A feature renderer group starts preparing */
    public static void beginGroup(PosedDrawGroup group) {
        preparingGroup = group;
    }

    /** Whether this model submit may be drawn from poses ({@link #prepare}): cheap checks only */
    public static boolean accepts(ModelFeatureRenderer.Submit<?> submit) {
        // The pipeline first: it rules out almost every other model submit
        RenderType renderType = submit.renderType();
        if (renderType.pipeline() != RenderPipelines.ENTITY_TRANSLUCENT || !frameActive || submit.model().getClass() != PlayerModel.class
                || submit.uvMapping() != null || submit.sheetedDecalPose() != null || !ModelCulling.isWorldPass()
                || !renderType.sortOnUpload() || !renderType.canConsolidateConsecutiveGeometry()) {
            return false;
        }
        PosedDrawGroup group = preparingGroup;
        return group != null && !group.ruthenium$canReorder() && Initializer.CONFIG.playerPoseDraws;
    }

    /**
     * Prepares an accepted submit: posed into a posed draw, or written as vanilla does if the capture fails.
     *
     * @param poseStack the feature renderer's pose stack, holding the submit's pose
     * @param builders  the group's vertex builder of a render type ({@code getVertexBuilder})
     */
    @SuppressWarnings({"unchecked", "rawtypes"})
    public static void prepare(ModelFeatureRenderer.Submit<?> submit, PoseStack poseStack, Function<RenderType, VertexConsumer> builders) {
        Model model = submit.model();
        Object state = submit.state();
        ModelPoseReplay.Record replay = ModelPoseReplay.begin(model, state, poseStack.last());
        if (replay != null) {
            replay.replay(builders.apply(submit.renderType()), submit.lightCoords(), submit.overlayCoords(), submit.tintedColor());
            return;
        }

        model.setupAnim(state);
        Layout layout = layoutOf((PlayerModel) model);
        boolean posed = layout != null && capture(submit, model, layout, poseStack);
        if (layout != null && !posed && captureFailed && ++layout.failures >= MAX_FAILURES) {
            // The model's parts or writes are not the layout's (a mod): vanilla's path from now on
            ((PoseLayoutHolder) (Object) model).ruthenium$setPoseLayout(NO_LAYOUT);
        }
        ModelPoseReplay.end();
        if (!posed) {
            VertexConsumer buffer = builders.apply(submit.renderType());
            // Writing a joined draw's posed members first may have posed the model for another state
            model.setupAnim(state);
            model.renderToBuffer(poseStack, buffer, submit.lightCoords(), submit.overlayCoords(), submit.tintedColor());
        }
    }

    @SuppressWarnings({"rawtypes"})
    private static boolean capture(ModelFeatureRenderer.Submit<?> submit, Model model, Layout layout, PoseStack poseStack) {
        captureFailed = false;
        PosedDrawGroup group = preparingGroup;
        if (group == null) {
            return false;
        }
        RenderType renderType = submit.renderType();
        StagedVertexBuffer.Draw joining = group.ruthenium$joiningDraw(renderType);
        PoseDraw target;
        int member;
        if (joining != null) {
            // The submit joins the group's last draw: posed if that draw is, with the same layout and room for it
            target = ((PosedDrawHolder) joining).ruthenium$poseDraw();
            if (target == null || target.layout != layout || target.members == MAX_MEMBERS) {
                return false;
            }
            member = target.members;
        } else {
            if (drawCount == DRAWS.size()) {
                DRAWS.add(new PoseDraw());
            }
            target = DRAWS.get(drawCount);
            int alignment = uniformAlignment;
            target.reset(layout, (poseSize + alignment - 1) / alignment * alignment);
            member = 0;
        }

        int offset = target.uboOffset + member * MEMBER_SIZE;
        if (poseBytes.capacity() < offset + MEMBER_SIZE) {
            poseBytes = MemoryUtil.memRealloc(poseBytes, Math.max(poseBytes.capacity() * 2, offset + MEMBER_SIZE));
        }
        long ptr = MemoryUtil.memAddress0(poseBytes) + offset;
        int light = submit.lightCoords();
        int overlay = submit.overlayCoords();
        int color = submit.tintedColor();
        // The light and overlay as the entity format's UV2 and UV1 (two signed shorts each), the color as its RGBA8 normalized
        MemoryUtil.memPutInt(ptr, (short) light);
        MemoryUtil.memPutInt(ptr + 4, (short) (light >>> 16));
        MemoryUtil.memPutInt(ptr + 8, (short) overlay);
        MemoryUtil.memPutInt(ptr + 12, (short) (overlay >>> 16));
        MemoryUtil.memPutFloat(ptr + 16, ARGB.red(color) / 255.0F);
        MemoryUtil.memPutFloat(ptr + 20, ARGB.green(color) / 255.0F);
        MemoryUtil.memPutFloat(ptr + 24, ARGB.blue(color) / 255.0F);
        MemoryUtil.memPutFloat(ptr + 28, ARGB.alpha(color) / 255.0F);

        int quadStart = target.quadCount;
        captureLayout = layout;
        captureDraw = target;
        captureVertexBase = member * layout.vertexCount;
        captureSeen = 0;
        captureRows = ptr + HEADER_SIZE;
        captureAlpha = ModelCulling.textureAlpha();
        captureFailed = false;
        SINK.written = false;
        capturing = true;
        try {
            model.renderToBuffer(poseStack, SINK, light, overlay, color);
        } finally {
            capturing = false;
            captureLayout = null;
            captureDraw = null;
            captureAlpha = null;
        }
        captureFailed |= SINK.written;
        if (captureFailed) {
            target.quadCount = quadStart;
            if (joining == null) {
                target.clear();
            }
            return false;
        }

        if (joining == null) {
            StagedVertexBuffer.Draw draw = group.ruthenium$addDraw(renderType);
            PosedDrawHolder holder = (PosedDrawHolder) draw;
            target.draw = draw;
            target.prepared = group.ruthenium$lastPreparedType();
            target.sorting = holder.ruthenium$quadSorting();
            holder.ruthenium$setPoseDraw(target);
            drawCount++;
        }
        target.submits[member] = submit;
        target.members = member + 1;
        poseSize = offset + MEMBER_SIZE;
        layout.failures = 0;
        return true;
    }

    public static boolean isCapturing() {
        return capturing;
    }

    /** {@code ModelPart.compile} while a member is captured: the part's pose and the centroids of its faces */
    public static void capturePart(ModelPart part, List<ModelPart.Cube> cubes, PoseStack.Pose pose) {
        if (captureFailed) {
            return;
        }
        Layout layout = captureLayout;
        PosedPart posed = (PosedPart) (Object) part;
        int slot = posed.ruthenium$poseSlot();
        if (layout == null || posed.ruthenium$poseLayout() != layout || layout.cubes[slot] != cubes || (captureSeen & 1 << slot) != 0) {
            captureFailed = true;
            return;
        }
        captureSeen |= 1 << slot;

        long ptr = captureRows + (long) slot * SLOT_ROWS * 16;
        Matrix4f m = pose.pose();
        putRow(ptr, m.m00(), m.m10(), m.m20(), m.m30());
        putRow(ptr + 16, m.m01(), m.m11(), m.m21(), m.m31());
        putRow(ptr + 32, m.m02(), m.m12(), m.m22(), m.m32());
        // Vanilla transforms normals with the normal matrix, n' = N n, normalized unless the pose's normals are trusted
        Matrix3f n = pose.normal();
        putRow(ptr + 48, n.m00(), n.m10(), n.m20(), ((PoseAccessor) (Object) pose).ruthenium$trustedNormals() ? 0.0F : 1.0F);
        putRow(ptr + 64, n.m01(), n.m11(), n.m21(), 0.0F);
        putRow(ptr + 80, n.m02(), n.m12(), n.m22(), 0.0F);

        PoseDraw draw = captureDraw;
        TransparentFaces.TextureAlpha alpha = captureAlpha;
        int vertexBase = captureVertexBase;
        Vector3f scratch = SCRATCH;
        for (CubeInfo cube : layout.cubeInfos[slot]) {
            // The faces ModelPartCubeM leaves out
            int skipped = 0;
            if (alpha != null) {
                skipped = cube.cube.ruthenium$transparentPolygons(alpha);
                if (skipped == cube.allPolygons) {
                    continue;
                }
            }
            float[] corners = cube.corners;
            float[] t = transformed;
            if (t.length < corners.length) {
                t = transformed = new float[corners.length];
            }
            for (int i = 0; i < corners.length; i += 3) {
                // The vertex write's transform (Cube.compile)
                m.transformPosition(corners[i], corners[i + 1], corners[i + 2], scratch);
                t[i] = scratch.x();
                t[i + 1] = scratch.y();
                t[i + 2] = scratch.z();
            }
            int[] first = cube.first;
            int[] third = cube.third;
            int quadVertex = vertexBase + cube.quadBase * 4;
            for (int p = 0; p < cube.polygons; p++, quadVertex += 4) {
                if ((skipped & 1 << p) != 0) {
                    continue;
                }
                int a = first[p];
                int b = third[p];
                // MeshData.decodeQuadCentroids: the mean of vertices 0 and 2
                draw.addQuad(quadVertex, (t[a] + t[b]) / 2.0F, (t[a + 1] + t[b + 1]) / 2.0F, (t[a + 2] + t[b + 2]) / 2.0F);
            }
        }
    }

    private static void putRow(long ptr, float x, float y, float z, float w) {
        MemoryUtil.memPutFloat(ptr, x);
        MemoryUtil.memPutFloat(ptr + 4, y);
        MemoryUtil.memPutFloat(ptr + 8, z);
        MemoryUtil.memPutFloat(ptr + 12, w);
    }

    /**
     * A vanilla write is about to join a posed draw: its members are written into the draw first, as vanilla would
     * have written them, and the draw is a vanilla draw from then on.
     */
    @SuppressWarnings({"unchecked", "rawtypes"})
    public static void materialize(PoseDraw draw, VertexConsumer builder) {
        StagedVertexBuffer.Draw vanilla = draw.draw;
        if (vanilla != null) {
            ((PosedDrawHolder) vanilla).ruthenium$setPoseDraw(null);
        }
        for (int i = 0; i < draw.members; i++) {
            ModelFeatureRenderer.Submit submit = draw.submits[i];
            Model model = submit.model();
            ModelCulling.beginModel(submit.renderType());
            MATERIALIZE_POSE.last().set(submit.pose());
            model.setupAnim(submit.state());
            model.renderToBuffer(MATERIALIZE_POSE, builder, submit.lightCoords(), submit.overlayCoords(), submit.tintedColor());
        }
        Arrays.fill(draw.submits, null);
        draw.members = 0;
        draw.quadCount = 0;
    }

    private static @Nullable Layout layoutOf(PlayerModel model) {
        PoseLayoutHolder holder = (PoseLayoutHolder) (Object) model;
        Object kept = holder.ruthenium$poseLayout();
        if (kept instanceof Layout layout && layout.generation == generation) {
            return layout;
        }
        if (kept == NO_LAYOUT) {
            return null;
        }
        Layout layout = LAYOUTS.size() < MAX_LAYOUTS ? buildLayout(model) : null;
        holder.ruthenium$setPoseLayout(layout != null ? layout : NO_LAYOUT);
        return layout;
    }

    private static @Nullable Layout buildLayout(PlayerModel model) {
        List<ModelPart> parts = new ArrayList<>();
        List<List<ModelPart.Cube>> cubeLists = new ArrayList<>();
        for (ModelPart part : model.allParts()) {
            List<ModelPart.Cube> cubes = ((PosedPart) (Object) part).ruthenium$cubes();
            if (!cubes.isEmpty()) {
                parts.add(part);
                cubeLists.add(cubes);
            }
        }
        if (parts.isEmpty() || parts.size() > MAX_SLOTS) {
            return null;
        }

        int slotCount = parts.size();
        CubeInfo[][] infos = new CubeInfo[slotCount][];
        int quads = 0;
        for (int s = 0; s < slotCount; s++) {
            List<ModelPart.Cube> cubes = cubeLists.get(s);
            infos[s] = new CubeInfo[cubes.size()];
            for (int c = 0; c < cubes.size(); c++) {
                ModelPart.Cube cube = cubes.get(c);
                ModelPart.Polygon[] polygons = cube.polygons;
                if (polygons.length > 31) {
                    return null;
                }
                // Distinct corners by bits, so that each is the vertex position vanilla transforms
                float[] corners = new float[polygons.length * 4 * 3];
                int count = 0;
                int[] first = new int[polygons.length];
                int[] third = new int[polygons.length];
                for (int p = 0; p < polygons.length; p++) {
                    ModelPart.Vertex[] vertices = polygons[p].vertices();
                    if (vertices.length != 4) {
                        return null;
                    }
                    for (int v = 0; v < 4; v += 2) {
                        float x = vertices[v].worldX(), y = vertices[v].worldY(), z = vertices[v].worldZ();
                        int found = -1;
                        for (int i = 0; i < count; i += 3) {
                            if (Float.floatToRawIntBits(corners[i]) == Float.floatToRawIntBits(x) && Float.floatToRawIntBits(corners[i + 1]) == Float.floatToRawIntBits(y)
                                    && Float.floatToRawIntBits(corners[i + 2]) == Float.floatToRawIntBits(z)) {
                                found = i;
                                break;
                            }
                        }
                        if (found == -1) {
                            found = count;
                            corners[count] = x;
                            corners[count + 1] = y;
                            corners[count + 2] = z;
                            count += 3;
                        }
                        if (v == 0) {
                            first[p] = found;
                        } else {
                            third[p] = found;
                        }
                    }
                }
                infos[s][c] = new CubeInfo((TransparentCube) (Object) cube, quads, polygons.length, Arrays.copyOf(corners, count), first, third);
                quads += polygons.length;
            }
        }
        int vertexCount = quads * 4;
        if (vertexCount * MAX_MEMBERS > 65536) {
            return null;
        }

        // The mesh: one copy per member, each vertex with its member and slot
        ByteBuffer mesh = MemoryUtil.memAlloc(vertexCount * MAX_MEMBERS * VERTEX_SIZE);
        try {
            long ptr = MemoryUtil.memAddress0(mesh);
            for (int k = 0; k < MAX_MEMBERS; k++) {
                for (int s = 0; s < slotCount; s++) {
                    for (ModelPart.Cube cube : cubeLists.get(s)) {
                        for (ModelPart.Polygon polygon : cube.polygons) {
                            Vector3fc normal = polygon.normal();
                            for (ModelPart.Vertex vertex : polygon.vertices()) {
                                MemoryUtil.memPutFloat(ptr, vertex.worldX());
                                MemoryUtil.memPutFloat(ptr + 4, vertex.worldY());
                                MemoryUtil.memPutFloat(ptr + 8, vertex.worldZ());
                                MemoryUtil.memPutFloat(ptr + 12, vertex.u());
                                MemoryUtil.memPutFloat(ptr + 16, vertex.v());
                                MemoryUtil.memPutFloat(ptr + 20, normal.x());
                                MemoryUtil.memPutFloat(ptr + 24, normal.y());
                                MemoryUtil.memPutFloat(ptr + 28, normal.z());
                                MemoryUtil.memPutShort(ptr + 32, (short) k);
                                MemoryUtil.memPutShort(ptr + 34, (short) s);
                                ptr += VERTEX_SIZE;
                            }
                        }
                    }
                }
            }
            Layout layout = new Layout(generation, slotCount, cubeLists.toArray(new List<?>[0]), infos, vertexCount);
            layout.mesh = RenderSystem.getDevice().createBuffer(() -> "Ruthenium player pose mesh", GpuBuffer.USAGE_VERTEX, mesh);
            layout.meshSlice = layout.mesh.slice();
            for (int s = 0; s < slotCount; s++) {
                ((PosedPart) (Object) parts.get(s)).ruthenium$setPoseLayout(layout, s);
            }
            LAYOUTS.add(layout);
            return layout;
        } finally {
            MemoryUtil.memFree(mesh);
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

    /** Sorts this frame's posed draws and uploads their poses and indices (after the world's features are prepared) */
    public static void upload() {
        framePoses = null;
        frameIndices = null;
        framePipeline = null;
        if (poseRing != null) {
            // Fences the previous frame's use of the current buffers
            poseRing.rotate();
        }
        if (indexRing != null) {
            indexRing.rotate();
        }
        if (drawCount == 0) {
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
        indexSize = 0;
        for (int d = 0; d < drawCount; d++) {
            PoseDraw draw = DRAWS.get(d);
            int quads = draw.quadCount;
            if (draw.members == 0 || quads == 0) {
                draw.indexCount = 0;
                continue;
            }
            int[] order = sort(draw, quads, reuseOrder);
            int bytes = quads * 6 * 2;
            if (indexBytes.capacity() < indexSize + bytes) {
                indexBytes = MemoryUtil.memRealloc(indexBytes, Math.max(indexBytes.capacity() * 2, indexSize + bytes));
            }
            long ptr = MemoryUtil.memAddress0(indexBytes) + indexSize;
            int[] vertices = draw.quadVertices;
            for (int q : order) {
                int base = vertices[q];
                MemoryUtil.memPutShort(ptr, (short) base);
                MemoryUtil.memPutShort(ptr + 2, (short) (base + 1));
                MemoryUtil.memPutShort(ptr + 4, (short) (base + 2));
                MemoryUtil.memPutShort(ptr + 6, (short) (base + 2));
                MemoryUtil.memPutShort(ptr + 8, (short) (base + 3));
                MemoryUtil.memPutShort(ptr + 10, (short) base);
                ptr += 12;
            }
            draw.firstIndex = indexSize / 2;
            draw.indexCount = quads * 6;
            indexSize += bytes;
        }
        if (indexSize == 0) {
            return;
        }

        // The last draw's block is bound at its full declared size
        int poseBufferSize = poseSize + BLOCK_SIZE;
        if (poseRing == null || poseRing.size() < poseBufferSize) {
            if (poseRing != null) {
                poseRing.close();
            }
            poseRing = new MappableRingBuffer(() -> "Ruthenium player poses", GpuBuffer.USAGE_UNIFORM | GpuBuffer.USAGE_MAP_WRITE,
                    Math.max(poseBufferSize * 2, 1 << 16));
        }
        if (indexRing == null || indexRing.size() < indexSize) {
            if (indexRing != null) {
                indexRing.close();
            }
            indexRing = new MappableRingBuffer(() -> "Ruthenium player pose indices", GpuBuffer.USAGE_INDEX | GpuBuffer.USAGE_MAP_WRITE,
                    Math.max(indexSize * 2, 1 << 14));
        }
        GpuBuffer poses = poseRing.currentBuffer();
        try (GpuBufferSlice.MappedView view = poses.slice(0, poseSize).map(false, true)) {
            MemoryUtil.memCopy(MemoryUtil.memAddress0(poseBytes), MemoryUtil.memAddress(view.data()), poseSize);
        }
        GpuBuffer indices = indexRing.currentBuffer();
        try (GpuBufferSlice.MappedView view = indices.slice(0, indexSize).map(false, true)) {
            MemoryUtil.memCopy(MemoryUtil.memAddress0(indexBytes), MemoryUtil.memAddress(view.data()), indexSize);
        }
        framePoses = poses;
        frameIndices = indices;
        framePipeline = RenderSystem.getCompiledPipelineNullable(PIPELINE);
    }

    /** The draw's quad order: what vanilla's upload sorts from the decoded centroids (the same floats) */
    private static int[] sort(PoseDraw draw, int quads, boolean reuseOrder) {
        if (centroidArrays.length <= quads) {
            centroidArrays = Arrays.copyOf(centroidArrays, Math.max(quads + 1, centroidArrays.length * 2));
        }
        CompactVectorArray centroids = centroidArrays[quads];
        if (centroids == null) {
            centroids = centroidArrays[quads] = new CompactVectorArray(quads);
        }
        float[] points = draw.centroids;
        for (int q = 0; q < quads; q++) {
            centroids.set(q, points[q * 3], points[q * 3 + 1], points[q * 3 + 2]);
        }
        VertexSorting sorting = draw.sorting;
        if (sorting == null) {
            int[] order = new int[quads];
            for (int q = 0; q < quads; q++) {
                order[q] = q;
            }
            return order;
        }
        if (reuseOrder && sorting instanceof DistanceSorting.Sorter sorter) {
            return SORT_ORDERS.sort(sorter, centroids);
        }
        return sorting.sort(centroids);
    }

    /** Draws a posed draw in its group draw's place */
    public static void draw(RenderPass renderPass, PoseDraw draw) {
        GpuBuffer poses = framePoses;
        GpuBuffer indices = frameIndices;
        CompiledRenderPipeline pipeline = framePipeline;
        Layout layout = draw.layout;
        PreparedRenderType prepared = draw.prepared;
        if (draw.members == 0 || draw.indexCount == 0 || poses == null || indices == null || pipeline == null || layout == null
                || layout.meshSlice == null || prepared == null) {
            return;
        }
        // PreparedRenderType.draw with the mesh, the member block and the sorted indices
        renderPass.setPipeline(pipeline);
        ScissorState scissor = prepared.scissorState();
        if (scissor.enabled()) {
            renderPass.enableScissor(scissor.x(), scissor.y(), scissor.width(), scissor.height());
        }
        RenderSystem.bindDefaultUniforms(renderPass);
        renderPass.setUniform("DynamicTransforms", prepared.dynamicTransforms());
        renderPass.setVertexBuffer(0, layout.meshSlice);
        for (PreparedRenderType.Texture texture : prepared.textures()) {
            renderPass.setUniform(texture.name(), texture.textureView(), texture.sampler());
        }
        renderPass.setUniform("PlayerPoses", poses.slice(draw.uboOffset, BLOCK_SIZE));
        renderPass.setIndexBuffer(indices, IndexType.SHORT);
        renderPass.drawIndexed(draw.indexCount, 1, draw.firstIndex, 0, 0);
    }
}
