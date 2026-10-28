package net.rubidium.mixin.render.vertex;

import com.mojang.blaze3d.vertex.VertexSorting;
import net.rubidium.render.vertex.DistanceSorting;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(VertexSorting.class)
public interface VertexSortingM {

    @Inject(method = "byDistance(Lcom/mojang/blaze3d/vertex/VertexSorting$DistanceFunction;)Lcom/mojang/blaze3d/vertex/VertexSorting;",
            at = @At("HEAD"), cancellable = true)
    private static void fastByDistance(VertexSorting.DistanceFunction function, CallbackInfoReturnable<VertexSorting> cir) {
        cir.setReturnValue(DistanceSorting.byDistance(function));
    }
}
