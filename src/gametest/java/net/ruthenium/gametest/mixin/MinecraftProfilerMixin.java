package net.ruthenium.gametest.mixin;

import net.minecraft.client.Minecraft;
import net.minecraft.util.profiling.InactiveProfiler;
import net.minecraft.util.profiling.ProfilerFiller;
import net.ruthenium.gametest.LightProfiler;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(Minecraft.class)
public class MinecraftProfilerMixin {

    /** Installs {@link LightProfiler} for each loop iteration unless a vanilla profiler is in use */
    @Inject(method = "constructProfiler", at = @At("RETURN"), cancellable = true)
    private void useLightProfiler(CallbackInfoReturnable<ProfilerFiller> cir) {
        if (LightProfiler.enabled() && cir.getReturnValue() == InactiveProfiler.INSTANCE) {
            cir.setReturnValue(LightProfiler.INSTANCE);
        }
    }
}
