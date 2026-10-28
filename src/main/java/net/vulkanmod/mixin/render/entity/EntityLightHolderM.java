package net.vulkanmod.mixin.render.entity;

import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LightLayer;
import net.vulkanmod.interfaces.CachedEntityLight;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/** The kept light probe brightness of an entity ({@link net.vulkanmod.render.entity.EntityLight}) */
@Mixin(Entity.class)
public class EntityLightHolderM implements CachedEntityLight {
    @Unique private @Nullable Level vulkanmod$lightLevel;
    @Unique private long vulkanmod$lightPos;
    /** 0 (never a generation) until a value is kept */
    @Unique private int vulkanmod$lightGeneration;
    /** Brightness + 1, 0 when not kept */
    @Unique private byte vulkanmod$blockLight;
    @Unique private byte vulkanmod$skyLight;

    @Override
    public int vulkanmod$light(Level level, LightLayer layer, long pos, int generation) {
        if (generation != this.vulkanmod$lightGeneration || pos != this.vulkanmod$lightPos || level != this.vulkanmod$lightLevel) {
            this.vulkanmod$lightGeneration = generation;
            this.vulkanmod$lightPos = pos;
            this.vulkanmod$lightLevel = level;
            this.vulkanmod$blockLight = 0;
            this.vulkanmod$skyLight = 0;
            return -1;
        }
        return (layer == LightLayer.SKY ? this.vulkanmod$skyLight : this.vulkanmod$blockLight) - 1;
    }

    @Override
    public void vulkanmod$setLight(LightLayer layer, int light) {
        if (light < 0 || light > 126) {
            return;
        }
        if (layer == LightLayer.SKY) {
            this.vulkanmod$skyLight = (byte) (light + 1);
        } else {
            this.vulkanmod$blockLight = (byte) (light + 1);
        }
    }
}
