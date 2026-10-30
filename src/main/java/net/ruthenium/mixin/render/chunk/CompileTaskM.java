package net.ruthenium.mixin.render.chunk;

import net.minecraft.client.renderer.chunk.RenderSectionRegion;
import net.ruthenium.Initializer;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Mutable;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/**
 * Switch {@code freeUnusedChunkData}: a section keeps its last compile task ({@code RenderSection.lastCompileTask}, for
 * cancelling) after it has run, and the task keeps its region: copies of the block states of the 27 sections around it
 * and of their chunks' block entity maps, plus references to the chunks themselves. Only {@code doTask} reads the
 * region, so it is dropped once {@code doTask} returns. Not on an exception: the dispatcher queues a task again after a
 * {@code NullPointerException}.
 */
@Mixin(targets = "net.minecraft.client.renderer.chunk.SectionRenderDispatcher$RenderSection$CompileTask")
public class CompileTaskM {
    @Shadow @Final @Mutable private RenderSectionRegion region;

    @Inject(method = "doTask", at = @At("RETURN"))
    private void ruthenium$releaseRegion(CallbackInfoReturnable<?> cir) {
        if (Initializer.CONFIG.freeUnusedChunkData) {
            this.region = null;
        }
    }
}
