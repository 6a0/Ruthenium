package net.vulkanmod.render.entity;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
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
import net.fabricmc.fabric.api.client.renderer.v1.mesh.Mesh;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.geom.builders.UVPair;
import net.minecraft.client.renderer.BindGroupLayouts;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.client.renderer.texture.UvMapping;
import net.minecraft.client.resources.model.geometry.BakedQuad;
import net.minecraft.resources.Identifier;
import net.minecraft.util.ARGB;
import net.minecraft.util.LightCoordsUtil;
import net.vulkanmod.Initializer;
import net.vulkanmod.interfaces.InstancedPartHolder;
import org.joml.Matrix3f;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.joml.Vector3fc;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.EnumSet;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Function;

/**
 * Draws the parts of the world's posed models (mobs and other entities) as GPU instances instead of writing their
 * vertices every frame.
 * <p>
 * Each model part's cubes are written once, in the part's own space, into a mesh per part, render type and UV
 * mapping ({@link PartBatch}). While a model submit of the world's solid phase is prepared, every part that vanilla
 * would write ({@code ModelPart.compile}) adds one instance instead: the part's pose, its normal matrix, light,
 * overlay and color (96 bytes instead of 36 bytes for each of its vertices). The instances of a feature renderer
 * group are drawn right after that group's own draws ({@code FeatureRenderDispatcher$PreparedGroup.execute}), with
 * instanced copies of vanilla's entity pipelines, so they keep their place among the submit orders (layers like
 * villager clothing sit on the same faces as the body, one order later) and phases.
 * <p>
 * Only render types of vanilla's {@code entity_solid}, {@code entity_cutout} and {@code entity_cutout_cull}
 * pipelines are taken (drawn with the type's prepared transform and textures); they never blend and always go to
 * the solid phase, whose groups vanilla may reorder. The only differences to vanilla are rounding: the GPU transforms
 * the positions, and normals are transformed in full precision and quantized nowhere.
 * <p>
 * Items work the same way ({@link #addItem}): the quads of an item submit without foil, all of vanilla's
 * {@code item_cutout} type (solid phase), are written once into an item-space mesh per quad list and tint colors,
 * and every submit of them adds one instance.
 * <p>
 * Only used from the render thread.
 */
public final class InstancedParts {
    public static final VertexFormat INSTANCE_FORMAT = VertexFormat.builder(1)
            .addAttribute("InstanceRow0", GpuFormat.RGBA32_FLOAT)
            .addAttribute("InstanceRow1", GpuFormat.RGBA32_FLOAT)
            .addAttribute("InstanceRow2", GpuFormat.RGBA32_FLOAT)
            .addAttribute("InstanceNormal0", GpuFormat.RGB32_FLOAT)
            .addAttribute("InstanceNormal1", GpuFormat.RGB32_FLOAT)
            .addAttribute("InstanceNormal2", GpuFormat.RGB32_FLOAT)
            .addAttribute("InstanceLightOverlay", GpuFormat.RGBA16_SINT)
            .addAttribute("InstanceColor", GpuFormat.RGBA8_UNORM)
            .build();
    private static final int INSTANCE_SIZE = INSTANCE_FORMAT.getVertexSize();

    /** Instanced copies of vanilla's entity_solid, entity_cutout and entity_cutout_cull */
    public static final RenderPipeline SOLID = instancedPipeline("entity_solid", false, false, true, false);
    public static final RenderPipeline CUTOUT = instancedPipeline("entity_cutout", true, true, false, false);
    public static final RenderPipeline CUTOUT_CULL = instancedPipeline("entity_cutout_cull", true, false, true, false);
    /** Instanced copy of vanilla's entity_solid for Fabric Renderer API meshes (item frames' z offset type) */
    public static final RenderPipeline SOLID_MESH = instancedPipeline("entity_solid_mesh", false, false, true, true);
    /** Instanced copies of vanilla's item_cutout, for baked quads and for Fabric Renderer API meshes */
    public static final RenderPipeline ITEM_CUTOUT = instancedItemPipeline("instanced_item_cutout", false);
    public static final RenderPipeline ITEM_CUTOUT_MESH = instancedItemPipeline("instanced_item_cutout_mesh", true);
    public static final List<RenderPipeline> PIPELINES = List.of(SOLID, CUTOUT, CUTOUT_CULL, SOLID_MESH, ITEM_CUTOUT, ITEM_CUTOUT_MESH);

