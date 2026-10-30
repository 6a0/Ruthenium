package net.ruthenium.gametest;

import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.VertexSorting;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.SectionBufferBuilderPack;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.RenderRegionCache;
import net.minecraft.client.renderer.chunk.SectionCompiler;
import net.minecraft.core.SectionPos;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.chunk.DataLayer;
import net.ruthenium.Initializer;
import net.ruthenium.gametest.mixin.SectionRenderDispatcherAccessor;
import net.ruthenium.mixin.world.DataLayerAccessor;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.time.LocalDateTime;
import java.time.format.DateTimeFormatter;
import java.util.EnumMap;
import java.util.HexFormat;
import java.util.Map;

/**
 * {@code -Pbench.meshCheck=true}: compiles the sections around the player with the mesher optimisations
 * ({@code biomeBlendCache}, {@code sectionLightCache}, {@code sectionStateCache}) on and off and requires byte-identical meshes. Vanilla's
 * tint caches are cleared before each compile, so the "off" compile computes every tint itself.
 * <p>
 * It also writes a SHA-256 digest of every section's meshes (the "off" compile) to
 * {@code results/meshcheck/<compactSkyLight>-<time>.txt} and logs one digest over all of them, for comparing launches
 * whose difference is fixed when the world loads (switch {@code compactSkyLight}: how the client stores its light).
 */
final class MeshCompareCheck {
    static final boolean ENABLED = Boolean.parseBoolean(BenchmarkClientGameTest.prop("meshCheck", "false"));
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Mesh Check");
    private static final int RADIUS = 4;

    private MeshCompareCheck() {
    }

    static void run(ClientGameTestContext context) {
        context.runOnClient(mc -> {
            SectionCompiler compiler = ((SectionRenderDispatcherAccessor) mc.levelRenderer.sectionRenderDispatcher()).ruthenium$getSectionCompiler();
            SectionPos center = SectionPos.of(mc.player.blockPosition());
            boolean tint = Initializer.CONFIG.biomeBlendCache;
            boolean light = Initializer.CONFIG.sectionLightCache;
            boolean states = Initializer.CONFIG.sectionStateCache;
            int sections = 0, quads = 0, mismatches = 0;
            int skyNoArray = 0, skyArray = 0, skyMissing = 0;
            MessageDigest all = sha256();
            StringBuilder perSection = new StringBuilder();
            try (SectionBufferBuilderPack buffers = new SectionBufferBuilderPack()) {
                for (int dx = -RADIUS; dx <= RADIUS; dx++) {
                    for (int dz = -RADIUS; dz <= RADIUS; dz++) {
                        for (int dy = -4; dy <= 1; dy++) {
                            SectionPos pos = center.offset(dx, dy, dz);
                            Map<ChunkSectionLayer, byte[]> fast = compile(mc, compiler, buffers, pos, true);
                            Map<ChunkSectionLayer, byte[]> vanilla = compile(mc, compiler, buffers, pos, false);
                            sections++;
                            DataLayer sky = mc.level.getLightEngine().getLayerListener(LightLayer.SKY).getDataLayerData(pos);
                            if (sky == null) skyMissing++;
                            else if (((DataLayerAccessor) sky).ruthenium$data() == null) skyNoArray++;
                            else skyArray++;
                            MessageDigest one = sha256();
                            for (ChunkSectionLayer layer : ChunkSectionLayer.values()) {
                                byte[] b = vanilla.get(layer);
                                one.update((byte) layer.ordinal());
                                if (b != null) one.update(b);
                            }
                            byte[] sectionDigest = one.digest();
                            all.update(sectionDigest);
                            perSection.append(pos.x()).append(' ').append(pos.y()).append(' ').append(pos.z()).append(' ')
                                    .append(HexFormat.of().formatHex(sectionDigest)).append('\n');
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
            String digest = HexFormat.of().formatHex(all.digest());
            LOGGER.info("Mesh check digest {} (compactSkyLight {}; sky layers of these sections: {} without array, {} with, {} none)",
                    digest, Initializer.CONFIG.compactSkyLight, skyNoArray, skyArray, skyMissing);
            try {
                Path dir = Path.of(BenchmarkClientGameTest.prop("outDir", "../results")).resolve("meshcheck");
                Files.createDirectories(dir);
                String time = LocalDateTime.now().format(DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss"));
                Files.writeString(dir.resolve("compactSkyLight-" + Initializer.CONFIG.compactSkyLight + "-" + time + ".txt"),
                        "digest " + digest + "\n" + perSection);
            } catch (IOException e) {
                throw new RuntimeException("Failed to write mesh digests", e);
            }
            if (mismatches > 0) {
                throw new AssertionError(mismatches + " mismatching section layers");
            }
        });
    }

    private static MessageDigest sha256() {
        try {
            return MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
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
