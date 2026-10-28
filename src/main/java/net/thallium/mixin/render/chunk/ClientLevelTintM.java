package net.thallium.mixin.render.chunk;

import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.ColorResolver;
import net.minecraft.world.level.chunk.LevelChunk;
import net.thallium.Initializer;
import net.thallium.render.chunk.BiomeTintBlend;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/** Biome tint blending with cached samples ({@link BiomeTintBlend}) */
@Mixin(ClientLevel.class)
public class ClientLevelTintM {
    @Inject(method = "calculateBlockTint", at = @At("HEAD"), cancellable = true)
    private void blendWithCachedSamples(BlockPos pos, ColorResolver colorResolver, CallbackInfoReturnable<Integer> cir) {
        if (Initializer.CONFIG.biomeBlendCache) {
            int radius = Minecraft.getInstance().options.biomeBlendRadius().get();
            if (BiomeTintBlend.supports(radius)) {
                cir.setReturnValue(BiomeTintBlend.blend((ClientLevel) (Object) this, pos, colorResolver, radius));
            }
        }
    }

    // After the chunk's (or its biomes') data is in place
    @Inject(method = "onChunkLoaded", at = @At("RETURN"))
    private void invalidateOnLoad(CallbackInfo ci) {
        BiomeTintBlend.invalidate();
    }

    @Inject(method = "unload", at = @At("RETURN"))
    private void invalidateOnUnload(LevelChunk levelChunk, CallbackInfo ci) {
        BiomeTintBlend.invalidate();
    }

    @Inject(method = "clearTintCaches", at = @At("RETURN"))
    private void invalidateOnClear(CallbackInfo ci) {
        BiomeTintBlend.invalidate();
    }
}
