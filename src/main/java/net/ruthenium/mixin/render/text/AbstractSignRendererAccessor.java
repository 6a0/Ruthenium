package net.ruthenium.mixin.render.text;

import net.minecraft.client.renderer.blockentity.AbstractSignRenderer;
import net.minecraft.core.BlockPos;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Invoker;

@Mixin(AbstractSignRenderer.class)
public interface AbstractSignRendererAccessor {

    /** Whether glowing sign text at this position gets its outline this frame (camera entity within 16 blocks, or scoping) */
    @Invoker("isOutlineVisible")
    static boolean invokeIsOutlineVisible(BlockPos pos) {
        throw new AssertionError();
    }
}
