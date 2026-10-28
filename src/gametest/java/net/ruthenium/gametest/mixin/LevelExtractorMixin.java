package net.ruthenium.gametest.mixin;

import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import net.ruthenium.gametest.FrameRecorder;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(LevelExtractor.class)
public class LevelExtractorMixin {
    @Shadow @Final private LevelRenderState levelRenderState;

    @Inject(method = "extract", at = @At("RETURN"))
    private void countExtracted(CallbackInfo ci) {
        FrameRecorder.onLevelExtracted(this.levelRenderState);
    }
}
