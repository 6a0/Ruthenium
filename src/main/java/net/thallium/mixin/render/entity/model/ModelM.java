package net.thallium.mixin.render.entity.model;

import net.minecraft.client.model.Model;
import net.thallium.interfaces.LayeredModel;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(Model.class)
public class ModelM implements LayeredModel {
    @Unique private boolean layered;

    @Override
    public boolean thallium$isLayered() {
        return this.layered;
    }

    @Override
    public void thallium$setLayered() {
        this.layered = true;
    }
}
