package net.thallium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.thallium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;

import java.util.List;

/** Times each atlas's animation tick; extra = animated sprites drawn this tick (each once per mip level) */
@Mixin(TextureAtlas.class)
public class TextureAtlasTickPartsProbeMixin {
    @Shadow private List<SpriteContents.AnimationState> animatedTexturesStates;
    @Shadow private int maxMipLevel;

    @WrapMethod(method = "cycleAnimationFrames")
    private void timeAnimation(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        long nanos = System.nanoTime() - start;
        int drawn = 0;
        for (SpriteContents.AnimationState state : this.animatedTexturesStates) {
            if (state.needsToDraw()) {
                drawn++;
            }
        }
        EntityCostProbe.TICK_PARTS.add("TextureAtlas.cycleAnimationFrames " + ((TextureAtlas) (Object) this).location()
                + " (" + this.animatedTexturesStates.size() + " animated, mips " + (this.maxMipLevel + 1) + ")", nanos, drawn);
    }
}
