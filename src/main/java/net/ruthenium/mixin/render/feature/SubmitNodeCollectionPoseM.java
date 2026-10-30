package net.ruthenium.mixin.render.feature;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.renderer.SubmitNodeCollection;
import net.ruthenium.render.feature.SubmitPosePool;
import org.joml.Matrix4f;
import org.joml.Matrix4fc;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/**
 * Submit poses and matrices from {@link SubmitPosePool} instead of a new copy per submit, in vanilla's submit methods of
 * the collection. The Fabric Renderer API's mesh submits (methods its mixin merges into the class) are not reached:
 * MixinExtras' injectors don't target merged methods, so they keep copying.
 */
@Mixin(value = SubmitNodeCollection.class, priority = 1100)
public class SubmitNodeCollectionPoseM {
    @WrapOperation(method = {"submitFlame", "submitModel", "submitCrumblingOverlay", "submitBlockModel", "submitBreakingBlockModel",
            "submitShapeOutline", "submitItem", "submitCustomGeometry"}, require = 0, at = @At(value = "INVOKE",
            target = "Lcom/mojang/blaze3d/vertex/PoseStack$Pose;copy()Lcom/mojang/blaze3d/vertex/PoseStack$Pose;"))
    private PoseStack.Pose pooledPose(PoseStack.Pose pose, Operation<PoseStack.Pose> original) {
        return SubmitPosePool.copy(pose);
    }

    @WrapOperation(method = {"submitShadow", "submitNameTag", "submitTextPart", "submitLeash", "submitMovingBlock"}, require = 0,
            at = @At(value = "NEW", target = "(Lorg/joml/Matrix4fc;)Lorg/joml/Matrix4f;"))
    private Matrix4f pooledMatrix(Matrix4fc matrix, Operation<Matrix4f> original) {
        return SubmitPosePool.copy(matrix);
    }
}
