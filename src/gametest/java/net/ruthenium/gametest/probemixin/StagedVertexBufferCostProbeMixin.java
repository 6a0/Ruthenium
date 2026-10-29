package net.ruthenium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.blaze3d.vertex.CompactVectorArray;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.VertexSorting;
import com.mojang.renderpearl.api.device.GpuDevice;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import net.minecraft.client.renderer.StagedVertexBuffer;
import net.ruthenium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

import java.nio.ByteBuffer;
import java.util.List;

/** The shared vertex buffer's upload: in total (extra = draws), decoding quad centroids and sorting (extra = quads) */
@Mixin(StagedVertexBuffer.class)
public class StagedVertexBufferCostProbeMixin {
    @WrapMethod(method = "uploadDrawsToBuffers")
    private void timeUpload(GpuDevice device, List<StagedVertexBuffer.Draw> draws, GpuBuffer vertexBuffer, GpuBuffer indexBuffer, int vertexSize,
                            int indexSize, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(device, draws, vertexBuffer, indexBuffer, vertexSize, indexSize);
        EntityCostProbe.UPLOAD.add("uploadDrawsToBuffers (extra = draws)", System.nanoTime() - start, draws.size());
        EntityCostProbe.UPLOAD.add("vertex bytes (extra = KB)", 0, vertexSize / 1024);
        EntityCostProbe.UPLOAD.add("index bytes (extra = KB)", 0, indexSize / 1024);
    }

    @WrapOperation(method = "uploadDrawsToBuffers", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/StagedVertexBuffer;decodeSortingPoints(Lnet/minecraft/client/renderer/StagedVertexBuffer$Draw;)Lcom/mojang/blaze3d/vertex/CompactVectorArray;"))
    private CompactVectorArray timeDecode(StagedVertexBuffer.Draw draw, Operation<CompactVectorArray> original) {
        long start = System.nanoTime();
        CompactVectorArray points = original.call(draw);
        EntityCostProbe.UPLOAD.add("decode quad centroids (extra = quads)", System.nanoTime() - start, points.size());
        return points;
    }

    @WrapOperation(method = "uploadDrawsToBuffers", at = @At(value = "INVOKE",
            target = "Lcom/mojang/blaze3d/vertex/MeshData$SortState;writeSortedIndexBuffer(Ljava/nio/ByteBuffer;Lcom/mojang/blaze3d/vertex/VertexSorting;)V"))
    private void timeSort(MeshData.SortState state, ByteBuffer buffer, VertexSorting sorting, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(state, buffer, sorting);
        EntityCostProbe.UPLOAD.add("sort and write indices (extra = quads)", System.nanoTime() - start, state.centroids().size());
    }
}
