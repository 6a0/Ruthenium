package net.ruthenium.mixin.render.entity.model;

import net.minecraft.client.model.Model;
import net.ruthenium.interfaces.LayeredModel;
import net.ruthenium.interfaces.PoseLayoutHolder;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(Model.class)
public class ModelM implements LayeredModel, PoseLayoutHolder {
    @Unique private boolean layered;
    @Unique private @Nullable Object poseLayout;

    @Override
    public boolean ruthenium$isLayered() {
        return this.layered;
    }

    @Override
    public void ruthenium$setLayered() {
        this.layered = true;
    }

    @Override
    public @Nullable Object ruthenium$poseLayout() {
        return this.poseLayout;
    }

    @Override
    public void ruthenium$setPoseLayout(@Nullable Object layout) {
        this.poseLayout = layout;
    }
}
