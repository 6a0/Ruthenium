package net.vulkanmod.mixin.render.feature;

import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.vulkanmod.render.entity.ModelCulling;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(ModelFeatureRenderer.class)
public class ModelFeatureRendererM {

    @Inject(method = "prepareModel", at = @At("HEAD"))
    private void beginModel(ModelFeatureRenderer.Submit<?> submit, CallbackInfo ci) {
        ModelCulling.beginModel(submit.renderType());
    }

    @Inject(method = "prepareModel", at = @At("RETURN"))
    private void endModel(ModelFeatureRenderer.Submit<?> submit, CallbackInfo ci) {
        ModelCulling.endModel();
    }
}
