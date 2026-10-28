package net.thallium.mixin.render.entity;

import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LightLayer;
import net.thallium.interfaces.CachedEntityLight;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/** The kept light probe brightness of an entity ({@link net.thallium.render.entity.EntityLight}) */
@Mixin(Entity.class)
public class EntityLightHolderM implements CachedEntityLight {
    @Unique private @Nullable Level thallium$lightLevel;
    @Unique private long thallium$lightPos;
    /** 0 (never a generation) until a value is kept */
    @Unique private int thallium$lightGeneration;
    /** Brightness + 1, 0 when not kept */
    @Unique private byte thallium$blockLight;
    @Unique private byte thallium$skyLight;

    @Override
    public int thallium$light(Level level, LightLayer layer, long pos, int generation) {
        if (generation != this.thallium$lightGeneration || pos != this.thallium$lightPos || level != this.thallium$lightLevel) {
            this.thallium$lightGeneration = generation;
            this.thallium$lightPos = pos;
            this.thallium$lightLevel = level;
            this.thallium$blockLight = 0;
            this.thallium$skyLight = 0;
            return -1;
        }
        return (layer == LightLayer.SKY ? this.thallium$skyLight : this.thallium$blockLight) - 1;
    }

    @Override
    public void thallium$setLight(LightLayer layer, int light) {
        if (light < 0 || light > 126) {
            return;
        }
        if (layer == LightLayer.SKY) {
            this.thallium$skyLight = (byte) (light + 1);
        } else {
            this.thallium$blockLight = (byte) (light + 1);
        }
    }
}
