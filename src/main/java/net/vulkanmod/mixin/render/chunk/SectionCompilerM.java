package net.vulkanmod.mixin.render.chunk;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.llamalad7.mixinextras.sugar.Local;
import com.mojang.blaze3d.vertex.MeshData;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.SectionCompiler;
import net.vulkanmod.Initializer;
import net.vulkanmod.interfaces.FaceGroupsHolder;
import net.vulkanmod.render.chunk.SectionFaceGroups;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

import java.util.Map;

@Mixin(SectionCompiler.class)
public class SectionCompilerM {

    /** Groups the quads of each opaque layer's mesh by facing as it is added to the results */
    @WrapOperation(method = "compile", at = @At(value = "INVOKE", target = "Ljava/util/Map;put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;"))
    private Object groupQuadsByFacing(Map<Object, Object> layers, Object layer, Object mesh, Operation<Object> original,
                                      @Local SectionCompiler.Results results) {
        if (Initializer.CONFIG.terrainFaceCulling && layer instanceof ChunkSectionLayer sectionLayer
                && !sectionLayer.translucent() && mesh instanceof MeshData meshData) {
            ((FaceGroupsHolder) (Object) results).vulkanmod$setFaceGroups(sectionLayer, SectionFaceGroups.sort(meshData));
        }
        return original.call(layers, layer, mesh);
    }
}
