package net.ruthenium.mixin.render.entity.model;

import net.minecraft.client.model.Model;
import net.ruthenium.interfaces.LayeredModel;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(Model.class)
public class ModelM implements LayeredModel {
    @Unique private boolean layered;

    @Override
    public boolean ruthenium$isLayered() {
        return this.layered;
    }

    @Override
    public void ruthenium$setLayered() {
        this.layered = true;
    }
}
