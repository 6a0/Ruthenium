package net.ruthenium.mixin.render;

import com.mojang.renderpearl.api.commands.CommandEncoder;
import com.mojang.renderpearl.api.textures.GpuTextureView;
import com.mojang.renderpearl.frontend.FrontendGpuSurface;
import net.ruthenium.render.FrameClear;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** {@link FrameClear}: a deferred clear of the presented texture is performed before the swapchain blit */
@Mixin(FrontendGpuSurface.class)
public class FrontendGpuSurfaceM {
    @Inject(method = "blitFromTexture", at = @At("HEAD"))
    private void ruthenium$beforeBlit(CommandEncoder commandEncoder, GpuTextureView textureView, CallbackInfo ci) {
        FrameClear.beforeUse(textureView.texture());
    }
}
