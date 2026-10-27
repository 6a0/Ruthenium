package net.vulkanmod.gametest;

import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.VertexSorting;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.SectionBufferBuilderPack;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.RenderRegionCache;
import net.minecraft.client.renderer.chunk.SectionCompiler;
import net.minecraft.core.SectionPos;
import net.vulkanmod.Initializer;
import net.vulkanmod.gametest.mixin.SectionRenderDispatcherAccessor;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.nio.ByteBuffer;
import java.util.EnumMap;
import java.util.Map;

/**
 * {@code -Pbench.meshCheck=true}: compiles the sections around the player with the mesher optimisations
 * ({@code biomeBlendCache}, {@code sectionLightCache}, {@code sectionStateCache}) on and off and requires byte-identical meshes. Vanilla's
 * tint caches are cleared before each compile, so the "off" compile computes every tint itself.
 */
final class MeshCompareCheck {
    static final boolean ENABLED = Boolean.parseBoolean(BenchmarkClientGameTest.prop("meshCheck", "false"));
    private static final Logger LOGGER = LoggerFactory.getLogger("VulkanMod Mesh Check");
    private static final int RADIUS = 4;

    private MeshCompareCheck() {
    }

    static void run(ClientGameTestContext context) {
        context.runOnClient(mc -> {
            SectionCompiler compiler = ((SectionRenderDispatcherAccessor) mc.levelRenderer.sectionRenderDispatcher()).vulkanmod$getSectionCompiler();
            SectionPos center = SectionPos.of(mc.player.blockPosition());
            boolean tint = Initializer.CONFIG.biomeBlendCache;
            boolean light = Initializer.CONFIG.sectionLightCache;
            boolean states = Initializer.CONFIG.sectionStateCache;
            int sections = 0, quads = 0, mismatches = 0;
            try (SectionBufferBuilderPack buffers = new SectionBufferBuilderPack()) {
                for (int dx = -RADIUS; dx <= RADIUS; dx++) {
                    for (int dz = -RADIUS; dz <= RADIUS; dz++) {
                        for (int dy = -4; dy <= 1; dy++) {
                            SectionPos pos = center.offset(dx, dy, dz);
                            Map<ChunkSectionLayer, byte[]> fast = compile(mc, compiler, buffers, pos, true);
                            Map<ChunkSectionLayer, byte[]> vanilla = compile(mc, compiler, buffers, pos, false);
                            sections++;
                            for (ChunkSectionLayer layer : ChunkSectionLayer.values()) {
                                byte[] a = fast.get(layer), b = vanilla.get(layer);
                                if (b != null) {
                                    quads += b.length / layer.vertexFormat().getVertexSize() / 4;
                                }
                                if (!java.util.Arrays.equals(a, b) && mismatches++ < 10) {
                                    LOGGER.error("Mesh mismatch in section {} layer {}: {} vs {} bytes", pos, layer.label(),
                                            a == null ? -1 : a.length, b == null ? -1 : b.length);
                                }
                            }
                        }
                    }
                }
            } finally {
                Initializer.CONFIG.biomeBlendCache = tint;
                Initializer.CONFIG.sectionLightCache = light;
                Initializer.CONFIG.sectionStateCache = states;
            }
            LOGGER.info("Mesh check: {} sections ({} quads) compiled twice, {} mismatching layers", sections, quads, mismatches);
            if (mismatches > 0) {
                throw new AssertionError(mismatches + " mismatching section layers");
            }
        });
    }

    private static Map<ChunkSectionLayer, byte[]> compile(Minecraft mc, SectionCompiler compiler, SectionBufferBuilderPack buffers,
                                                          SectionPos pos, boolean optimised) {
        Initializer.CONFIG.biomeBlendCache = optimised;
        Initializer.CONFIG.sectionLightCache = optimised;
        Initializer.CONFIG.sectionStateCache = optimised;
        mc.level.clearTintCaches();
        var region = new RenderRegionCache().createRegion(mc.level, pos.asLong());
        SectionCompiler.Results results = compiler.compile(pos, region, VertexSorting.byDistance(0, 0, 0), buffers);
        Map<ChunkSectionLayer, byte[]> meshes = new EnumMap<>(ChunkSectionLayer.class);
        try {
            for (Map.Entry<ChunkSectionLayer, MeshData> entry : results.renderedLayers.entrySet()) {
                ByteBuffer vertices = entry.getValue().vertexBuffer();
                byte[] bytes = new byte[vertices.remaining()];
                vertices.duplicate().get(bytes);
                meshes.put(entry.getKey(), bytes);
            }
        } finally {
            results.release();
            buffers.clearAll();
        }
        return meshes;
    }
}
