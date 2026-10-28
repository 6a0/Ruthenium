package net.ruthenium.gametest;

import net.minecraft.world.entity.animal.parrot.Parrot;
import org.jspecify.annotations.Nullable;

/**
 * Shoulder parrots for client mannequins in the players scene ({@link PlayerBenchmark}). A remote player's parrots
 * come from its synced shoulder data; vanilla's {@code ClientMannequin} never has any, so the benchmark sets them on
 * the client entity ({@code ClientMannequinMixin}) to cover {@code ParrotOnShoulderLayer} as on players.
 */
public interface BenchParrots {
    void ruthenium$setParrots(Parrot.@Nullable Variant left, Parrot.@Nullable Variant right);
}
