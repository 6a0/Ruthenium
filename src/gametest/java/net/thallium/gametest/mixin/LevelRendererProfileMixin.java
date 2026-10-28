package net.thallium.gametest.mixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.chunk.ChunkSectionsToRender;
import net.minecraft.util.profiling.Profiler;
import net.minecraft.util.profiling.ProfilerFiller;
import org.joml.Matrix4fc;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/** Profiler sections inside vanilla's "setupFrameGraph" for the light profiler */
@Mixin(LevelRenderer.class)
public class LevelRendererProfileMixin {
    @WrapOperation(method = "render", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/LevelRenderer;prepareChunkRendersIndirect(Lorg/joml/Matrix4fc;Z)Lnet/minecraft/client/renderer/chunk/ChunkSectionsToRender;"))
    private ChunkSectionsToRender profileTerrain(LevelRenderer renderer, Matrix4fc modelView, boolean respectOrder,
                                                 Operation<ChunkSectionsToRender> original) {
        ProfilerFiller profiler = Profiler.get();
        profiler.push("prepareChunkRendersIndirect");
        ChunkSectionsToRender result = original.call(renderer, modelView, respectOrder);
        profiler.pop();
        return result;
    }
}
