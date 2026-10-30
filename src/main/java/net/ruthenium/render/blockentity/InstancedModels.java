package net.ruthenium.render.blockentity;

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
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.CachedBlockEntityLight;
import net.ruthenium.mixin.render.entity.model.ArmorStandModelAccessor;
import net.ruthenium.render.entity.ModelCulling;
import net.ruthenium.render.entity.ModelGeometryCache;
import org.joml.Matrix4f;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
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
 * model rendered once with the identity pose. {@code ArmorStandModel.setupAnim} turns the base plate back by the
 * stand's yaw (it keeps its world orientation), so with {@code armorStandPlates} the plate is left out of the snapshot
 * and the mesh and drawn as an instance
 * of its own ({@link #PLATE_KEY}): stands posed alike share their groups whichever way they face. A model has at most
 * {@link #MAX_GROUPS_PER_MODEL} groups; when it needs another, a group unused for {@link #EVICT_AFTER_FRAMES} is evicted
 * (poses and yaws change: with the plates in the snapshot, the server's byte-quantized rotation sync gave every stand a new
 * key ~60 s after it spawned, and the groups full of the old keys sent most stands down vanilla's path). The instances are
 * drawn after the world's solid features with
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
    /** {@link #CUTOUT} with back faces culled, for groups of closed opaque boxes ({@link ClosedBoxes}) */
    public static final RenderPipeline CUTOUT_CLOSED = instancedPipeline("entity_cutout_closed", true, true, true);
    public static final List<RenderPipeline> PIPELINES = List.of(SOLID, CUTOUT, CUTOUT_CULL, CUTOUT_CLOSED);
    /**
     * How much farther than its mesh's bounding sphere an instance's center must be from the eye for back face culling: more
     * than the near plane (0.05) and view bobbing's offset of the eye, so the eye is outside every box and no box is clipped
     */
    private static final float CLOSED_EYE_MARGIN = 1.0f;

    /** Models drawn from their prepared submits when their parts are posed like an existing group */
    private static final Set<Class<?>> POSED_MODELS = Set.of(ArmorStandModel.class);
    private static final int MAX_GROUPS_PER_MODEL = 32;
    /**
     * A model's group that hasn't been used for this many frames may be evicted when the model is at its cap and needs a new
     * group (least recently used first, at most one eviction per frame). Groups aren't evicted otherwise, so out-of-view models
     * keep theirs; this bounds mesh rebuilds to {@link #MAX_GROUPS_PER_MODEL} per model per this many frames however fast the
     * poses change, and is far more than the frames in flight that may still read an evicted group's mesh
     */
    private static final int EVICT_AFTER_FRAMES = 1024;

    private static final int INSTANCE_SIZE = INSTANCE_FORMAT.getVertexSize();
    /** Tolerance of the rotation test on the pose's linear part */
    private static final float ROTATION_TOLERANCE = 1.0e-4f;

    private static final List<Group> GROUPS = new ArrayList<>();
    /**
     * Group key of an armor stand model's base plate: the plate alone at its scale (0.5 on small stands), without its
     * offset and rotation, which are in the instance
     */
    private static final Object PLATE_KEY = new Object();
    private static final PoseStack PLATE_POSE = new PoseStack();
    /** {@code armorStandPlates} when the groups were made; they are dropped when it changes (the groups' keys differ) */
    private static boolean groupsWithPlates;
    /** Counts frames, for {@link Group#lastUsedFrame} */
    private static int frame;
    /**
     * One eviction per frame: when a model's groups have all been idle (e.g. the models were out of view) and more keys than its
     * cap come back, the first frame would otherwise evict and rebuild a group for every key past the cap in turn
     */
    private static boolean evictedThisFrame;
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
    /** This frame's instances of every group, one after the other */
    private static @Nullable MappableRingBuffer ring;
    /** This frame's prepared render types (their textures), one per render type */
    private static final Map<RenderType, PreparedRenderType> PREPARED = new IdentityHashMap<>();

    private InstancedModels() {
    }

    private static RenderPipeline instancedPipeline(String name, boolean cutout, boolean perFaceLighting, boolean cull) {
        RenderPipeline.Builder builder = RenderPipeline.builder()
                .withBindGroupLayout(BindGroupLayouts.GLOBALS)
                .withBindGroupLayout(BindGroupLayouts.PROJECTION)
                .withBindGroupLayout(BindGroupLayouts.DYNAMIC_TRANSFORMS)
                .withBindGroupLayout(BindGroupLayouts.FOG)
                .withVertexShader(Identifier.fromNamespaceAndPath("ruthenium", "core/instanced_entity"))
                .withFragmentShader(Identifier.withDefaultNamespace("core/entity"))
                .withBindGroupLayout(BindGroupLayouts.SAMPLER0)
                .withVertexBinding(0, DefaultVertexFormat.ENTITY)
                .withVertexBinding(1, INSTANCE_FORMAT)
                .withPrimitiveTopology(PrimitiveTopology.QUADS)
                .withDepthStencilState(DepthStencilState.DEFAULT)
                .withBindGroupLayout(BindGroupLayouts.LIGHTING)
                .withBindGroupLayout(BindGroupLayouts.SAMPLER2)
                .withLocation(Identifier.fromNamespaceAndPath("ruthenium", "pipeline/instanced_" + name));
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
        /**
         * The last {@link #frame} that looked this group up or drew it (closed books after the first add instances without a
         * lookup; {@link #prepareDraws} stamps every group with instances)
         */
        int lastUsedFrame;

        ByteBuffer instances = MemoryUtil.memAlloc(64 * 52);
        int count;
        @Nullable GpuBufferSlice instanceSlice;
        /** This frame's instances' offset in the shared instance buffer */
        int instanceOffset;
        @Nullable PreparedRenderType prepared;
        /** Of a {@link #PLATE_KEY} group: the plate's scale in its mesh */
        float plateXScale, plateYScale, plateZScale;
        /** A {@link #CUTOUT} mesh of closed opaque boxes ({@link ClosedBoxes}) and its model-space bounding sphere */
        boolean closed;
        float boundX, boundY, boundZ, boundRadius;
        /** This frame: drawn with {@link #CUTOUT_CLOSED} */
        boolean cullBackFaces;

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
        boolean plates = Initializer.CONFIG.armorStandPlates;
        if (clearPending || plates != groupsWithPlates) {
            clearPending = false;
            groupsWithPlates = plates;
            pipelineAvailable = null;
            GROUPS.forEach(Group::close);
            GROUPS.clear();
            closedBookGroup = null;
            closedBookRenderer = null;
        }
        frameTableRenderer = null;
        frameLightStamp = lightGeneration;
        frame++;
        evictedThisFrame = false;
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
        if (cache.ruthenium$getLightStamp() == frameLightStamp) {
            light = cache.ruthenium$getLight();
        } else {
            if (!table.getType().isValid(table.getBlockState())) {
                return false;
            }
            light = LightCoordsUtil.getLightCoords(level, pos);
            cache.ruthenium$setLight(frameLightStamp, light);
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
        // The base plate is turned back by the stand's yaw: with armorStandPlates it is left out of the snapshot and the mesh
        // (hidden and unturned while they are taken) and drawn as an instance of its own
        ModelPart plate = groupsWithPlates && model instanceof ArmorStandModel ? ((ArmorStandModelAccessor) model).ruthenium$basePlate() : null;
        Group group;
        Group plateGroup = null;
        if (plate == null) {
            group = findGroup(model, submit.state(), snapshot(model), pipeline, submit.renderType(), submit.uvMapping(),
                    submit.overlayCoords(), submit.tintedColor());
        } else {
            float plateYRot = plate.yRot;
            boolean plateVisible = plate.visible;
            plate.yRot = 0.0F;
            plate.visible = false;
            try {
                group = findGroup(model, submit.state(), snapshot(model), pipeline, submit.renderType(), submit.uvMapping(),
                        submit.overlayCoords(), submit.tintedColor());
            } finally {
                plate.yRot = plateYRot;
                plate.visible = plateVisible;
            }
            if (group != null && plateVisible && !plate.skipDraw) {
                plateGroup = findGroup(model, submit.state(), PLATE_KEY, pipeline, submit.renderType(), submit.uvMapping(),
                        submit.overlayCoords(), submit.tintedColor());
                if (plateGroup == null || !platePose(model, plate, plateGroup, submit.pose())) {
                    return false;
                }
            }
        }
        if (group == null) {
            return false;
        }
        addInstance(group, submit.pose().pose(), submit.lightCoords());
        if (plateGroup != null) {
            addInstance(plateGroup, PLATE_POSE.last().pose(), submit.lightCoords());
        }
        return true;
    }

    /**
     * The plate's instance pose into {@link #PLATE_POSE}: what {@code ModelPart.render} multiplies onto the submit's pose
     * for it (the root's and the plate's {@code translateAndRotate}) without the plate's scale, which is in its mesh, so
     * the pose stays a rotation.
     *
     * @return false if the plate isn't drawn like that (another scale than its mesh's, or not a rotation)
     */
    private static boolean platePose(Model<?> model, ModelPart plate, Group plateGroup, PoseStack.Pose pose) {
        float xScale = plate.xScale, yScale = plate.yScale, zScale = plate.zScale;
        if (xScale != plateGroup.plateXScale || yScale != plateGroup.plateYScale || zScale != plateGroup.plateZScale) {
            return false;
        }
        PLATE_POSE.last().set(pose);
        model.root().translateAndRotate(PLATE_POSE);
        plate.xScale = plate.yScale = plate.zScale = 1.0F;
        try {
            plate.translateAndRotate(PLATE_POSE);
        } finally {
            plate.xScale = xScale;
            plate.yScale = yScale;
            plate.zScale = zScale;
        }
        return isOrthogonal(PLATE_POSE.last().pose());
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

    /**
     * Whether every instance of a closed group keeps the winding (positive determinant) and has the eye (the origin of the
     * camera-relative instance space) farther than {@link #CLOSED_EYE_MARGIN} outside its bounding sphere. The sphere's
     * radius is scaled by the linear part's Frobenius norm, which bounds how far it moves a point of the mesh.
     */
    private static boolean eyeOutsideAll(Group group) {
        long ptr = MemoryUtil.memAddress0(group.instances);
        float cx = group.boundX, cy = group.boundY, cz = group.boundZ;
        for (int i = 0; i < group.count; i++, ptr += INSTANCE_SIZE) {
            float a = MemoryUtil.memGetFloat(ptr), b = MemoryUtil.memGetFloat(ptr + 4), c = MemoryUtil.memGetFloat(ptr + 8);
            float d = MemoryUtil.memGetFloat(ptr + 16), e = MemoryUtil.memGetFloat(ptr + 20), f = MemoryUtil.memGetFloat(ptr + 24);
            float g = MemoryUtil.memGetFloat(ptr + 32), h = MemoryUtil.memGetFloat(ptr + 36), k = MemoryUtil.memGetFloat(ptr + 40);
            float det = a * (e * k - f * h) - b * (d * k - f * g) + c * (d * h - e * g);
            if (!(det > 0.0f)) {
                return false;
            }
            float x = a * cx + b * cy + c * cz + MemoryUtil.memGetFloat(ptr + 12);
            float y = d * cx + e * cy + f * cz + MemoryUtil.memGetFloat(ptr + 28);
            float z = g * cx + h * cy + k * cz + MemoryUtil.memGetFloat(ptr + 44);
            float norm = (float) Math.sqrt(a * a + b * b + c * c + d * d + e * e + f * f + g * g + h * h + k * k);
            float reach = group.boundRadius * norm + CLOSED_EYE_MARGIN;
            if (!(x * x + y * y + z * z > reach * reach)) {
                return false;
            }
        }
        return true;
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
        Group evictable = null;
        for (Group group : GROUPS) {
            if (group.matches(model, key, renderType, uvMapping, overlay, color)) {
                // Also keeps the stand's body group from being evicted by its plate's lookup
                group.lastUsedFrame = frame;
                return group.failed ? null : group;
            }
            if (group.model == model) {
                modelGroups++;
                // Never a group with instances this frame (closed books add theirs without a lookup)
                if (frame - group.lastUsedFrame > EVICT_AFTER_FRAMES && group.count == 0
                        && (evictable == null || group.lastUsedFrame - evictable.lastUsedFrame < 0)) {
                    evictable = group;
                }
            }
        }
        if (modelGroups >= MAX_GROUPS_PER_MODEL) {
            // Groups whose key is no longer seen (e.g. a stand's pose or yaw changed) would otherwise keep the model at its cap
            if (evictable == null || evictedThisFrame) {
                return null;
            }
            evictedThisFrame = true;
            GROUPS.remove(evictable);
            evictable.close();
            if (evictable == closedBookGroup) {
                closedBookGroup = null;
                closedBookRenderer = null;
            }
        }

        Object groupKey = key instanceof float[] snapshot ? snapshot.clone() : key;
        Group group = new Group(model, groupKey, pipeline, renderType, uvMapping, overlay, color);
        group.lastUsedFrame = frame;
        GROUPS.add(group);

        try (ByteBufferBuilder bytes = new ByteBufferBuilder(1 << 12)) {
            BufferBuilder builder = new BufferBuilder(bytes, PrimitiveTopology.QUADS, DefaultVertexFormat.ENTITY);
            // Light comes from the instance; all faces, the pipeline culls
            boolean written;
            if (key == PLATE_KEY) {
                // The plate alone at its scale: its offset and rotation are in the instance. The scale is left as it is
                // (PartPose, as storePose/loadPose use it, doesn't carry it)
                ModelPart plate = ((ArmorStandModelAccessor) model).ruthenium$basePlate();
                float x = plate.x, y = plate.y, z = plate.z, xRot = plate.xRot, yRot = plate.yRot, zRot = plate.zRot;
                boolean visible = plate.visible;
                group.plateXScale = plate.xScale;
                group.plateYScale = plate.yScale;
                group.plateZScale = plate.zScale;
                ModelCulling.endModel();
                try {
                    plate.x = plate.y = plate.z = 0.0F;
                    plate.xRot = plate.yRot = plate.zRot = 0.0F;
                    plate.visible = true;
                    plate.render(new PoseStack(), uvMapping != null ? uvMapping.wrap(builder) : builder, 0, overlay, color);
                    written = true;
                } finally {
                    plate.x = x;
                    plate.y = y;
                    plate.z = z;
                    plate.xRot = xRot;
                    plate.yRot = yRot;
                    plate.zRot = zRot;
                    plate.visible = visible;
                    ModelCulling.beginModel(renderType);
                }
            } else if (key instanceof float[]) {
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
                if (pipeline == CUTOUT && uvMapping == null
                        && ClosedBoxes.check(data.vertexBuffer(), data.drawState().vertexCount() / 4, DefaultVertexFormat.ENTITY.getVertexSize(), renderType)) {
                    group.closed = true;
                    group.boundX = ClosedBoxes.centerX;
                    group.boundY = ClosedBoxes.centerY;
                    group.boundZ = ClosedBoxes.centerZ;
                    group.boundRadius = ClosedBoxes.radius;
                }
                group.mesh = RenderSystem.getDevice().createBuffer(() -> "Ruthenium instanced model", GpuBuffer.USAGE_VERTEX, data.vertexBuffer());
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

    /**
     * Uploads this frame's instances (after the features are prepared, before the passes): every group's instances one
     * after the other in one buffer, each render type prepared once
     */
    public static void prepareDraws() {
        drawPending = false;
        int maxIndexCount = 0;
        PREPARED.clear();
        if (ring != null) {
            // Fences the previous frame's use of the current buffer
            ring.rotate();
        }
        int total = 0;
        for (Group group : GROUPS) {
            group.instanceSlice = null;
            if (group.count == 0 || group.mesh == null) {
                continue;
            }
            group.lastUsedFrame = frame;
            group.cullBackFaces = group.closed && Initializer.CONFIG.closedModelCulling && eyeOutsideAll(group);
            group.instanceOffset = total;
            total += group.count * INSTANCE_SIZE;
            // The same textures for every group of a render type (the draws then keep their bindings)
            PreparedRenderType prepared = PREPARED.get(group.renderType);
            if (prepared == null) {
                prepared = group.renderType.prepare();
                PREPARED.put(group.renderType, prepared);
            }
            group.prepared = prepared;
            maxIndexCount = Math.max(maxIndexCount, group.indexCount);
            drawPending = true;
        }
        if (!drawPending) {
            return;
        }

        if (ring == null || ring.size() < total) {
            if (ring != null) {
                ring.close();
            }
            ring = new MappableRingBuffer(() -> "Ruthenium model instances", GpuBuffer.USAGE_VERTEX | GpuBuffer.USAGE_MAP_WRITE,
                    Math.max(total * 2, 256 * INSTANCE_SIZE));
        }
        GpuBuffer buffer = ring.currentBuffer();
        try (GpuBufferSlice.MappedView view = buffer.slice(0, total).map(false, true)) {
            long target = MemoryUtil.memAddress(view.data());
            for (Group group : GROUPS) {
                if (group.count == 0 || group.mesh == null) {
                    continue;
                }
                int size = group.count * INSTANCE_SIZE;
                MemoryUtil.memCopy(MemoryUtil.memAddress0(group.instances), target + group.instanceOffset, size);
                group.instanceSlice = buffer.slice(group.instanceOffset, size);
            }
        }
        transform = RenderSystem.getDynamicUniforms().writeTransform(RenderSystem.getModelViewMatrixCopy());
        RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS).getBuffer(maxIndexCount);
    }

    /** Draws the instances into the world's solid pass */
    public static void draw(RenderPass renderPass) {
        if (!drawPending) {
            return;
        }

        renderPass.pushDebugGroup(() -> "Ruthenium instanced models");
        RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS);
        RenderPipeline lastPipeline = null;
        PreparedRenderType lastPrepared = null;
        for (Group group : GROUPS) {
            if (group.instanceSlice == null) {
                continue;
            }
            // Bindings are kept between draws of one pipeline (the same values vanilla's per draw binding would set)
            RenderPipeline groupPipeline = group.cullBackFaces ? CUTOUT_CLOSED : group.pipeline;
            if (groupPipeline != lastPipeline) {
                CompiledRenderPipeline pipeline = RenderSystem.getCompiledPipelineNullable(groupPipeline);
                if (pipeline == null) {
                    continue;
                }
                renderPass.setPipeline(pipeline);
                RenderSystem.bindDefaultUniforms(renderPass);
                renderPass.setUniform("DynamicTransforms", transform);
                renderPass.setIndexBuffer(indices.getBuffer(), indices.type());
                lastPipeline = groupPipeline;
                lastPrepared = null;
            }
            if (group.prepared != lastPrepared) {
                for (PreparedRenderType.Texture texture : group.prepared.textures()) {
                    renderPass.setUniform(texture.name(), texture.textureView(), texture.sampler());
                }
                lastPrepared = group.prepared;
            }
            renderPass.setVertexBuffer(0, group.mesh.slice());
            renderPass.setVertexBuffer(1, group.instanceSlice);
            renderPass.drawIndexed(group.indexCount, group.count, 0, 0, 0);
        }
        renderPass.popDebugGroup();
    }
}
