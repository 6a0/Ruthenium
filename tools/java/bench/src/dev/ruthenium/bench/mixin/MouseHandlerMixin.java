package dev.ruthenium.bench.mixin;

import dev.ruthenium.bench.Bench;
import net.minecraft.client.MouseHandler;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * During a benchmark, the game never captures the cursor, so the rest of the desktop stays usable
 * while a run is going. (Input to the game is dropped anyway; see {@link SDLEventHandlerMixin}.)
 */
@Mixin(MouseHandler.class)
public abstract class MouseHandlerMixin {
    @Inject(method = "grabMouse", at = @At("HEAD"), cancellable = true)
    private void ruthenium$noGrab(CallbackInfo ci) {
        if (Bench.isActive()) {
            ci.cancel();
        }
    }
}
