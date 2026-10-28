package net.rubidium.mixin.render.feature;

import net.minecraft.client.renderer.feature.ItemFeatureRenderer;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.rubidium.render.entity.InstancedParts;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;

/** Item submits without foil become instances ({@link InstancedParts#addItem}) where they can */
@Mixin(ItemFeatureRenderer.class)
public abstract class ItemFeatureRendererM {
    @Shadow protected abstract void prepareOutlineSubmit(ItemFeatureRenderer.Submit submit);

    @Shadow protected abstract void prepareMainSubmit(ItemFeatureRenderer.Submit submit);

    /**
     * @author Rubidium
     * @reason Instanced items; otherwise the vanilla body
     */
    @Overwrite
    private void prepareSubmit(final ItemFeatureRenderer.Submit submit) {
        if (submit.outlineColor() != 0) {
            this.prepareOutlineSubmit(submit);
        } else if (submit.foilType() != ItemStackRenderState.FoilType.NONE
                || !InstancedParts.addItem(submit.quads(), submit.tintLayers(), submit.pose(), submit.lightCoords(), submit.overlayCoords())) {
            this.prepareMainSubmit(submit);
        }
    }
}
