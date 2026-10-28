package net.rubidium.mixin.world;

import net.minecraft.world.level.block.entity.EnchantingTableBlockEntity;
import net.rubidium.interfaces.CachedBlockEntityLight;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(EnchantingTableBlockEntity.class)
public class EnchantingTableBlockEntityM implements CachedBlockEntityLight {
    /** 0 until the light is first cached */
    @Unique private int rubidium$lightStamp;
    @Unique private int rubidium$light;

    @Override
    public int rubidium$getLightStamp() {
        return this.rubidium$lightStamp;
    }

    @Override
    public int rubidium$getLight() {
        return this.rubidium$light;
    }

    @Override
    public void rubidium$setLight(int stamp, int light) {
        this.rubidium$lightStamp = stamp;
        this.rubidium$light = light;
    }
}
