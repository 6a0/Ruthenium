package net.thallium.interfaces;

/**
 * The light of a block entity (and that its block state is valid for it), cached by
 * {@link net.thallium.render.blockentity.InstancedModels} for as long as the stamp is current.
 */
public interface CachedBlockEntityLight {
    int thallium$getLightStamp();

    int thallium$getLight();

    void thallium$setLight(int stamp, int light);
}
