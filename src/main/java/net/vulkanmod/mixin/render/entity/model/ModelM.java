package net.vulkanmod.mixin.render.entity.model;

import net.minecraft.client.model.Model;
import net.vulkanmod.interfaces.LayeredModel;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(Model.class)
public class ModelM implements LayeredModel {
    @Unique private boolean layered;

    @Override
    public boolean vulkanmod$isLayered() {
        return this.layered;
    }

    @Override
    public void vulkanmod$setLayered() {
        this.layered = true;
    }
}
