package dev.ruthenium.bench;

import net.fabricmc.api.ClientModInitializer;

/** Starts the benchmark driver when cargo xtask bench asked for a scene. */
public final class BenchClient implements ClientModInitializer {
    @Override
    public void onInitializeClient() {
        Bench.init();
    }
}
