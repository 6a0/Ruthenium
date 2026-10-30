package net.ruthenium.mixin.render.chunk;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.llamalad7.mixinextras.sugar.Local;
import com.llamalad7.mixinextras.sugar.Share;
import com.llamalad7.mixinextras.sugar.ref.LocalRef;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import com.mojang.blaze3d.vertex.VertexSorting;
import net.minecraft.client.renderer.SectionBufferBuilderPack;
import net.minecraft.client.renderer.chunk.RenderSectionRegion;
import net.minecraft.client.renderer.chunk.SectionCompiler;
import net.minecraft.core.SectionPos;
import net.ruthenium.interfaces.SectionLightWindow;
import net.ruthenium.interfaces.SectionScratchHolder;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.FaceGroupsHolder;
import net.ruthenium.render.chunk.CompressedTerrain;
import net.ruthenium.render.chunk.CompressedTerrainBuilder;
import net.ruthenium.render.chunk.SectionFaceGroups;
import net.ruthenium.render.chunk.SectionScratch;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import java.util.Map;

@Mixin(SectionCompiler.class)
public class SectionCompilerM {

    /**
     * The compile borrows its scratch ({@link SectionScratch}: block states, light window, tint grid) and binds the
     * region to it on this thread
     */
    @Inject(method = "compile", at = @At("HEAD"))
    private void borrowScratch(SectionPos sectionPos, RenderSectionRegion region, VertexSorting vertexSorting,
                               SectionBufferBuilderPack builders, CallbackInfoReturnable<SectionCompiler.Results> cir,
                               @Share("scratch") LocalRef<SectionScratch> scratch) {
        SectionScratch borrowed = SectionScratch.borrow();
        scratch.set(borrowed);
        ((SectionScratchHolder) region).ruthenium$bindScratch(borrowed);
    }

    /** The smooth lighting keeps this section's light while it compiles ({@code BlockModelLighterCacheM}) */
    @Inject(method = "compile", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/renderer/block/BlockModelLighter;enableCaching()V", shift = At.Shift.AFTER))
    private void beginLightWindow(SectionPos sectionPos, RenderSectionRegion region, VertexSorting vertexSorting,
                                  SectionBufferBuilderPack builders, CallbackInfoReturnable<SectionCompiler.Results> cir,
                                  @Share("scratch") LocalRef<SectionScratch> scratch) {
        ((SectionLightWindow) BlockModelLighterAccessor.ruthenium$getCache().get()).ruthenium$beginSection(sectionPos.origin(), scratch.get());
    }

    /**
     * The scratch goes back after the light cache was disabled. Not after an exception: the region and this thread's
     * light cache may still point at it then, so it is left to them (and the garbage collector) instead of the pool.
     */
    @Inject(method = "compile", at = @At("RETURN"))
    private void releaseScratch(SectionPos sectionPos, RenderSectionRegion region, VertexSorting vertexSorting,
                                SectionBufferBuilderPack builders, CallbackInfoReturnable<SectionCompiler.Results> cir,
                                @Share("scratch") LocalRef<SectionScratch> scratch) {
        ((SectionScratchHolder) region).ruthenium$unbindScratch();
        scratch.get().release();
    }

    /** Groups the quads of each opaque layer's mesh by facing as it is added to the results */
    @WrapOperation(method = "compile", at = @At(value = "INVOKE", target = "Ljava/util/Map;put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;"))
    private Object groupQuadsByFacing(Map<Object, Object> layers, Object layer, Object mesh, Operation<Object> original,
                                      @Local SectionCompiler.Results results) {
        if (Initializer.CONFIG.terrainFaceCulling && layer instanceof ChunkSectionLayer sectionLayer
                && !sectionLayer.translucent() && mesh instanceof MeshData meshData) {
            ((FaceGroupsHolder) (Object) results).ruthenium$setFaceGroups(sectionLayer, SectionFaceGroups.sort(meshData));
        }
        return original.call(layers, layer, mesh);
    }

    /** Sections in the compressed format get its builder */
    @WrapOperation(method = "getOrBeginLayer", at = @At(value = "NEW", target = "(Lcom/mojang/blaze3d/vertex/ByteBufferBuilder;Lcom/mojang/renderpearl/api/pipeline/PrimitiveTopology;Lcom/mojang/renderpearl/api/vertex/VertexFormat;)Lcom/mojang/blaze3d/vertex/BufferBuilder;"))
    private BufferBuilder compressedBuilder(ByteBufferBuilder buffer, PrimitiveTopology topology, VertexFormat format,
                                            Operation<BufferBuilder> original) {
        return format == CompressedTerrain.FORMAT ? new CompressedTerrainBuilder(buffer, topology) : original.call(buffer, topology, format);
    }
}
