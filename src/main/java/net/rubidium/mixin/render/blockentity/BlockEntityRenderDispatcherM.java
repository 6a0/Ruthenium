package net.rubidium.mixin.render.blockentity;

import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.server.packs.resources.ResourceManager;
import net.rubidium.render.blockentity.InstancedModels;
import net.rubidium.render.entity.InstancedParts;
import net.rubidium.render.blockentity.RetainedBlockEntities;
import net.rubidium.render.entity.BlockModelMeshCache;
import net.rubidium.render.entity.ItemBoundingBoxes;
import net.rubidium.render.entity.TransparentFaces;
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
