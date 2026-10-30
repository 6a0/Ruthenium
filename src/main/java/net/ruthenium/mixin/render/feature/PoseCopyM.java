package net.ruthenium.mixin.render.feature;

import com.mojang.blaze3d.vertex.PoseStack;
import net.ruthenium.render.feature.SubmitPosePool;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/** A submit's pose copy that {@link SubmitPosePool#poolNextCopy} marked comes from the pool (the same values) */
@Mixin(PoseStack.Pose.class)
public class PoseCopyM {
    @Inject(method = "copy", at = @At("HEAD"), cancellable = true)
    private void pooledCopy(CallbackInfoReturnable<PoseStack.Pose> cir) {
        if (SubmitPosePool.takeNextCopy()) {
            cir.setReturnValue(SubmitPosePool.copy((PoseStack.Pose) (Object) this));
        }
    }
}
