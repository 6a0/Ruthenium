package net.thallium.mixin.world;

import net.minecraft.world.level.block.entity.EnchantingTableBlockEntity;
import net.thallium.interfaces.CachedBlockEntityLight;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(EnchantingTableBlockEntity.class)
public class EnchantingTableBlockEntityM implements CachedBlockEntityLight {
    /** 0 until the light is first cached */
    @Unique private int thallium$lightStamp;
    @Unique private int thallium$light;

    @Override
    public int thallium$getLightStamp() {
        return this.thallium$lightStamp;
    }

    @Override
    public int thallium$getLight() {
        return this.thallium$light;
    }

    @Override
    public void thallium$setLight(int stamp, int light) {
        this.thallium$lightStamp = stamp;
        this.thallium$light = light;
    }
}
