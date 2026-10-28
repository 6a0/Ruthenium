package net.thallium.gametest;

import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Diagnostic: with -Dthallium.bench.probeSeconds=N (and without client gametest mode), quits the game N seconds
 * after the first client tick. Used to check startup stability outside the gametest framework.
 */
public class StartupProbe implements ClientModInitializer {
    private static final Logger LOGGER = LoggerFactory.getLogger("Thallium Benchmark");

    @Override
    public void onInitializeClient() {
        String seconds = System.getProperty("thallium.bench.probeSeconds");
        if (seconds == null) {
            return;
        }

        long durationNanos = (long) (Double.parseDouble(seconds) * 1.0e9);
        long[] start = {0};

        ClientTickEvents.END_CLIENT_TICK.register(client -> {
            if (start[0] == 0) {
                start[0] = System.nanoTime();
            } else if (System.nanoTime() - start[0] > durationNanos) {
                LOGGER.info("Startup probe finished; stopping");
                client.stop();
            }
        });
    }
}
