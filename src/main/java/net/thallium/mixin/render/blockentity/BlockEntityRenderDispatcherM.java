package net.thallium.mixin.render.blockentity;

import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.server.packs.resources.ResourceManager;
import net.thallium.render.blockentity.InstancedModels;
import net.thallium.render.entity.InstancedParts;
import net.thallium.render.blockentity.RetainedBlockEntities;
import net.thallium.render.entity.BlockModelMeshCache;
import net.thallium.render.entity.ItemBoundingBoxes;
import net.thallium.render.entity.TransparentFaces;
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
        BlockModelMeshCache.clear();
        ItemBoundingBoxes.clear();
        TransparentFaces.clear();
    }
}
