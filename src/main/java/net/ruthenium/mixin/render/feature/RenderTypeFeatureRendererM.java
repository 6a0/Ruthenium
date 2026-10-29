package net.ruthenium.mixin.render.feature;

import com.llamalad7.mixinextras.sugar.Local;
import com.mojang.renderpearl.api.commands.RenderPass;
import net.minecraft.client.renderer.StagedVertexBuffer;
import net.minecraft.client.renderer.feature.RenderTypeFeatureRenderer;
import net.minecraft.client.renderer.oit.OitStage;
import net.ruthenium.interfaces.LabelDrawHolder;
import net.ruthenium.interfaces.PosedDrawHolder;
import net.ruthenium.render.entity.PlayerPoseDraws;
import net.ruthenium.render.text.RetainedLabels;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/** A group's posed draws ({@link PlayerPoseDraws}) and label draws ({@link RetainedLabels}) are drawn in the place of their (empty) staged draws */
@Mixin(RenderTypeFeatureRenderer.class)
public class RenderTypeFeatureRendererM {
    /** A redirect, so that every draw executed by any group pays no wrapper object */
    @Redirect(method = "executeGroup", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/StagedVertexBuffer;getExecuteInfo(Lnet/minecraft/client/renderer/StagedVertexBuffer$Draw;)Lnet/minecraft/client/renderer/StagedVertexBuffer$ExecuteInfo;"))
    private StagedVertexBuffer.@Nullable ExecuteInfo drawPosed(StagedVertexBuffer buffer, StagedVertexBuffer.Draw draw,
                                                               @Local(argsOnly = true) RenderPass renderPass, @Local(argsOnly = true) @Nullable OitStage stage) {
        PlayerPoseDraws.PoseDraw posed = ((PosedDrawHolder) draw).ruthenium$poseDraw();
        if (posed != null && stage == null) {
            PlayerPoseDraws.draw(renderPass, posed);
            return null;
        }
        RetainedLabels.LabelDraw labels = ((LabelDrawHolder) draw).ruthenium$labelDraw();
        if (labels != null && stage == null) {
            RetainedLabels.draw(renderPass, labels);
            return null;
        }
        return buffer.getExecuteInfo(draw);
    }
}
