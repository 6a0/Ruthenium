package net.ruthenium.gametest.mixin;

import net.minecraft.client.entity.ClientMannequin;
import net.minecraft.world.entity.animal.parrot.Parrot;
import net.ruthenium.gametest.BenchParrots;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Unique;

/**
 * Benchmark harness only: lets the players scene give client mannequins shoulder parrots ({@link BenchParrots}).
 * Without them set this returns null, as vanilla does. An overwrite, so that every run pays the same.
 */
@Mixin(ClientMannequin.class)
public class ClientMannequinMixin implements BenchParrots {
    @Unique private Parrot.@Nullable Variant ruthenium$leftParrot;
    @Unique private Parrot.@Nullable Variant ruthenium$rightParrot;

    /**
     * @author Ruthenium benchmark
     * @reason Shoulder parrots for the players scene
     */
    @Overwrite
    public Parrot.@Nullable Variant getParrotVariantOnShoulder(boolean left) {
        return left ? this.ruthenium$leftParrot : this.ruthenium$rightParrot;
    }

    @Override
    public void ruthenium$setParrots(Parrot.@Nullable Variant left, Parrot.@Nullable Variant right) {
        this.ruthenium$leftParrot = left;
        this.ruthenium$rightParrot = right;
    }
}
