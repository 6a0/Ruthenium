package net.ruthenium.interfaces;

import net.minecraft.world.level.Level;
import net.minecraft.world.level.LightLayer;
import net.ruthenium.render.entity.EntityLight;

/** An entity's light probe brightness kept by {@link EntityLight} */
public interface CachedEntityLight {
    /** The kept brightness of the layer at the position (packed {@code BlockPos}), or -1 if none is kept for it */
    int ruthenium$light(Level level, LightLayer layer, long pos, int generation);

    void ruthenium$setLight(LightLayer layer, int light);
}
