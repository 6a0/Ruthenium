package net.rubidium.mixin.render.chunk;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.llamalad7.mixinextras.sugar.Local;
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
import net.rubidium.interfaces.SectionLightWindow;
import net.rubidium.Initializer;
import net.rubidium.interfaces.FaceGroupsHolder;
import net.rubidium.render.chunk.CompressedTerrain;
import net.rubidium.render.chunk.CompressedTerrainBuilder;
import net.rubidium.render.chunk.SectionFaceGroups;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import java.util.Map;

@Mixin(SectionCompiler.class)
public class SectionCompilerM {

    /** The smooth lighting keeps this section's light while it compiles ({@code BlockModelLighterCacheM}) */
    @Inject(method = "compile", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/renderer/block/BlockModelLighter;enableCaching()V", shift = At.Shift.AFTER))
    private void beginLightWindow(SectionPos sectionPos, RenderSectionRegion region, VertexSorting vertexSorting,
                                  SectionBufferBuilderPack builders, CallbackInfoReturnable<SectionCompiler.Results> cir) {
        ((SectionLightWindow) BlockModelLighterAccessor.rubidium$getCache().get()).rubidium$beginSection(sectionPos.origin());
    }

    /** Groups the quads of each opaque layer's mesh by facing as it is added to the results */
    @WrapOperation(method = "compile", at = @At(value = "INVOKE", target = "Ljava/util/Map;put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;"))
    private Object groupQuadsByFacing(Map<Object, Object> layers, Object layer, Object mesh, Operation<Object> original,
                                      @Local SectionCompiler.Results results) {
        if (Initializer.CONFIG.terrainFaceCulling && layer instanceof ChunkSectionLayer sectionLayer
                && !sectionLayer.translucent() && mesh instanceof MeshData meshData) {
            ((FaceGroupsHolder) (Object) results).rubidium$setFaceGroups(sectionLayer, SectionFaceGroups.sort(meshData));
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
