package net.ruthenium.mixin.render.feature;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.renderer.SubmitNodeCollection;
import net.ruthenium.render.feature.SubmitPosePool;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/**
 * Submit poses from {@link SubmitPosePool} instead of a new copy per submit, in every method of the collection, the
 * Fabric Renderer API's submits included (applied after its mixin, priority)
 */
@Mixin(value = SubmitNodeCollection.class, priority = 1100)
public class SubmitNodeCollectionPoseM {
    @WrapOperation(method = "*", require = 0, at = @At(value = "INVOKE",
            target = "Lcom/mojang/blaze3d/vertex/PoseStack$Pose;copy()Lcom/mojang/blaze3d/vertex/PoseStack$Pose;"))
    private PoseStack.Pose pooledPose(PoseStack.Pose pose, Operation<PoseStack.Pose> original) {
        return SubmitPosePool.copy(pose);
    }
}
