package net.ruthenium.mixin.render.entity.model;

import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.object.armorstand.ArmorStandModel;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

/** The base plate part, which {@code setupAnim} turns by the armor stand's yaw */
@Mixin(ArmorStandModel.class)
public interface ArmorStandModelAccessor {
    @Accessor("basePlate")
    ModelPart ruthenium$basePlate();
}
