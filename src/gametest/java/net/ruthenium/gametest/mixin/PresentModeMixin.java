package net.ruthenium.gametest.mixin;

import com.mojang.renderpearl.api.device.GpuSurface;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import java.util.Collection;

/**
 * Diagnostic ({@code -Pbench.presentMode=IMMEDIATE|MAILBOX|FIFO|FIFO_RELAXED}): the swapchain's present mode, when supported.
 * A comma-separated list gives the modes of the first configurations in turn (vanilla's choice after the list); every
 * configuration is logged.
 */
@Mixin(value = GpuSurface.PresentMode.class, remap = false)
public class PresentModeMixin {
    @Inject(method = "getSupportedVsyncMode", at = @At("HEAD"), cancellable = true)
    private static void benchPresentMode(Collection<GpuSurface.PresentMode> supported, boolean vsync,
                                         CallbackInfoReturnable<GpuSurface.PresentMode> cir) {
        String list = System.getProperty("ruthenium.bench.presentMode");
        String[] modes = list == null ? new String[0] : list.split(",");
        int call = net.ruthenium.gametest.PresentModeState.calls++;
        String mode = call < modes.length ? modes[call] : null;
        if (mode == null) {
            org.slf4j.LoggerFactory.getLogger("Ruthenium Benchmark").info("Surface configuration {}: vanilla's choice (vsync {})", call, vsync);
        } else {
            GpuSurface.PresentMode wanted = GpuSurface.PresentMode.valueOf(mode);
            org.slf4j.LoggerFactory.getLogger("Ruthenium Benchmark").info("Surface configuration {}: present modes {}, using {}", call, supported,
                    supported.contains(wanted) ? wanted : "vanilla's choice");
            if (supported.contains(wanted)) {
                cir.setReturnValue(wanted);
            }
        }
    }
}
