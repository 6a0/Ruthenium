package net.thallium.mixin.render.entity;

import it.unimi.dsi.fastutil.longs.LongSet;
import net.minecraft.client.multiplayer.ClientChunkCache;
import net.minecraft.world.level.chunk.LightChunkGetter;
import net.minecraft.world.level.lighting.LayerLightSectionStorage;
import net.thallium.render.entity.EntityLight;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** The client's light storages publish new light data: kept entity light is stale ({@link EntityLight}) */
@Mixin(LayerLightSectionStorage.class)
public class LayerLightSectionStorageM {
    @Shadow @Final protected LightChunkGetter chunkSource;
    @Shadow @Final protected LongSet changedSections;

    @Inject(method = "swapSectionMap", at = @At("HEAD"))
    private void thallium$invalidateEntityLight(CallbackInfo ci) {
        if (!this.changedSections.isEmpty() && this.chunkSource instanceof ClientChunkCache) {
            EntityLight.invalidate();
        }
    }
}
