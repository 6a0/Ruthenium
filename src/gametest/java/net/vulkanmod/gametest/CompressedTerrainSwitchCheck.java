package net.vulkanmod.gametest;

import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.vulkanmod.config.gui.OptionBlock;
import net.vulkanmod.config.option.Option;
import net.vulkanmod.config.option.Options;
import net.vulkanmod.render.chunk.CompressedTerrain;

/**
 * Switches VulkanMod's compressed terrain vertices off and on through the settings option while in the terrain world
 * ({@code -Pbench.terrainSwitchCheck=true}), checking the solid layer's vertex format and taking a screenshot after
 * each switch, to compare with the benchmark's static screenshot.
 */
final class CompressedTerrainSwitchCheck {
    private static final String OPTION_KEY = "vulkanmod.options.compressedTerrain";

    private CompressedTerrainSwitchCheck() {
    }

    static void run(ClientGameTestContext context, String label, String runId) {
        for (int i = 0; i < 2; i++) {
            set(context, false);
            context.takeScreenshot(label + "-terrain-vanillaformat" + i + "-" + runId);
            set(context, true);
            context.takeScreenshot(label + "-terrain-compressedformat" + i + "-" + runId);
        }
    }

    @SuppressWarnings("unchecked")
    private static void set(ClientGameTestContext context, boolean compressed) {
        context.runOnClient(_ -> {
            Option<Boolean> option = (Option<Boolean>) find();
            option.setNewValue(compressed);
            option.apply();
        });
        context.waitTicks(5);
        TerrainBenchmark.waitForVisibleCompiled(context);
        // Recompiled sections fade in again (chunk fade-in time)
        context.waitTicks(3 * 20);
        boolean active = context.computeOnClient(_ -> ChunkSectionLayer.SOLID.vertexFormat() == CompressedTerrain.FORMAT);
        boolean vanilla = context.computeOnClient(_ -> ChunkSectionLayer.SOLID.vertexFormat() == DefaultVertexFormat.BLOCK);
        if (active != compressed || vanilla == compressed) {
            throw new AssertionError("Solid terrain vertex format after switching compressed terrain to " + compressed
                    + ": compressed " + active + ", vanilla " + vanilla);
        }
    }

    private static Option<?> find() {
        for (OptionBlock block : Options.getOptimizationOpts()) {
            for (Option<?> option : block.options()) {
                if (option.getName().getContents() instanceof TranslatableContents contents && contents.getKey().equals(OPTION_KEY)) {
                    return option;
                }
            }
        }
        throw new AssertionError("No " + OPTION_KEY + " option");
    }
}
