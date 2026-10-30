package net.ruthenium.mixin.startup;

import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.PreferredGraphicsApi;
import net.ruthenium.Initializer;
import net.ruthenium.config.Config;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Vulkan by default: the first time the mod sees the options (also without an {@code options.txt}), a graphics API left
 * at Default (OpenGL first) becomes Vulkan. Only once, so vanilla's fallback stays: after a start that didn't finish,
 * Vulkan is reset to Default and the next start uses OpenGL. An API the player chose is left as it is.
 */
@Mixin(Options.class)
public class OptionsGraphicsApiM {
    @Shadow @Final private OptionInstance<PreferredGraphicsApi> preferredGraphicsBackend;
    @Shadow private PreferredGraphicsApi preferredGraphicsBackendFromStartup;

    @Inject(method = "load", at = @At("RETURN"))
    private void ruthenium$vulkanByDefault(CallbackInfo ci) {
        Config config = Initializer.config();
        if (config.vulkanByDefault) {
            return;
        }

        if (this.preferredGraphicsBackend.get() == PreferredGraphicsApi.DEFAULT) {
            this.preferredGraphicsBackend.set(PreferredGraphicsApi.VULKAN);
            this.preferredGraphicsBackendFromStartup = PreferredGraphicsApi.VULKAN;
            Initializer.LOGGER.info("Graphics API: Default changed to Vulkan");
        }
        config.vulkanByDefault = true;
        config.write();
    }
}
