package net.thallium.mixin.render.feature;

import com.mojang.renderpearl.api.commands.RenderPass;
import net.minecraft.client.renderer.feature.FeatureFrameContext;
import net.minecraft.client.renderer.feature.FeatureRendererMap;
import net.minecraft.client.renderer.feature.submit.SubmitNode;
import net.minecraft.client.renderer.oit.OitStage;
import net.thallium.render.entity.InstancedParts;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.List;

/**
 * Records the {@link InstancedParts} draws a feature renderer group adds while it is prepared, and draws them right
 * after the group's own draws.
 */
@Mixin(targets = "net.minecraft.client.renderer.feature.FeatureRenderDispatcher$PreparedGroup")
public class PreparedGroupM {
    @Shadow @Final private boolean strictlyOrdered;

    @Unique private int firstPartDraw;
    @Unique private int partDrawEnd;

    @Inject(method = "prepare", at = @At("HEAD"))
    private void beginParts(CallbackInfo ci) {
        this.firstPartDraw = InstancedParts.beginGroup(this.strictlyOrdered);
    }

    @Inject(method = "prepare", at = @At("RETURN"))
    private void endParts(CallbackInfo ci) {
        this.partDrawEnd = InstancedParts.endGroup();
    }

    @Inject(method = "execute", at = @At("RETURN"))
    private void drawParts(FeatureFrameContext context, @Nullable OitStage stage, RenderPass renderPass, FeatureRendererMap featureRenderers,
                           List<SubmitNode> submits, CallbackInfo ci) {
        if (stage == null && this.partDrawEnd > this.firstPartDraw) {
            InstancedParts.draw(renderPass, this.firstPartDraw, this.partDrawEnd);
        }
    }
}
