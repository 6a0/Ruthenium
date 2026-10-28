package net.rubidium.mixin.world;

import net.minecraft.world.attribute.EnvironmentAttribute;
import net.minecraft.world.attribute.EnvironmentAttributeSystem;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import java.util.Map;

@Mixin(EnvironmentAttributeSystem.class)
public interface EnvironmentAttributeSystemAccessor {
    @Accessor("attributeSamplers")
    Map<EnvironmentAttribute<?>, ?> rubidium$attributeSamplers();
}
