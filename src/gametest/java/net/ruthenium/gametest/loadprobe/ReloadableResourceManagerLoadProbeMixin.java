package net.ruthenium.gametest.loadprobe;

import com.llamalad7.mixinextras.injector.ModifyExpressionValue;
import net.minecraft.server.packs.resources.ReloadableResourceManager;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/**
 * Resource reloads use vanilla's profiled reload instance, which logs each listener's preparation and apply times
 * (normally only with debug logging).
 */
@Mixin(ReloadableResourceManager.class)
public class ReloadableResourceManagerLoadProbeMixin {
    @ModifyExpressionValue(method = "createReload", at = @At(value = "INVOKE", target = "Lorg/slf4j/Logger;isDebugEnabled()Z"))
    private boolean ruthenium$profile(boolean debug) {
        return true;
    }
}
