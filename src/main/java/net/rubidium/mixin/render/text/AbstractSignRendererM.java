package net.rubidium.mixin.render.text;

import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.gui.Font;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.blockentity.AbstractSignRenderer;
import net.minecraft.client.renderer.blockentity.state.SignRenderState;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.level.block.entity.SignText;
import net.rubidium.render.entity.ModelCulling;
import net.rubidium.render.text.SignTextCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/**
 * Measures sign lines through {@link SignTextCache}, which also registers them for the cached layout in
 * {@link TextFeatureRendererM}, and skips the text of sign sides facing away from the camera
 * ({@link ModelCulling#isSignTextHidden}).
 */
@Mixin(AbstractSignRenderer.class)
public abstract class AbstractSignRendererM<S extends SignRenderState> {

    @Shadow
    protected abstract void submitSignText(S state, PoseStack poseStack, SubmitNodeCollector submitNodeCollector, SignText signText);

    @Redirect(method = "submitSignText", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/gui/Font;width(Lnet/minecraft/util/FormattedCharSequence;)I"))
    private int cachedWidth(Font font, FormattedCharSequence line) {
        return SignTextCache.width(font, line);
    }

    @Redirect(method = "submit(Lnet/minecraft/client/renderer/blockentity/state/SignRenderState;Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;Lnet/minecraft/client/renderer/state/level/CameraRenderState;)V",
            at = @At(value = "INVOKE", target = "Lnet/minecraft/client/renderer/blockentity/AbstractSignRenderer;submitSignText(Lnet/minecraft/client/renderer/blockentity/state/SignRenderState;Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;Lnet/minecraft/world/level/block/entity/SignText;)V"))
    private void cullSignText(AbstractSignRenderer<S> renderer, S state, PoseStack poseStack, SubmitNodeCollector submitNodeCollector,
                              SignText signText) {
        if (!ModelCulling.isSignTextHidden(poseStack.last().pose())) {
            this.submitSignText(state, poseStack, submitNodeCollector, signText);
        }
    }
}
