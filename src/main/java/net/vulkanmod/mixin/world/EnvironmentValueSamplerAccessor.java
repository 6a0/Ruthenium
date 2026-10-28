package net.vulkanmod.mixin.world;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import java.util.List;

@Mixin(targets = "net.minecraft.world.attribute.EnvironmentAttributeSystem$ValueSampler")
public interface EnvironmentValueSamplerAccessor {
    @Accessor("layers")
    List<?> vulkanmod$layers();
}
