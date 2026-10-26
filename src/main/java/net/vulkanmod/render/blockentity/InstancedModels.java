package net.vulkanmod.render.blockentity;

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
import net.minecraft.client.renderer.BindGroupLayouts;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.EnchantTableRenderer;
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.blockentity.state.EnchantTableRenderState;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.client.renderer.texture.UvMapping;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.Identifier;
import net.vulkanmod.render.entity.ModelGeometryCache;
import org.joml.Matrix4f;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.List;

/**
 * Draws block entity models whose geometry only differs by their pose as GPU instances: currently the closed
 * books of enchanting tables, which spin and bob every frame (~5k of them facing the chest cube).
 * <p>
 * The model is captured once in model space ({@link ModelGeometryCache#modelSpaceKey}; all faces, exact repeats
 * left out) into a vertex buffer. Each frame the extraction runs the real renderer into a
 * {@link ModelSubmitCapture} instead of submitting it, and stores the captured pose (camera-relative, rotation
 * only) and light as one instance; the instances are drawn after the world's solid features with
 * {@link #PIPELINE}: vanilla's entity_solid with a vertex shader that applies the instance's pose to the position
 * and normal. The only difference to vanilla is rounding: the normal is quantized before the rotation instead of
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

    public static final RenderPipeline PIPELINE = RenderPipeline.builder()
            .withBindGroupLayout(BindGroupLayouts.GLOBALS)
            .withBindGroupLayout(BindGroupLayouts.PROJECTION)
            .withBindGroupLayout(BindGroupLayouts.DYNAMIC_TRANSFORMS)
            .withBindGroupLayout(BindGroupLayouts.FOG)
            .withVertexShader(Identifier.fromNamespaceAndPath("vulkanmod", "core/instanced_entity"))
            .withFragmentShader(Identifier.withDefaultNamespace("core/entity"))
            .withBindGroupLayout(BindGroupLayouts.SAMPLER0)
            .withVertexBinding(0, DefaultVertexFormat.ENTITY)
            .withVertexBinding(1, INSTANCE_FORMAT)
            .withPrimitiveTopology(PrimitiveTopology.QUADS)
            .withDepthStencilState(DepthStencilState.DEFAULT)
            .withBindGroupLayout(BindGroupLayouts.LIGHTING)
            .withBindGroupLayout(BindGroupLayouts.SAMPLER2)
            .withLocation(Identifier.fromNamespaceAndPath("vulkanmod", "pipeline/instanced_entity_solid"))
            .withBindGroupLayout(BindGroupLayouts.SAMPLER1)
            .withColorTargetState(ColorTargetState.DEFAULT)
            .build();

    /** {@code -Dvulkanmod.instancedModels=false} turns this off (until the config screen is ported) */
    private static final boolean ENABLED = !"false".equals(System.getProperty("vulkanmod.instancedModels"));
    private static final int INSTANCE_SIZE = INSTANCE_FORMAT.getVertexSize();
    /** Tolerance of the rotation test on the pose's linear part */
    private static final float ROTATION_TOLERANCE = 1.0e-4f;

    private static final List<Group> GROUPS = new ArrayList<>();
    private static final ModelSubmitCapture CAPTURE = new ModelSubmitCapture();
    private static final PoseStack POSE_STACK = new PoseStack();
    private static final PoseStack.Pose IDENTITY_POSE = new PoseStack().last();
    private static volatile boolean clearPending;
    private static @Nullable Boolean pipelineAvailable;
    private static @Nullable GpuBufferSlice transform;
    private static boolean drawPending;

    private InstancedModels() {
    }

    private static final class Group {
        final Model<?> model;
        final Object key;
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

        Group(Model<?> model, Object key, RenderType renderType, @Nullable UvMapping uvMapping, int overlay, int color) {
            this.model = model;
            this.key = key;
            this.renderType = renderType;
            this.uvMapping = uvMapping;
            this.overlay = overlay;
            this.color = color;
        }

        boolean matches(Model<?> model, Object key, RenderType renderType, @Nullable UvMapping uvMapping, int overlay, int color) {
            return this.model == model && this.key == key && this.renderType == renderType && this.uvMapping == uvMapping
                    && this.overlay == overlay && this.color == color;
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
    }

    public static void beginFrame() {
        if (clearPending) {
            clearPending = false;
            pipelineAvailable = null;
            GROUPS.forEach(Group::close);
            GROUPS.clear();
        }
        for (Group group : GROUPS) {
            group.count = 0;
        }
    }

    /**
     * Turns an extracted block entity into an instance instead of a render state to submit.
     *
     * @return false if the render state must be submitted as usual
     */
    @SuppressWarnings({"rawtypes", "unchecked"})
    public static boolean tryAdd(BlockEntityRenderDispatcher dispatcher, BlockEntityRenderState state, double camX, double camY,
                                 double camZ, CameraRenderState camera) {
        if (!ENABLED || !(state instanceof EnchantTableRenderState) || state.breakProgress != null) {
            return false;
        }
        BlockEntityRenderer renderer = dispatcher.getRenderer(state);
        if (renderer == null || renderer.getClass() != EnchantTableRenderer.class || !isPipelineAvailable()) {
            return false;
        }

        BlockPos pos = state.blockPos;
        POSE_STACK.pushPose();
        POSE_STACK.translate(pos.getX() - camX, pos.getY() - camY, pos.getZ() - camZ);
        ModelSubmitCapture capture = CAPTURE;
        capture.reset();
        renderer.submit(state, POSE_STACK, capture, camera);
        POSE_STACK.popPose();

        Object key;
        if (!capture.isSingleModel() || capture.outlineColor != 0 || capture.renderType == null
                || capture.renderType.pipeline() != RenderPipelines.ENTITY_SOLID
                || (key = ModelGeometryCache.modelSpaceKey(capture.state)) == null || !isRotation(capture.pose.pose())) {
            return false;
        }

        Group group = findGroup(capture, key);
        if (group == null) {
            return false;
        }

        if (group.instances.capacity() < (group.count + 1) * INSTANCE_SIZE) {
            group.instances = MemoryUtil.memRealloc(group.instances, group.instances.capacity() * 2);
        }
        Matrix4f m = capture.pose.pose();
        long ptr = MemoryUtil.memAddress0(group.instances) + (long) group.count * INSTANCE_SIZE;
        putRow(ptr, m.m00(), m.m10(), m.m20(), m.m30());
        putRow(ptr + 16, m.m01(), m.m11(), m.m21(), m.m31());
        putRow(ptr + 32, m.m02(), m.m12(), m.m22(), m.m32());
        MemoryUtil.memPutInt(ptr + 48, capture.lightCoords);
        group.count++;
        return true;
    }

    private static void putRow(long ptr, float x, float y, float z, float w) {
        MemoryUtil.memPutFloat(ptr, x);
        MemoryUtil.memPutFloat(ptr + 4, y);
        MemoryUtil.memPutFloat(ptr + 8, z);
        MemoryUtil.memPutFloat(ptr + 12, w);
    }

    /** The shader rotates normals with the pose itself, which is only right for rotations */
    private static boolean isRotation(Matrix4f m) {
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
    private static @Nullable Group findGroup(ModelSubmitCapture capture, Object key) {
        for (Group group : GROUPS) {
            if (group.matches(capture.model, key, capture.renderType, capture.uvMapping, capture.overlayCoords, capture.tintedColor)) {
                return group.failed ? null : group;
            }
        }

        Group group = new Group(capture.model, key, capture.renderType, capture.uvMapping, capture.overlayCoords, capture.tintedColor);
        GROUPS.add(group);

        try (ByteBufferBuilder bytes = new ByteBufferBuilder(1 << 12)) {
            BufferBuilder builder = new BufferBuilder(bytes, PrimitiveTopology.QUADS, DefaultVertexFormat.ENTITY);
            // Light comes from the instance
            boolean written = ModelGeometryCache.writeModelSpace((Model) capture.model, capture.state, key, IDENTITY_POSE, builder, 0,
                    capture.overlayCoords, capture.tintedColor, capture.uvMapping, false, true);
            try (MeshData data = written ? builder.build() : null) {
                if (data == null) {
                    group.failed = true;
                    return null;
                }
                group.mesh = RenderSystem.getDevice().createBuffer(() -> "VulkanMod instanced model", GpuBuffer.USAGE_VERTEX, data.vertexBuffer());
                group.indexCount = data.drawState().vertexCount() / 4 * 6;
            }
        }
        return group;
    }

    private static boolean isPipelineAvailable() {
        Boolean available = pipelineAvailable;
        if (available == null) {
            available = RenderSystem.getCompiledPipelineNullable(PIPELINE) != null;
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
                group.ring = new MappableRingBuffer(() -> "VulkanMod model instances",
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
        CompiledRenderPipeline pipeline = RenderSystem.getCompiledPipelineNullable(PIPELINE);
        if (pipeline == null) {
            return;
        }

        renderPass.pushDebugGroup(() -> "VulkanMod instanced models");
        renderPass.setPipeline(pipeline);
        RenderSystem.bindDefaultUniforms(renderPass);
        renderPass.setUniform("DynamicTransforms", transform);
        RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS);
        renderPass.setIndexBuffer(indices.getBuffer(), indices.type());

        for (Group group : GROUPS) {
            if (group.instanceSlice == null) {
                continue;
            }
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
