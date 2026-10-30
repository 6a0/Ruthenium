package net.ruthenium.mixin.render.vertex;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.CompactVectorArray;
import it.unimi.dsi.fastutil.ints.Int2ObjectOpenHashMap;
import net.minecraft.client.renderer.StagedVertexBuffer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;

/**
 * The centroids of a sorted draw ({@code decodeSortingPoints}) go into an array kept per size instead of a new one per draw
 * and frame. The upload decodes, sorts and writes each draw's indices before the next draw's centroids are decoded, and
 * nothing keeps the array (the sort order cache keys on its values), so an array is free again for the next draw of its
 * size. Every element is written by the decode. Render thread only; at most 256 sizes are kept.
 */
@Mixin(StagedVertexBuffer.class)
public class StagedVertexBufferSortM {
    @Unique private static final Int2ObjectOpenHashMap<CompactVectorArray> ARRAYS = new Int2ObjectOpenHashMap<>();

    @WrapOperation(method = "decodeSortingPoints", at = @At(value = "NEW", target = "(I)Lcom/mojang/blaze3d/vertex/CompactVectorArray;"))
    private static CompactVectorArray ruthenium$keptArray(int count, Operation<CompactVectorArray> original) {
        if (!RenderSystem.isOnRenderThread()) {
            return original.call(count);
        }
        CompactVectorArray array = ARRAYS.get(count);
        if (array == null) {
            array = new CompactVectorArray(count);
            if (ARRAYS.size() < 256) {
                ARRAYS.put(count, array);
            }
        }
        return array;
    }
}
