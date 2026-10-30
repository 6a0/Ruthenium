package net.ruthenium.mixin.render.chunk;

import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.ruthenium.Initializer;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Switch {@code freeUnusedChunkData}: leaving a world only flags {@code shouldResetLevelRenderData}, and the reset runs
 * in {@code extract}, i.e. on the first frame of the next world. Until then the level renderer's view area, visible
 * section lists and section dispatcher keep the old world's sections, their compile regions and through them its
 * chunks and {@code ClientLevel} reachable (~82 MB of heap and ~350 MB of GPU memory at render distance 12, the
 * twenty-ninth session). The reset now runs right away; the flag stays set, so the next world's first frame still
 * runs it (a no-op then), as vanilla does.
 */
@Mixin(LevelExtractor.class)
public class LevelExtractorResetM {
    @Shadow @Final private LevelRenderer levelRenderer;

    @Inject(method = "setLevel", at = @At("TAIL"))
    private void ruthenium$resetNow(@Nullable ClientLevel level, CallbackInfo ci) {
        if (level == null && Initializer.CONFIG.freeUnusedChunkData) {
            this.levelRenderer.resetLevelRenderData();
        }
    }
}
