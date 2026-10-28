package net.ruthenium.mixin.render.entity;

import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LightLayer;
import net.ruthenium.interfaces.CachedEntityLight;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/** The kept light probe brightness of an entity ({@link net.ruthenium.render.entity.EntityLight}) */
@Mixin(Entity.class)
public class EntityLightHolderM implements CachedEntityLight {
    @Unique private @Nullable Level ruthenium$lightLevel;
    @Unique private long ruthenium$lightPos;
    /** 0 (never a generation) until a value is kept */
    @Unique private int ruthenium$lightGeneration;
    /** Brightness + 1, 0 when not kept */
    @Unique private byte ruthenium$blockLight;
    @Unique private byte ruthenium$skyLight;

    @Override
    public int ruthenium$light(Level level, LightLayer layer, long pos, int generation) {
        if (generation != this.ruthenium$lightGeneration || pos != this.ruthenium$lightPos || level != this.ruthenium$lightLevel) {
            this.ruthenium$lightGeneration = generation;
            this.ruthenium$lightPos = pos;
            this.ruthenium$lightLevel = level;
            this.ruthenium$blockLight = 0;
            this.ruthenium$skyLight = 0;
            return -1;
        }
        return (layer == LightLayer.SKY ? this.ruthenium$skyLight : this.ruthenium$blockLight) - 1;
    }

    @Override
    public void ruthenium$setLight(LightLayer layer, int light) {
        if (light < 0 || light > 126) {
            return;
        }
        if (layer == LightLayer.SKY) {
            this.ruthenium$skyLight = (byte) (light + 1);
        } else {
            this.ruthenium$blockLight = (byte) (light + 1);
        }
    }
}
