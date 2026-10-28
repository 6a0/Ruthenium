package net.rubidium.mixin.render.entity.model;

import net.minecraft.client.model.Model;
import net.rubidium.interfaces.LayeredModel;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(Model.class)
public class ModelM implements LayeredModel {
    @Unique private boolean layered;

    @Override
    public boolean rubidium$isLayered() {
        return this.layered;
    }

    @Override
    public void rubidium$setLayered() {
        this.layered = true;
    }
}
