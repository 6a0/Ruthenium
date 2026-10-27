package net.vulkanmod.mixin.render.blockentity;

import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.server.packs.resources.ResourceManager;
import net.vulkanmod.render.blockentity.InstancedModels;
import net.vulkanmod.render.entity.InstancedParts;
import net.vulkanmod.render.blockentity.RetainedBlockEntities;
import net.vulkanmod.render.entity.BlockModelMeshCache;
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
    }
}