    /** Meshes kept at most; models created over and over (by mods) fall back to vanilla beyond this */
    private static final int MAX_BATCHES = 4096;
    private static final PoseStack.Pose IDENTITY_POSE = new PoseStack().last();
    private static final ChunkSectionLayer[] NO_LAYERS = new ChunkSectionLayer[0];

    private static final List<PartBatch> BATCHES = new ArrayList<>();
    /** Item meshes by quad list (vanilla's lists belong to baked models), chained by tint colors */
    private static final Map<List<BakedQuad>, PartBatch> ITEM_BATCHES = new IdentityHashMap<>();
    /** Block model meshes by the cached mesh ({@link BlockModelMeshCache}), chained by tint colors */
    private static final Map<Mesh, PartBatch> MESH_BATCHES = new IdentityHashMap<>();
    private static int generation = 1;
    private static boolean clearPending;
    private static @Nullable Boolean pipelineAvailable;

    /** The group being prepared may take instances (world pass, reorderable, a switch on) */
    private static boolean groupActive;
    /** The model being written takes instances; its render type, pipeline and UV mapping */
    private static boolean modelActive;
    private static @Nullable RenderType modelRenderType;
    private static @Nullable RenderPipeline modelPipeline;
    private static @Nullable UvMapping modelUvMapping;
    private static final List<PartBatch> touched = new ArrayList<>();

    /** This frame's instances, in draw order, and the draws ({@link #cmds}, the first {@link #cmdCount}) */
    private static ByteBuffer frameInstances = MemoryUtil.memAlloc(1 << 16);
    private static int frameBytes;
    private static final List<DrawCmd> cmds = new ArrayList<>();
    private static int cmdCount;
    private static int maxIndexCount;
    private static final Map<RenderType, PreparedRenderType> prepared = new IdentityHashMap<>();
    private static @Nullable MappableRingBuffer ring;
    private static @Nullable GpuBuffer frameBuffer;

    private InstancedParts() {
    }

    private static RenderPipeline instancedPipeline(String name, boolean cutout, boolean perFaceLighting, boolean cull, boolean meshLight) {
        RenderPipeline.Builder builder = RenderPipeline.builder()
                .withBindGroupLayout(BindGroupLayouts.GLOBALS)
                .withBindGroupLayout(BindGroupLayouts.PROJECTION)
                .withBindGroupLayout(BindGroupLayouts.DYNAMIC_TRANSFORMS)
                .withBindGroupLayout(BindGroupLayouts.FOG)
                .withVertexShader(Identifier.fromNamespaceAndPath("vulkanmod", "core/instanced_part"))
                .withFragmentShader(Identifier.withDefaultNamespace("core/entity"))
                .withBindGroupLayout(BindGroupLayouts.SAMPLER0)
                .withVertexBinding(0, DefaultVertexFormat.ENTITY)
                .withVertexBinding(1, INSTANCE_FORMAT)
                .withPrimitiveTopology(PrimitiveTopology.QUADS)
                .withDepthStencilState(DepthStencilState.DEFAULT)
                .withBindGroupLayout(BindGroupLayouts.LIGHTING)
                .withBindGroupLayout(BindGroupLayouts.SAMPLER2)
                .withLocation(Identifier.fromNamespaceAndPath("vulkanmod", "pipeline/instanced_part_" + name));
        if (cutout) {
            builder.withShaderDefine("ALPHA_CUTOUT", 0.1F);
        }
        if (perFaceLighting) {
            builder.withShaderDefine("PER_FACE_LIGHTING");
        }
        if (meshLight) {
            builder.withShaderDefine("MESH_LIGHT");
        }
        return builder.withBindGroupLayout(BindGroupLayouts.SAMPLER1)
                .withCull(cull)
                .withColorTargetState(ColorTargetState.DEFAULT)
                .build();
    }

