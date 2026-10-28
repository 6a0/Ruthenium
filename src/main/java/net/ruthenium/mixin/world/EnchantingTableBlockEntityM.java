package net.ruthenium.mixin.world;

import net.minecraft.world.level.block.entity.EnchantingTableBlockEntity;
import net.ruthenium.interfaces.CachedBlockEntityLight;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(EnchantingTableBlockEntity.class)
public class EnchantingTableBlockEntityM implements CachedBlockEntityLight {
    /** 0 until the light is first cached */
    @Unique private int ruthenium$lightStamp;
    @Unique private int ruthenium$light;

    @Override
    public int ruthenium$getLightStamp() {
        return this.ruthenium$lightStamp;
    }

    @Override
    public int ruthenium$getLight() {
        return this.ruthenium$light;
    }

    @Override
    public void ruthenium$setLight(int stamp, int light) {
        this.ruthenium$lightStamp = stamp;
        this.ruthenium$light = light;
    }
}
