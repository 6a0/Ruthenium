package net.vulkanmod.mixin.render.feature;

import net.minecraft.client.renderer.LevelRenderer;
import net.vulkanmod.render.entity.ModelCulling;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Marks the world's feature preparation and block entity submission for {@link ModelCulling}.
 */
@Mixin(LevelRenderer.class)
public class LevelRendererM {
    private static final String PREPARE_FRAME = "Lnet/minecraft/client/renderer/feature/FeatureRenderDispatcher;prepareFrame(Lnet/minecraft/client/renderer/SubmitNodeStorage;)Lnet/minecraft/client/renderer/feature/FeatureRenderDispatcher$PreparedFrame;";

    @Inject(method = "render", at = @At(value = "INVOKE", target = PREPARE_FRAME))
    private void beginWorldFeatures(CallbackInfo ci) {
        ModelCulling.setWorldPass(true);
    }

    @Inject(method = "render", at = @At(value = "INVOKE", target = PREPARE_FRAME, shift = At.Shift.AFTER))
    private void endWorldFeatures(CallbackInfo ci) {
        ModelCulling.setWorldPass(false);
    }

    @Inject(method = "submitBlockEntities", at = @At("HEAD"))
    private void beginBlockEntities(CallbackInfo ci) {
        ModelCulling.setBlockEntitySubmit(true);
    }

    @Inject(method = "submitBlockEntities", at = @At("RETURN"))
    private void endBlockEntities(CallbackInfo ci) {
        ModelCulling.setBlockEntitySubmit(false);
    }
}
