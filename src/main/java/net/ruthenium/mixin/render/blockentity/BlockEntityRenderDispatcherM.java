package net.ruthenium.mixin.render.blockentity;

import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.server.packs.resources.ResourceManager;
import net.ruthenium.render.blockentity.InstancedModels;
import net.ruthenium.render.entity.InstancedParts;
import net.ruthenium.render.blockentity.RetainedBlockEntities;
import net.ruthenium.render.entity.BlockModelMeshCache;
import net.ruthenium.render.entity.ItemBoundingBoxes;
import net.ruthenium.render.entity.PlayerPoseDraws;
import net.ruthenium.render.text.RetainedLabels;
import net.ruthenium.render.entity.TransparentFaces;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Renderers, models and sprites are replaced on resource reload */
@Mixin(BlockEntityRenderDispatcher.class)
public class BlockEntityRenderDispatcherM {
    @Inject(method = "onResourceManagerReload", at = @At("RETURN"))
    private void clearRetained(ResourceManager resourceManager, CallbackInfo ci) {
        RetainedBlockEntities.clear();
        InstancedModels.clear();
        InstancedParts.clear();
        PlayerPoseDraws.clear();
        RetainedLabels.clear();
        BlockModelMeshCache.clear();
        ItemBoundingBoxes.clear();
        TransparentFaces.clear();
    }
}
