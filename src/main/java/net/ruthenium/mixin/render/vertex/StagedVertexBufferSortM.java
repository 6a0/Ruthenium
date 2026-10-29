package net.ruthenium.mixin.render.vertex;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.VertexSorting;
import net.minecraft.client.renderer.StagedVertexBuffer;
import net.ruthenium.Initializer;
import net.ruthenium.render.vertex.DistanceSorting;
import net.ruthenium.render.vertex.SortOrderCache;
import net.ruthenium.render.vertex.SortedQuadIndices;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.nio.ByteBuffer;

/** Sorted draws start their quad sort from their order in the previous upload ({@link SortOrderCache}) */
@Mixin(StagedVertexBuffer.class)
public class StagedVertexBufferSortM {
    private static final String UPLOAD = "uploadDrawsToBuffers";

    @Unique
    private final SortOrderCache ruthenium$sortOrders = new SortOrderCache();
    @Unique
    private boolean ruthenium$sortOrdersUsed;

    @Inject(method = UPLOAD, at = @At("HEAD"))
    private void beginSortOrders(CallbackInfo ci) {
        if (Initializer.CONFIG.reuseSortOrder) {
            this.ruthenium$sortOrders.beginUpload();
            this.ruthenium$sortOrdersUsed = true;
        } else if (this.ruthenium$sortOrdersUsed) {
            this.ruthenium$sortOrders.clear();
            this.ruthenium$sortOrdersUsed = false;
        }
    }

    @WrapOperation(method = UPLOAD, at = @At(value = "INVOKE",
            target = "Lcom/mojang/blaze3d/vertex/MeshData$SortState;writeSortedIndexBuffer(Ljava/nio/ByteBuffer;Lcom/mojang/blaze3d/vertex/VertexSorting;)V"))
    private void sortFromLastOrder(MeshData.SortState state, ByteBuffer buffer, VertexSorting sorting, Operation<Void> original) {
        if (this.ruthenium$sortOrdersUsed && sorting instanceof DistanceSorting.Sorter sorter) {
            SortedQuadIndices.write(buffer, this.ruthenium$sortOrders.sort(sorter, state.centroids()), state.indexType());
        } else {
            original.call(state, buffer, sorting);
        }
    }
}