    private static RenderPipeline instancedItemPipeline(String name, boolean meshLight) {
        RenderPipeline.Builder builder = RenderPipeline.builder()
                .withBindGroupLayout(BindGroupLayouts.GLOBALS)
                .withBindGroupLayout(BindGroupLayouts.PROJECTION)
                .withBindGroupLayout(BindGroupLayouts.DYNAMIC_TRANSFORMS)
                .withBindGroupLayout(BindGroupLayouts.FOG)
                .withBindGroupLayout(BindGroupLayouts.LIGHTING)
                .withVertexShader(Identifier.fromNamespaceAndPath("vulkanmod", "core/instanced_item"))
                .withFragmentShader(Identifier.withDefaultNamespace("core/item"))
                .withBindGroupLayout(BindGroupLayouts.SAMPLER0_SAMPLER1_SAMPLER2)
                .withVertexBinding(0, DefaultVertexFormat.ENTITY)
                .withVertexBinding(1, INSTANCE_FORMAT)
                .withPrimitiveTopology(PrimitiveTopology.QUADS)
                .withDepthStencilState(DepthStencilState.DEFAULT)
                .withLocation(Identifier.fromNamespaceAndPath("vulkanmod", "pipeline/" + name))
                .withShaderDefine("ALPHA_CUTOUT", 0.1F);
        if (meshLight) {
            builder.withShaderDefine("MESH_LIGHT");
        }
        return builder.withColorTargetState(ColorTargetState.DEFAULT).build();
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

    /**
     * A part's mesh for one render type and UV mapping, or an item's for its tint colors, and the instances it collects
     * in the current group
     */
    public static final class PartBatch {
        /** Null only for a block model mesh whose layers have no render type (never drawn) */
        final @Nullable RenderType renderType;
        final @Nullable UvMapping uvMapping;
        final int @Nullable [] tints;
        /** The chunk layers of a block model mesh's quads, each drawn with {@link #renderType} */
        ChunkSectionLayer[] layers = NO_LAYERS;
        final RenderPipeline pipeline;
        final int generation;
        final @Nullable PartBatch next;
        @Nullable GpuBuffer mesh;
        int indexCount;
        ByteBuffer pending = MemoryUtil.memAlloc(16 * INSTANCE_SIZE);
        int count;

        PartBatch(@Nullable RenderType renderType, @Nullable UvMapping uvMapping, int @Nullable [] tints, RenderPipeline pipeline, int generation,
                  @Nullable PartBatch next) {
            this.renderType = renderType;
            this.uvMapping = uvMapping;
            this.tints = tints;
            this.pipeline = pipeline;
            this.generation = generation;
            this.next = next;
        }

        void close() {
            if (this.mesh != null) {
                this.mesh.close();
                this.mesh = null;
            }
            MemoryUtil.memFree(this.pending);
        }
    }

    private static final class DrawCmd {
        PartBatch batch;
        PreparedRenderType prepared;
        int offset;
        int count;
    }

    /** Drops every mesh at the start of the next frame (resource reload, switch toggled) */
    public static void clear() {
        clearPending = true;
    }

    /** Called before the world's features are prepared */
    public static void beginFrame() {
        if (clearPending) {
            clearPending = false;
            pipelineAvailable = null;
            BATCHES.forEach(PartBatch::close);
            BATCHES.clear();
            ITEM_BATCHES.clear();
            MESH_BATCHES.clear();
            generation++;
        }
        frameBytes = 0;
        cmdCount = 0;
        maxIndexCount = 0;
        prepared.clear();
        frameBuffer = null;
    }

    /**
     * A feature renderer group starts preparing.
     *
     * @return the index of its first draw
     */
    public static int beginGroup(boolean strictlyOrdered) {
        groupActive = !strictlyOrdered && ModelCulling.isWorldPass()
                && (Initializer.CONFIG.instancedParts || Initializer.CONFIG.instancedItems || Initializer.CONFIG.instancedBlockModels)
                && isPipelineAvailable();
        return cmdCount;
    }

    /**
     * The group has been prepared: its instances are moved into the frame's buffer, one draw per part batch.
     *
     * @return the index after its last draw
     */
    public static int endGroup() {
        groupActive = false;
        for (int i = 0; i < touched.size(); i++) {
            PartBatch batch = touched.get(i);
            int size = batch.count * INSTANCE_SIZE;
            if (frameInstances.capacity() < frameBytes + size) {
                frameInstances = MemoryUtil.memRealloc(frameInstances, Math.max(frameInstances.capacity() * 2, frameBytes + size));
            }
            MemoryUtil.memCopy(MemoryUtil.memAddress0(batch.pending), MemoryUtil.memAddress0(frameInstances) + frameBytes, size);

            if (cmdCount == cmds.size()) {
                cmds.add(new DrawCmd());
            }
            DrawCmd cmd = cmds.get(cmdCount++);
            cmd.batch = batch;
            cmd.prepared = prepared.computeIfAbsent(batch.renderType, RenderType::prepare);
            cmd.offset = frameBytes;
            cmd.count = batch.count;
            frameBytes += size;
            maxIndexCount = Math.max(maxIndexCount, batch.indexCount);
            batch.count = 0;
        }
        touched.clear();
        return cmdCount;
    }

    /**
     * A model submit is about to be written: its parts become instances if its render type allows it.
     *
     * @return whether {@link #endModel} must be called
     */
    public static boolean beginModel(RenderType renderType, @Nullable UvMapping uvMapping) {
        if (!groupActive || !Initializer.CONFIG.instancedParts) {
            return false;
        }
        RenderPipeline pipeline = instancedPipeline(renderType.pipeline());
        if (pipeline == null) {
            return false;
        }
        modelActive = true;
        modelRenderType = renderType;
        modelPipeline = pipeline;
        modelUvMapping = uvMapping;
        return true;
    }

    public static void endModel() {
        modelActive = false;
        modelRenderType = null;
        modelPipeline = null;
        modelUvMapping = null;
    }

    /**
     * Adds a part's cubes, posed by {@code pose}, as an instance.
     *
     * @return false if the cubes must be written as usual
     */
    public static boolean addPart(ModelPart part, List<ModelPart.Cube> cubes, PoseStack.Pose pose, int lightCoords, int overlayCoords, int color) {
        if (!modelActive) {
            return false;
        }
        if (cubes.isEmpty()) {
            return true;
        }
        PartBatch batch = findBatch(part, cubes);
        if (batch == null) {
            return false;
        }
        addInstance(batch, pose, lightCoords, overlayCoords, color);
        return true;
    }

    /**
     * Adds the quads of an item submit without foil, posed by {@code pose}, as an instance: an item-space mesh of the
     * quads per tint colors, in vanilla's {@code item_cutout} pipeline (the item's light and overlay per instance,
     * the quads' tints and light emission in the mesh, as {@code ItemFeatureRenderer} writes them).
     *
     * @return false if the quads must be written as usual
     */
    public static boolean addItem(List<BakedQuad> quads, int[] tintLayers, PoseStack.Pose pose, int lightCoords, int overlayCoords) {
        if (!groupActive || !Initializer.CONFIG.instancedItems || quads.isEmpty()) {
            return false;
        }
        PartBatch batch = findItemBatch(quads, tintLayers);
        if (batch == null) {
            return false;
        }
        addInstance(batch, pose, lightCoords, overlayCoords, -1);
        return true;
    }

    /**
     * Adds a block model mesh of an entity (item frames, block displays, minecart blocks: Indigo's
     * {@code ExtendedBlockModelFeatureRenderer}), posed by {@code pose}, as an instance: a model-space mesh of its quads
     * per tint colors, in vanilla's {@code item_cutout} or {@code entity_solid} pipeline, with the colors, normals and
     * light that renderer writes (each vertex's light raised to the submit's, emissive quads at full brightness).
     *
     * @param renderTypes the render type of each chunk layer; every layer of the mesh must give the same such type
     * @return false if the mesh must be written as usual
     */
    public static boolean addBlockMesh(Mesh mesh, Function<ChunkSectionLayer, @Nullable RenderType> renderTypes, int tintColor, int[] tintLayers,
                                       PoseStack.Pose pose, int lightCoords, int overlayCoords) {
        if (!groupActive || !Initializer.CONFIG.instancedBlockModels) {
            return false;
        }
        PartBatch batch = findMeshBatch(mesh, renderTypes, tintColor, tintLayers);
        if (batch == null) {
            return false;
        }
        ChunkSectionLayer[] layers = batch.layers;
        for (int i = 1; i < layers.length; i++) {
            if (renderTypes.apply(layers[i]) != batch.renderType) {
                return false;
            }
        }
        addInstance(batch, pose, lightCoords, overlayCoords, -1);
        return true;
    }

    private static void addInstance(PartBatch batch, PoseStack.Pose pose, int lightCoords, int overlayCoords, int color) {
        if (batch.count == 0) {
            touched.add(batch);
        }
        if (batch.pending.capacity() < (batch.count + 1) * INSTANCE_SIZE) {
            batch.pending = MemoryUtil.memRealloc(batch.pending, batch.pending.capacity() * 2);
        }
        long ptr = MemoryUtil.memAddress0(batch.pending) + (long) batch.count * INSTANCE_SIZE;
        Matrix4f m = pose.pose();
        putRow(ptr, m.m00(), m.m10(), m.m20(), m.m30());
        putRow(ptr + 16, m.m01(), m.m11(), m.m21(), m.m31());
        putRow(ptr + 32, m.m02(), m.m12(), m.m22(), m.m32());
        // Vanilla transforms normals with the normal matrix: n' = N n (normalized in the shader)
        Matrix3f n = pose.normal();
        putRow3(ptr + 48, n.m00(), n.m10(), n.m20());
        putRow3(ptr + 60, n.m01(), n.m11(), n.m21());
        putRow3(ptr + 72, n.m02(), n.m12(), n.m22());
        MemoryUtil.memPutInt(ptr + 84, lightCoords);
        MemoryUtil.memPutInt(ptr + 88, overlayCoords);
        MemoryUtil.memPutInt(ptr + 92, ARGB.toABGR(color));
        batch.count++;
    }

    private static void putRow(long ptr, float x, float y, float z, float w) {
        MemoryUtil.memPutFloat(ptr, x);
        MemoryUtil.memPutFloat(ptr + 4, y);
        MemoryUtil.memPutFloat(ptr + 8, z);
        MemoryUtil.memPutFloat(ptr + 12, w);
    }

    private static void putRow3(long ptr, float x, float y, float z) {
        MemoryUtil.memPutFloat(ptr, x);
        MemoryUtil.memPutFloat(ptr + 4, y);
        MemoryUtil.memPutFloat(ptr + 8, z);
    }

    private static @Nullable PartBatch findBatch(ModelPart part, List<ModelPart.Cube> cubes) {
        InstancedPartHolder holder = (InstancedPartHolder) (Object) part;
        PartBatch head = holder.vulkanmod$getBatch();
        if (head != null && head.generation != generation) {
            head = null;
        }
        for (PartBatch batch = head; batch != null; batch = batch.next) {
            if (batch.renderType == modelRenderType && batch.uvMapping == modelUvMapping) {
                return batch.mesh != null ? batch : null;
            }
        }
        if (BATCHES.size() >= MAX_BATCHES) {
            return null;
        }

        PartBatch batch = new PartBatch(modelRenderType, modelUvMapping, null, modelPipeline, generation, head);
        holder.vulkanmod$setBatch(batch);
        BATCHES.add(batch);

        try (ByteBufferBuilder bytes = new ByteBufferBuilder(cubes.size() * 24 * DefaultVertexFormat.ENTITY.getVertexSize())) {
            BufferBuilder builder = new BufferBuilder(bytes, PrimitiveTopology.QUADS, DefaultVertexFormat.ENTITY);
            VertexConsumer target = modelUvMapping != null ? modelUvMapping.wrap(builder) : builder;
            // All faces in part space: no CPU culling (the pipeline culls), light, overlay and color come from the instance
            ModelCulling.endModel();
            try {
                for (ModelPart.Cube cube : cubes) {
                    cube.compile(IDENTITY_POSE, target, 0, 0, -1);
                }
            } finally {
                ModelCulling.beginModel(modelRenderType);
            }
            try (MeshData data = builder.build()) {
                if (data == null) {
                    return null;
                }
                batch.mesh = RenderSystem.getDevice().createBuffer(() -> "VulkanMod instanced part", GpuBuffer.USAGE_VERTEX, data.vertexBuffer());
                batch.indexCount = data.drawState().vertexCount() / 4 * 6;
            }
        }
        return batch;
    }

    private static @Nullable PartBatch findItemBatch(List<BakedQuad> quads, int[] tintLayers) {
        PartBatch head = ITEM_BATCHES.get(quads);
        for (PartBatch batch = head; batch != null; batch = batch.next) {
            if (Arrays.equals(batch.tints, tintLayers)) {
                return batch.mesh != null ? batch : null;
            }
        }
        if (BATCHES.size() >= MAX_BATCHES) {
            return null;
        }

        // Only lists of item_cutout quads (one render type): others keep vanilla's path, the batch without a mesh says so
        RenderType renderType = quads.getFirst().materialInfo().itemRenderType();
        boolean instanced = renderType.pipeline() == RenderPipelines.ITEM_CUTOUT;
        for (int i = 1; i < quads.size() && instanced; i++) {
            instanced = quads.get(i).materialInfo().itemRenderType() == renderType;
        }

        PartBatch batch = new PartBatch(renderType, null, tintLayers.clone(), ITEM_CUTOUT, generation, head);
        ITEM_BATCHES.put(quads, batch);
        BATCHES.add(batch);
        if (!instanced) {
            return null;
        }

        try (ByteBufferBuilder bytes = new ByteBufferBuilder(quads.size() * 4 * DefaultVertexFormat.ENTITY.getVertexSize())) {
            BufferBuilder builder = new BufferBuilder(bytes, PrimitiveTopology.QUADS, DefaultVertexFormat.ENTITY);
            for (BakedQuad quad : quads) {
                BakedQuad.MaterialInfo material = quad.materialInfo();
                // ItemFeatureRenderer.getLayerColorSafe
                int tint = material.isTinted() && material.tintIndex() < tintLayers.length ? tintLayers[material.tintIndex()] : -1;
                Vector3fc normal = quad.direction().getUnitVec3f();
                for (int vertex = 0; vertex < 4; vertex++) {
                    Vector3fc position = quad.position(vertex);
                    long packedUv = quad.packedUV(vertex);
                    // The light is the quad's emission alone, raised in the shader to the instance's light
                    builder.addVertex(position.x(), position.y(), position.z(), tint, UVPair.unpackU(packedUv), UVPair.unpackV(packedUv),
                            OverlayTexture.NO_OVERLAY, material.lightEmission(), normal.x(), normal.y(), normal.z());
                }
            }
            try (MeshData data = builder.build()) {
                if (data == null) {
                    return null;
                }
                batch.mesh = RenderSystem.getDevice().createBuffer(() -> "VulkanMod instanced item", GpuBuffer.USAGE_VERTEX, data.vertexBuffer());
                batch.indexCount = data.drawState().vertexCount() / 4 * 6;
            }
        }
        return batch;
    }

    private static @Nullable RenderPipeline instancedMeshPipeline(RenderPipeline vanilla) {
        if (vanilla == RenderPipelines.ITEM_CUTOUT) {
            return ITEM_CUTOUT_MESH;
        } else if (vanilla == RenderPipelines.ENTITY_SOLID || vanilla == RenderPipelines.ENTITY_SOLID_Z_OFFSET_FORWARD) {
            return SOLID_MESH;
        }
        return null;
    }

    private static @Nullable PartBatch findMeshBatch(Mesh mesh, Function<ChunkSectionLayer, @Nullable RenderType> renderTypes, int tintColor,
                                                     int[] tintLayers) {
        // Batches are per render type of the mesh's first layer (an outline or other submit of the same mesh may
        // come first) and tint colors
        PartBatch head = MESH_BATCHES.get(mesh);
        ChunkSectionLayer[] layers;
        if (head != null) {
            layers = head.layers;
        } else {
            if (mesh.size() == 0) {
                return null;
            }
            EnumSet<ChunkSectionLayer> layerSet = EnumSet.noneOf(ChunkSectionLayer.class);
            mesh.forEach(quad -> layerSet.add(quad.chunkLayer()));
            layers = layerSet.toArray(NO_LAYERS);
        }
        RenderType renderType = renderTypes.apply(layers[0]);
        for (PartBatch batch = head; batch != null; batch = batch.next) {
            int[] tints = batch.tints;
            if (batch.renderType == renderType && tints[0] == tintColor && Arrays.equals(tints, 1, tints.length, tintLayers, 0, tintLayers.length)) {
                return batch.mesh != null ? batch : null;
            }
        }
        if (BATCHES.size() >= MAX_BATCHES) {
            return null;
        }

        // Only meshes whose layers all map to one item_cutout or entity_solid type (item frames submit with a z
        // offset): others keep Indigo's path, the batch without a mesh says so
        RenderPipeline pipeline = renderType == null ? null : instancedMeshPipeline(renderType.pipeline());
        boolean instanced = pipeline != null;
        for (ChunkSectionLayer layer : layers) {
            instanced &= renderTypes.apply(layer) == renderType;
        }

        int[] tints = new int[tintLayers.length + 1];
        tints[0] = tintColor;
        System.arraycopy(tintLayers, 0, tints, 1, tintLayers.length);
        PartBatch batch = new PartBatch(renderType, null, tints, instanced ? pipeline : ITEM_CUTOUT_MESH, generation, head);
        batch.layers = layers;
        MESH_BATCHES.put(mesh, batch);
        BATCHES.add(batch);
        if (!instanced) {
            return null;
        }

        try (ByteBufferBuilder bytes = new ByteBufferBuilder(mesh.size() * 4 * DefaultVertexFormat.ENTITY.getVertexSize())) {
            BufferBuilder builder = new BufferBuilder(bytes, PrimitiveTopology.QUADS, DefaultVertexFormat.ENTITY);
            Vector3f normal = new Vector3f();
            mesh.forEach(quad -> {
                // ExtendedBlockModelFeatureRenderer.bufferQuad
                int tintIndex = quad.tintIndex();
                int tint = tintIndex != -1 && tintIndex < tintLayers.length ? ARGB.multiply(tintColor, tintLayers[tintIndex]) : tintColor;
                for (int vertex = 0; vertex < 4; vertex++) {
                    if (quad.hasNormal(vertex)) {
                        quad.copyNormal(vertex, normal);
                    } else {
                        normal.set(quad.faceNormal());
                    }
                    // The light is the vertex's own, raised in the shader to the instance's
                    int light = quad.emissive() ? LightCoordsUtil.FULL_BRIGHT : quad.lightmap(vertex);
                    builder.addVertex(quad.x(vertex), quad.y(vertex), quad.z(vertex), ARGB.multiply(quad.color(vertex), tint), quad.u(vertex),
                            quad.v(vertex), OverlayTexture.NO_OVERLAY, light, normal.x(), normal.y(), normal.z());
                }
            });
            try (MeshData data = builder.build()) {
                if (data == null) {
                    return null;
                }
                batch.mesh = RenderSystem.getDevice().createBuffer(() -> "VulkanMod instanced block model", GpuBuffer.USAGE_VERTEX, data.vertexBuffer());
                batch.indexCount = data.drawState().vertexCount() / 4 * 6;
            }
        }
        return batch;
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

    /** Uploads this frame's instances (after the world's features are prepared, before the passes) */
    public static void upload() {
        frameBuffer = null;
        if (ring != null) {
            // Fences the previous frame's use of the current buffer
            ring.rotate();
        }
        if (frameBytes == 0) {
            return;
        }
        if (ring == null || ring.size() < frameBytes) {
            if (ring != null) {
                ring.close();
            }
            int capacity = Math.max(frameBytes * 2, 1 << 16);
            ring = new MappableRingBuffer(() -> "VulkanMod part instances", GpuBuffer.USAGE_VERTEX | GpuBuffer.USAGE_MAP_WRITE, capacity);
        }
        GpuBuffer buffer = ring.currentBuffer();
        try (GpuBufferSlice.MappedView view = buffer.slice(0, frameBytes).map(false, true)) {
            MemoryUtil.memCopy(MemoryUtil.memAddress0(frameInstances), MemoryUtil.memAddress(view.data()), frameBytes);
        }
        frameBuffer = buffer;
        RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS).getBuffer(maxIndexCount);
    }

    /** Draws the instances of a group's draws {@code [first, end)}, right after the group's own draws */
    public static void draw(RenderPass renderPass, int first, int end) {
        GpuBuffer instances = frameBuffer;
        if (first >= end || instances == null || end > cmdCount) {
            return;
        }

        renderPass.pushDebugGroup(() -> "VulkanMod instanced parts");
        RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS);
        RenderPipeline lastPipeline = null;
        PreparedRenderType lastPrepared = null;
        for (int i = first; i < end; i++) {
            DrawCmd cmd = cmds.get(i);
            PartBatch batch = cmd.batch;
            if (batch.mesh == null) {
                continue;
            }
            if (batch.pipeline != lastPipeline) {
                CompiledRenderPipeline pipeline = RenderSystem.getCompiledPipelineNullable(batch.pipeline);
                if (pipeline == null) {
                    continue;
                }
                renderPass.setPipeline(pipeline);
                RenderSystem.bindDefaultUniforms(renderPass);
                renderPass.setIndexBuffer(indices.getBuffer(), indices.type());
                lastPipeline = batch.pipeline;
                lastPrepared = null;
            }
            if (cmd.prepared != lastPrepared) {
                renderPass.setUniform("DynamicTransforms", cmd.prepared.dynamicTransforms());
                for (PreparedRenderType.Texture texture : cmd.prepared.textures()) {
                    renderPass.setUniform(texture.name(), texture.textureView(), texture.sampler());
                }
                lastPrepared = cmd.prepared;
            }
            renderPass.setVertexBuffer(0, batch.mesh.slice());
            renderPass.setVertexBuffer(1, instances.slice(cmd.offset, (long) cmd.count * INSTANCE_SIZE));
            renderPass.drawIndexed(batch.indexCount, cmd.count, 0, 0, 0);
        }
        renderPass.popDebugGroup();
    }
}
