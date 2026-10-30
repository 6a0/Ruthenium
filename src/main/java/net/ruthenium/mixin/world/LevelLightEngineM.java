package net.ruthenium.mixin.world;

import net.minecraft.world.level.chunk.DataLayer;
import net.minecraft.world.level.lighting.LevelLightEngine;
import net.ruthenium.Initializer;
import net.ruthenium.world.UniformLight;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyVariable;

/**
 * Switch {@code compactSkyLight} ({@link UniformLight}): every light array that enters a light engine from outside
 * passes here, on both sides: the client's from light and chunk packets ({@code ClientPacketListener.readSectionList}),
 * the integrated server's from the chunks it loads from disk ({@code SerializableChunkData.read}, through
 * {@code ThreadedLevelLightEngine}, whose override calls this method on the light thread).
 */
@Mixin(LevelLightEngine.class)
public class LevelLightEngineM {
    @ModifyVariable(method = "queueSectionData", at = @At("HEAD"), argsOnly = true)
    private DataLayer ruthenium$compactSkyLight(DataLayer data) {
        return Initializer.CONFIG.compactSkyLight ? UniformLight.compact(data) : data;
    }
}
