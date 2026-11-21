package dev.ruthenium.bench.mixin;

import com.mojang.blaze3d.platform.SDLEventHandler;
import dev.ruthenium.bench.Bench;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * During a benchmark, drops the player's keyboard, text, mouse and file-drop events, so a stray
 * key or mouse movement can't turn the view or open a screen in the middle of a run. Window events
 * (focus, resize, close) still go through.
 */
@Mixin(SDLEventHandler.class)
public abstract class SDLEventHandlerMixin {
    @Inject(
            method = {
                "handleKeyEvent",
                "handleTextInputEvent",
                "handleTextEditingEvent",
                "handleMouseMotionEvent",
                "handleMouseButtonEvent",
                "handleMouseWheelEvent",
                "handleDropCompleteEvent"
            },
            at = @At("HEAD"),
            cancellable = true,
            require = 7)
    private void ruthenium$dropInput(CallbackInfo ci) {
        if (Bench.isActive()) {
            ci.cancel();
        }
    }
}
