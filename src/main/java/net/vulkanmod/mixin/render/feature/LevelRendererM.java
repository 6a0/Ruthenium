package net.vulkanmod.mixin.render.feature;

import net.minecraft.client.renderer.LevelRenderer;
import com.mojang.renderpearl.api.commands.RenderPass;
import net.minecraft.client.renderer.chunk.ChunkSectionsToRender;
import net.minecraft.client.renderer.feature.FeatureRenderDispatcher;
import net.vulkanmod.render.blockentity.InstancedModels;
import net.vulkanmod.render.blockentity.RetainedBlockEntities;
import net.vulkanmod.render.entity.InstancedParts;
import net.vulkanmod.render.entity.ModelPoseReplay;
import net.vulkanmod.render.entity.ModelCulling;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Marks the world's feature preparation and block entity submission for {@link ModelCulling}, and draws
 * {@link RetainedBlockEntities} and {@link InstancedModels} after the world's solid features.
 */
@Mixin(LevelRenderer.class)
public class LevelRendererM {
    private static final String PREPARE_FRAME = "Lnet/minecraft/client/renderer/feature/FeatureRenderDispatcher;prepareFrame(Lnet/minecraft/client/renderer/SubmitNodeStorage;)Lnet/minecraft/client/renderer/feature/FeatureRenderDispatcher$PreparedFrame;";

    @Inject(method = "render", at = @At(value = "INVOKE", target = PREPARE_FRAME))
    private void beginWorldFeatures(CallbackInfo ci) {
        ModelCulling.setWorldPass(true);
        InstancedParts.beginFrame();
        ModelPoseReplay.beginFrame();
    }

    @Inject(method = "render", at = @At(value = "INVOKE", target = PREPARE_FRAME, shift = At.Shift.AFTER))
    private void endWorldFeatures(CallbackInfo ci) {
        ModelCulling.setWorldPass(false);
        RetainedBlockEntities.prepareDraws();
        InstancedModels.prepareDraws();
        InstancedParts.upload();
    }

    @Inject(method = "executeSolid", at = @At(value = "INVOKE", shift = At.Shift.AFTER,
            target = "Lnet/minecraft/client/renderer/feature/FeatureRenderDispatcher$PreparedFrame;executeSolid(Lcom/mojang/renderpearl/api/commands/RenderPass;)V"))
    private void drawRetainedBlockEntities(ChunkSectionsToRender chunkSectionsToRender, FeatureRenderDispatcher.PreparedFrame featureFrame,
                                           RenderPass renderPass, CallbackInfo ci) {
        RetainedBlockEntities.draw(renderPass);
        InstancedModels.draw(renderPass);
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
