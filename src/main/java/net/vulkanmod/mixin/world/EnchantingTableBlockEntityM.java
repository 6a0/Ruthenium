package net.vulkanmod.mixin.world;

import net.minecraft.world.level.block.entity.EnchantingTableBlockEntity;
import net.vulkanmod.interfaces.CachedBlockEntityLight;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(EnchantingTableBlockEntity.class)
public class EnchantingTableBlockEntityM implements CachedBlockEntityLight {
    /** 0 until the light is first cached */
    @Unique private int vulkanmod$lightStamp;
    @Unique private int vulkanmod$light;

    @Override
    public int vulkanmod$getLightStamp() {
        return this.vulkanmod$lightStamp;
    }

    @Override
    public int vulkanmod$getLight() {
        return this.vulkanmod$light;
    }

    @Override
    public void vulkanmod$setLight(int stamp, int light) {
        this.vulkanmod$lightStamp = stamp;
        this.vulkanmod$light = light;
    }
}
