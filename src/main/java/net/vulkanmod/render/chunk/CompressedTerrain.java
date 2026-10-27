package net.vulkanmod.render.chunk;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.GpuFormat;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.resources.Identifier;
import net.vulkanmod.Initializer;

import java.nio.ByteOrder;
import java.util.List;
import java.util.Optional;

/**
 * Smaller vertices for the solid and cutout terrain layers: 20 instead of vanilla's 28 bytes.
 * <ul>
 *     <li>Position (8 bytes): x, y and z as 21 bit fixed point with 1/65536 block steps, section-relative from -8 to
 *     24 blocks. Vanilla positions are model coordinates (multiples of 1/16 pixel or finer) plus whole blocks, so
 *     they are kept exactly, except rotated model elements, which move by at most 1/131072 block.</li>
 *     <li>Color (4 bytes) and UV2 (light, 4 bytes) as in vanilla.</li>
 *     <li>UV0 (4 bytes): 16 bit fixed point, exact for texel corners of atlases up to 65536 texels wide.</li>
 * </ul>
 * Translucent sections keep vanilla's format, which their quad sorting reads. The layers' pipelines are copies of
 * vanilla's terrain pipelines with a vertex shader that decodes this format ({@code core/terrain_compressed}).
 * <p>
 * The format of a layer is fixed while its section dispatcher (and its uber buffers) lives: it is chosen when
 * vanilla creates the dispatcher, after joining a world or when the switch changes.
 */
public final class CompressedTerrain {
    public static final VertexFormat FORMAT = VertexFormat.builder(0)
            .addAttribute("Position", GpuFormat.RG32_UINT)
            .addAttribute("Color", GpuFormat.RGBA8_UNORM)
            .addAttribute("UV0", GpuFormat.RG16_UINT)
            .addAttribute("UV2", GpuFormat.RG16_SINT)
            .build();
    public static final int POSITION_OFFSET = 0;
    public static final int COLOR_OFFSET = 8;
    public static final int UV0_OFFSET = 12;
    public static final int UV2_OFFSET = 16;

    private static final double POSITION_SCALE = 65536.0;
    private static final double POSITION_BIAS = 8.0;
    private static final long POSITION_MAX = (1L << 21) - 1;
    private static final double UV_SCALE = 65536.0;

    public static final RenderPipeline SOLID = copy(RenderPipelines.SOLID_TERRAIN, "solid_terrain");
    public static final RenderPipeline SOLID_MULTIDRAW = copy(RenderPipelines.SOLID_TERRAIN_MULTIDRAW, "solid_terrain_multidraw");
    public static final RenderPipeline CUTOUT = copy(RenderPipelines.CUTOUT_TERRAIN, "cutout_terrain");
    public static final RenderPipeline CUTOUT_MULTIDRAW = copy(RenderPipelines.CUTOUT_TERRAIN_MULTIDRAW, "cutout_terrain_multidraw");
    public static final RenderPipeline WIREFRAME = copy(RenderPipelines.WIREFRAME, "wireframe");
    public static final RenderPipeline WIREFRAME_MULTIDRAW = copy(RenderPipelines.WIREFRAME_MULTIDRAW, "wireframe_multidraw");
    /** Required for the format; the wireframe copies are optional, like vanilla's */
    private static final List<RenderPipeline> LAYER_PIPELINES = List.of(SOLID, SOLID_MULTIDRAW, CUTOUT, CUTOUT_MULTIDRAW);
    public static final List<RenderPipeline> PIPELINES = List.of(SOLID, SOLID_MULTIDRAW, CUTOUT, CUTOUT_MULTIDRAW, WIREFRAME, WIREFRAME_MULTIDRAW);

    /** Whether the current section dispatcher uses this format; only changed when vanilla creates one */
    private static boolean active;

    private CompressedTerrain() {
    }

    /** Chooses the format for a section dispatcher that is about to be created */
    public static void onDispatcherCreated() {
        // The vertices are written as little-endian ints and longs
        boolean available = ByteOrder.nativeOrder() == ByteOrder.LITTLE_ENDIAN;
        for (RenderPipeline pipeline : LAYER_PIPELINES) {
            available &= RenderSystem.getCompiledPipelineNullable(pipeline) != null;
        }
        if (Initializer.CONFIG.compressedTerrain && !available) {
            Initializer.LOGGER.warn("Compressed terrain pipelines are unavailable, using vanilla's terrain vertices");
        }
        active = Initializer.CONFIG.compressedTerrain && available;
        Initializer.LOGGER.info("Terrain vertices: {}", active ? "compressed, 20 bytes" : "vanilla, 28 bytes");
    }

    public static boolean isActive(ChunkSectionLayer layer) {
        return active && !layer.translucent();
    }

    /** The layer's pipeline while this format is active, else vanilla's */
    public static RenderPipeline pipeline(ChunkSectionLayer layer, boolean multiDraw, RenderPipeline vanilla) {
        if (!isActive(layer)) {
            return vanilla;
        }
        return switch (layer) {
            case SOLID -> multiDraw ? SOLID_MULTIDRAW : SOLID;
            case CUTOUT -> multiDraw ? CUTOUT_MULTIDRAW : CUTOUT;
            default -> vanilla;
        };
    }

    /** A pipeline override for the layer: the wireframe debug view's, in this format while it is active */
    public static RenderPipeline wireframe(ChunkSectionLayer layer, RenderPipeline override) {
        if (!isActive(layer)) {
            return override;
        }
        if (override == RenderPipelines.WIREFRAME) {
            return WIREFRAME;
        }
        return override == RenderPipelines.WIREFRAME_MULTIDRAW ? WIREFRAME_MULTIDRAW : override;
    }

    public static long packPosition(float x, float y, float z) {
        return packCoordinate(x) | packCoordinate(y) << 21 | packCoordinate(z) << 42;
    }

    private static long packCoordinate(float c) {
        long fixed = Math.round((c + POSITION_BIAS) * POSITION_SCALE);
        return Math.clamp(fixed, 0, POSITION_MAX);
    }

    public static int packUv(float u, float v) {
        return packUvCoordinate(u) | packUvCoordinate(v) << 16;
    }

    private static int packUvCoordinate(float c) {
        return (int) Math.clamp(Math.round(c * UV_SCALE), 0, 65535);
    }

    /** Decodes one coordinate (0 = x, 1 = y, 2 = z) of a packed position, as the vertex shader does */
    public static float unpackCoordinate(long packed, int axis) {
        long fixed = packed >>> (21 * axis) & POSITION_MAX;
        return (float) (fixed / POSITION_SCALE - POSITION_BIAS);
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
                .withLocation(Identifier.fromNamespaceAndPath("vulkanmod", "pipeline/compressed_" + name))
                .withVertexShader(Identifier.fromNamespaceAndPath("vulkanmod", "core/terrain_compressed"))
                .withVertexBinding(0, FORMAT)
                .build();
    }
}
