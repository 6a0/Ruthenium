package net.vulkanmod.interfaces;

/**
 * The light of a block entity (and that its block state is valid for it), cached by
 * {@link net.vulkanmod.render.blockentity.InstancedModels} for as long as the stamp is current.
 */
public interface CachedBlockEntityLight {
    int vulkanmod$getLightStamp();

    int vulkanmod$getLight();

    void vulkanmod$setLight(int stamp, int light);
}
