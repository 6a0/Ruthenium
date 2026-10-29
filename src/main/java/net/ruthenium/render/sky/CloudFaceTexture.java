package net.ruthenium.render.sky;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.GpuFormat;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.device.GpuDevice;
import com.mojang.renderpearl.api.pipeline.BindGroupLayout;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.CompiledRenderPipeline;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.renderpearl.api.pipeline.UniformType;
import com.mojang.renderpearl.api.textures.FilterMode;
import com.mojang.renderpearl.api.textures.GpuTexture;
import com.mojang.renderpearl.api.textures.GpuTextureView;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.minecraft.client.renderer.BindGroupLayouts;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.oit.OitPipelineSet;
import net.minecraft.resources.Identifier;
import net.ruthenium.Initializer;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * Vanilla's clouds read their faces (3 bytes each: cell x / 2, cell z / 2, direction and flags) from a texel buffer
 * ({@code CloudFaces}, R8_SINT). On the NVIDIA Vulkan driver a submit that uses any texel buffer costs ~20 us more in
 * {@code vkQueueSubmit2}, and {@code VulkanRenderPass.pushDescriptors} creates a {@code VkBufferView} for every push
 * that binds one. With the switch on, the same bytes also go into an R8_SINT 2D texture when the cloud mesh is rebuilt
 * (face byte i at texel (i % 1024, i / 1024)), and the clouds are drawn with copies of vanilla's cloud pipelines whose
 * vertex shader ({@code ruthenium:core/clouds}) reads them with {@code texelFetch} from an {@code isampler2D}: the same
 * sign-extended integers and the same arithmetic as vanilla's shader. Vanilla's texel buffer is still written, so the
 * switch can change at any frame.
 */
public final class CloudFaceTexture {
    /** Texels per row of the face texture */
    public static final int WIDTH = 1024;
    private static final BindGroupLayout CLOUD_INFO = BindGroupLayout.builder()
            .withUniform("CloudInfo", UniformType.UNIFORM_BUFFER)
            .withUniform("CloudFaces", UniformType.COMBINED_IMAGE_SAMPLER)
            .build();

    /** Vanilla's cloud pipelines (also the OIT stages) and their copies reading the face texture */
    private static final Map<RenderPipeline, RenderPipeline> COPIES = new IdentityHashMap<>();
    public static final List<RenderPipeline> PIPELINES = new ArrayList<>();

    static {
        copy(RenderPipelines.CLOUDS);
        copy(RenderPipelines.FLAT_CLOUDS);
        copy(RenderPipelines.OIT_CLOUDS);
        copy(RenderPipelines.OIT_FLAT_CLOUDS);
    }

    private static void copy(OitPipelineSet set) {
        copy(set.depthBoundsPipeline());
        copy(set.transmittancePipeline());
        copy(set.accumulatePipeline());
    }

    private static void copy(RenderPipeline vanilla) {
        List<ColorTargetState> colorTargets = vanilla.getColorTargetStates();
        List<VertexFormat> vertexFormats = vanilla.getVertexFormatBindings();
        List<BindGroupLayout> layouts = new ArrayList<>(vanilla.getBindGroupLayouts());
        int cloudInfo = layouts.indexOf(BindGroupLayouts.CLOUD_INFO);
        if (cloudInfo < 0) {
            Initializer.LOGGER.warn("No CloudInfo layout in {}; its clouds keep vanilla's texel buffer", vanilla.getLocation());
            return;
        }
        layouts.set(cloudInfo, CLOUD_INFO);
        RenderPipeline.Snippet snippet = new RenderPipeline.Snippet(
                vanilla.getShaders(),
                Optional.of(vanilla.getShaderDefines()),
                Optional.of(List.copyOf(layouts)),
                colorTargets.toArray(new ColorTargetState[0]),
                colorTargets.size(),
                Optional.ofNullable(vanilla.getDepthStencilState()),
                Optional.of(vanilla.getPolygonMode()),
                Optional.of(vanilla.isCull()),
                vertexFormats.toArray(new VertexFormat[0]),
                Optional.of(vanilla.getPrimitiveTopology()),
                vanilla.pushConstantSize());
        RenderPipeline copy = RenderPipeline.builder(snippet)
                .withLocation(Identifier.fromNamespaceAndPath("ruthenium", vanilla.getLocation().getPath()))
                .withVertexShader(Identifier.fromNamespaceAndPath("ruthenium", "core/clouds"))
                .build();
        COPIES.put(vanilla, copy);
        PIPELINES.add(copy);
    }

    /** The faces as vanilla's buildMesh writes them, row padded to the texture's width */
    private @Nullable ByteBuffer scratch;
    /** The mapped texel buffer vanilla handed to buildMesh, while it builds into the scratch buffer instead */
    private @Nullable ByteBuffer target;
    private @Nullable GpuTexture texture;
    private @Nullable GpuTextureView view;
    /** Whether the texture holds the faces of vanilla's current mesh */
    private boolean current;

