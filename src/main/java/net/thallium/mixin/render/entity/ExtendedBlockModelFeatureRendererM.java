package net.thallium.mixin.render.entity;

import net.fabricmc.fabric.api.client.renderer.v1.mesh.Mesh;
import net.fabricmc.fabric.api.client.renderer.v1.mesh.QuadEmitter;
import net.fabricmc.fabric.api.client.renderer.v1.render.submit.ExtendedBlockModelSubmit;
import net.thallium.render.entity.InstancedParts;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Pseudo;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/**
 * Block model meshes of entities become instances ({@link InstancedParts#addBlockMesh}) where they can. Indigo's
 * renderer (Fabric API), absent when another renderer replaces it; if the call isn't found the class stays as it is.
 */
@Pseudo
@Mixin(targets = "net.fabricmc.fabric.impl.client.indigo.renderer.render.ExtendedBlockModelFeatureRenderer")
public class ExtendedBlockModelFeatureRendererM {
    @Shadow private ExtendedBlockModelSubmit submit;

    @Redirect(method = "buildGroup", require = 0, at = @At(value = "INVOKE",
            target = "Lnet/fabricmc/fabric/api/client/renderer/v1/mesh/Mesh;outputTo(Lnet/fabricmc/fabric/api/client/renderer/v1/mesh/QuadEmitter;)V"))
    private void instancedMesh(Mesh mesh, QuadEmitter emitter) {
        ExtendedBlockModelSubmit submit = this.submit;
        if (submit.sheetedDecalPose() != null
                || !InstancedParts.addBlockMesh(mesh, submit.renderTypeFunction(), submit.tintColor(), submit.tintLayers(), submit.pose(),
                        submit.lightCoords(), submit.overlayCoords())) {
            mesh.outputTo(emitter);
        }
    }
}
