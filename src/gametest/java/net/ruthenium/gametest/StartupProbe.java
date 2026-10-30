package net.ruthenium.gametest;

import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.TitleScreen;
import org.apache.commons.io.FileUtils;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.lang.management.ManagementFactory;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Diagnostic: with -Druthenium.bench.probeSeconds=N (and without client gametest mode), quits the game N seconds
 * after the first client tick. Used to check startup stability outside the gametest framework.
 * <p>
 * With {@code -Pbench.probeJoins=K} as well, it measures load times as in real play, without the client gametest's
 * thread phases (which park the integrated server between ticks instead of letting it run chunk tasks): the uptime
 * at the first usable title screen, then K joins of a fresh copy of the terrain scene's world at render distance
 * {@code bench.footprintRenderDistance} (12), each timed until the world is open and until its chunks stop arriving
 * with every section compiled, then it quits.
 */
public class StartupProbe implements ClientModInitializer {
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Benchmark");
    private static final int TICKS_PER_SECOND = 20;

    @Override
    public void onInitializeClient() {
        String seconds = System.getProperty("ruthenium.bench.probeSeconds");
        if (seconds == null) {
            return;
        }

        long durationNanos = (long) (Double.parseDouble(seconds) * 1.0e9);
        long[] start = {0};
        int joins = Integer.getInteger("ruthenium.bench.probeJoins", 0);
        JoinProbe joinProbe = joins > 0 ? new JoinProbe(joins) : null;

        ClientTickEvents.END_CLIENT_TICK.register(client -> {
            if (start[0] == 0) {
                start[0] = System.nanoTime();
            } else if (System.nanoTime() - start[0] > durationNanos || joinProbe != null && joinProbe.finished) {
                LOGGER.info("Startup probe finished; stopping");
                client.stop();
                return;
            }
            if (joinProbe != null) {
                joinProbe.tick(client);
            }
        });
    }

    private static long uptimeMs() {
        return ManagementFactory.getRuntimeMXBean().getUptime();
    }

    private static final class JoinProbe {
        private final int joins;
        private final int renderDistance = Integer.parseInt(System.getProperty("ruthenium.bench.footprintRenderDistance", "12"));
        private final String worldName = "terrain-" + System.getProperty("ruthenium.bench.terrainSeed", "8279265");
        private int join;
        private boolean titleSeen;
        private boolean joining;
        private boolean open;
        private long startMs;
        private int ticks;
        private int lastLoaded = -1;
        private int stableTicks;
        private int idleTicks;
        boolean finished;

        JoinProbe(int joins) {
            this.joins = joins;
        }

        void tick(Minecraft mc) {
            if (!this.joining) {
                // After a join, whatever screen leaving shows once the integrated server is gone
                boolean ready = this.join == 0 ? mc.gui.screen() instanceof TitleScreen : mc.getSingleplayerServer() == null;
                if (mc.gui.overlay() != null || !ready || mc.level != null) {
                    this.idleTicks = 0;
                    return;
                }
                if (!this.titleSeen) {
                    this.titleSeen = true;
                    LOGGER.info("Join probe: title screen usable at uptime {} ms", uptimeMs());
                }
                // A second at the title screen between joins, as the footprint scene waits after leaving
                if (++this.idleTicks < TICKS_PER_SECOND) {
                    return;
                }
                if (this.join == this.joins) {
                    this.finished = true;
                    return;
                }
                this.startJoin(mc);
                return;
            }
            this.ticks++;
            if (!this.open && mc.level != null && mc.player != null && mc.gui.screen() == null) {
                this.open = true;
                LOGGER.info("Join probe: join {} open after {} ms", this.join, uptimeMs() - this.startMs);
            }
            if (!this.open) {
                return;
            }
            int loaded = mc.level.getChunkSource().getLoadedChunksCount();
            if (this.ticks % 10 == 0) {
                LOGGER.info("Join probe: {} ms, {} chunks loaded, all compiled {}", uptimeMs() - this.startMs, loaded,
                        mc.levelRenderer.hasRenderedAllSections());
            }
            int expected = (int) (Math.PI * this.renderDistance * this.renderDistance * 0.9);
            boolean done = loaded == this.lastLoaded && loaded >= expected && mc.levelRenderer.hasRenderedAllSections();
            this.stableTicks = done ? this.stableTicks + 1 : 0;
            this.lastLoaded = loaded;
            if (this.stableTicks >= TICKS_PER_SECOND) {
                LOGGER.info("Join probe: join {} loaded and compiled {} chunks after {} ms", this.join, loaded,
                        uptimeMs() - this.startMs - 1000);
                this.joining = false;
                this.idleTicks = 0;
                mc.disconnectWithProgressScreen();
            }
        }

        private void startJoin(Minecraft mc) {
            this.join++;
            mc.options.renderDistance().set(this.renderDistance);
            Path template = Path.of("..", "worlds", this.worldName);
            Path save = mc.getLevelSource().getBaseDir().resolve(this.worldName);
            try {
                if (!Files.isDirectory(template)) {
                    throw new IllegalStateException("No terrain world at " + template.toAbsolutePath());
                }
                FileUtils.deleteDirectory(save.toFile());
                FileUtils.copyDirectory(template.toFile(), save.toFile());
            } catch (IOException e) {
                throw new RuntimeException("Failed to copy the terrain world", e);
            }
            this.joining = true;
            this.open = false;
            this.ticks = 0;
            this.lastLoaded = -1;
            this.stableTicks = 0;
            this.startMs = uptimeMs();
            LOGGER.info("Join probe: join {} starts at uptime {} ms", this.join, this.startMs);
            mc.createWorldOpenFlows().openWorld(this.worldName, () -> LOGGER.error("Join probe: join cancelled"));
        }
    }
}