    public static boolean enabled() {
        return Initializer.CONFIG.cloudFaceTexture;
    }

    /** Whether the next prepare must rebuild the mesh so the texture gets its faces */
    public boolean needsBuild() {
        return enabled() && !this.current;
    }

    /**
     * The buffer vanilla's buildMesh writes into: with the switch on the scratch buffer (copied into vanilla's mapped
     * buffer and the texture afterwards, {@link #afterBuild}), else vanilla's own
     */
    public ByteBuffer beforeBuild(ByteBuffer faceBuffer) {
        this.target = null;
        if (!enabled()) {
            this.current = false;
            return faceBuffer;
        }
        int capacity = (faceBuffer.remaining() + WIDTH - 1) / WIDTH * WIDTH;
        ByteBuffer buffer = this.scratch;
        if (buffer == null || buffer.capacity() < capacity) {
            if (buffer != null) {
                MemoryUtil.memFree(buffer);
            }
            buffer = this.scratch = MemoryUtil.memCalloc(capacity);
        }
        // Vanilla's buffer may not take more than it has left
        buffer.clear().limit(faceBuffer.remaining());
        this.target = faceBuffer;
        return buffer;
    }

    /** After buildMesh: the faces go into vanilla's mapped buffer as it would have written them, and into the texture */
    public void afterBuild() {
        ByteBuffer faceBuffer = this.target;
        ByteBuffer buffer = this.scratch;
        if (faceBuffer == null || buffer == null) {
            return;
        }
        this.target = null;
        int bytes = buffer.position();
        int at = faceBuffer.position();
        MemoryUtil.memCopy(MemoryUtil.memAddress0(buffer), MemoryUtil.memAddress0(faceBuffer) + at, bytes);
        faceBuffer.position(at + bytes);

        GpuDevice device = RenderSystem.getDevice();
        int height = buffer.capacity() / WIDTH;
        if (this.texture == null || this.texture.getHeight(0) < height) {
            this.closeTexture();
            this.texture = device.createTexture(() -> "Ruthenium cloud faces", GpuTexture.USAGE_COPY_DST | GpuTexture.USAGE_TEXTURE_BINDING,
                    GpuFormat.R8_SINT, WIDTH, height, 1, 1);
            this.view = device.createTextureView(this.texture);
        }
        int rows = (bytes + WIDTH - 1) / WIDTH;
        if (rows > 0) {
            // Whole rows; the bytes after the last face are never read
            device.createCommandEncoder().writeToTexture(this.texture, MemoryUtil.memByteBuffer(MemoryUtil.memAddress0(buffer), rows * WIDTH),
                    0, 0, 0, 0, WIDTH, rows);
        }
        this.current = true;
    }

    /**
     * Vanilla's {@code CloudRenderer.render(RenderPass, RenderPipeline)} with the pipeline's copy and the face texture;
     * false if they can't be used (vanilla draws then)
     */
    public boolean draw(RenderPass renderPass, RenderPipeline renderPipeline, MappableRingBuffer ubo, int quadCount) {
        GpuTextureView faces = this.view;
        if (!enabled() || !this.current || faces == null) {
            return false;
        }
        RenderPipeline copy = COPIES.get(renderPipeline);
        CompiledRenderPipeline compiled = copy != null ? RenderSystem.getCompiledPipelineNullable(copy) : null;
        if (compiled == null) {
            return false;
        }
        GpuBufferSlice dynamicTransforms = RenderSystem.getDynamicUniforms().writeTransform(RenderSystem.getModelViewMatrixCopy());
        RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS);
        GpuBuffer indexBuffer = indices.getBuffer();
        renderPass.setPipeline(compiled);
        renderPass.setUniform("DynamicTransforms", dynamicTransforms);
        renderPass.setIndexBuffer(indexBuffer, indices.type());
        renderPass.setVertexBuffer(0, null);
        renderPass.setUniform("CloudInfo", ubo.currentBuffer());
        renderPass.setUniform("CloudFaces", faces, RenderSystem.getSamplerCache().getClampToEdge(FilterMode.NEAREST));
        renderPass.drawIndexed(6 * quadCount, 1, 0, 0, 0);
        return true;
    }

    private void closeTexture() {
        if (this.view != null) {
            this.view.close();
            this.view = null;
        }
        if (this.texture != null) {
            this.texture.close();
            this.texture = null;
        }
        this.current = false;
    }

    public void close() {
        this.closeTexture();
        if (this.scratch != null) {
            MemoryUtil.memFree(this.scratch);
            this.scratch = null;
        }
    }
}
