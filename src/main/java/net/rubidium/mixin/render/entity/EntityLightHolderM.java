package net.rubidium.mixin.render.entity;

import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LightLayer;
import net.rubidium.interfaces.CachedEntityLight;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/** The kept light probe brightness of an entity ({@link net.rubidium.render.entity.EntityLight}) */
@Mixin(Entity.class)
public class EntityLightHolderM implements CachedEntityLight {
    @Unique private @Nullable Level rubidium$lightLevel;
    @Unique private long rubidium$lightPos;
    /** 0 (never a generation) until a value is kept */
    @Unique private int rubidium$lightGeneration;
    /** Brightness + 1, 0 when not kept */
    @Unique private byte rubidium$blockLight;
    @Unique private byte rubidium$skyLight;

    @Override
    public int rubidium$light(Level level, LightLayer layer, long pos, int generation) {
        if (generation != this.rubidium$lightGeneration || pos != this.rubidium$lightPos || level != this.rubidium$lightLevel) {
            this.rubidium$lightGeneration = generation;
            this.rubidium$lightPos = pos;
            this.rubidium$lightLevel = level;
            this.rubidium$blockLight = 0;
            this.rubidium$skyLight = 0;
            return -1;
        }
        return (layer == LightLayer.SKY ? this.rubidium$skyLight : this.rubidium$blockLight) - 1;
    }

    @Override
    public void rubidium$setLight(LightLayer layer, int light) {
        if (light < 0 || light > 126) {
            return;
        }
        if (layer == LightLayer.SKY) {
            this.rubidium$skyLight = (byte) (light + 1);
        } else {
            this.rubidium$blockLight = (byte) (light + 1);
        }
    }
}
