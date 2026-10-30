package net.ruthenium.mixin.render.chunk;

import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.client.renderer.state.level.LevelRenderState;
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
 * <p>
 * The last frame's {@link LevelRenderState} is likewise only reset by the next world's first {@code extract}. Its
 * {@code sectionUpdateRenderStates} hold the regions of the sections that frame found dirty (each region keeps the
 * {@code ClientLevel}), so whenever sections were still being updated as the world was left, the whole world stayed
 * reachable (~640 chunks, +35-48 MB live heap after leaving, in some launches only). It is reset here too, as
 * {@code extract} would, along with the level load tracker's callback (it keeps the connection and its level).
 */
@Mixin(LevelExtractor.class)
public class LevelExtractorResetM {
    @Shadow @Final private LevelRenderer levelRenderer;
    @Shadow @Final private LevelRenderState levelRenderState;

    @Inject(method = "setLevel", at = @At("TAIL"))
    private void ruthenium$resetNow(@Nullable ClientLevel level, CallbackInfo ci) {
        if (level == null && Initializer.CONFIG.freeUnusedChunkData) {
            this.levelRenderer.resetLevelRenderData();
            this.levelRenderState.reset();
            this.levelRenderState.playerCompiledSectionCallback = null;
        }
    }
}
