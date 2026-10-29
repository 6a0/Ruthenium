package net.ruthenium.mixin.render.entity;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.entity.ArmorStandRenderer;
import net.minecraft.client.renderer.entity.LivingEntityRenderer;
import net.minecraft.client.renderer.entity.layers.CustomHeadLayer;
import net.minecraft.client.renderer.entity.layers.HumanoidArmorLayer;
import net.minecraft.client.renderer.entity.layers.ItemInHandLayer;
import net.minecraft.client.renderer.entity.layers.RenderLayer;
import net.minecraft.client.renderer.entity.layers.WingsLayer;
import net.minecraft.client.renderer.entity.state.ArmorStandRenderState;
import net.minecraft.client.renderer.entity.state.LivingEntityRenderState;
import net.ruthenium.Initializer;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;

import java.util.List;

/**
 * Armor stands without equipment skip their layers ({@code emptyArmorStandLayers}). {@code LivingEntityRenderer.submit}
 * poses the model ({@code setupAnim}) and runs every layer for every entity; vanilla's armor stand layers (armor,
 * held items, elytra, head items) draw nothing for an armor stand with empty equipment, hands and head, and the model
 * pose they would read is set again by any later submit that uses it (the model's own submit poses it when it is
 * prepared). Hologram plugins show text with hundreds of such armor stands (invisible markers with a visible name).
 * Only for vanilla's armor stand renderer with exactly its four vanilla layers (a mod that adds a layer keeps
 * vanilla's path), so the image is the same.
 */
@Mixin(LivingEntityRenderer.class)
public class LivingEntityRendererLayersM {
    @Shadow @Final protected List<RenderLayer<?, ?>> layers;

    @WrapOperation(method = "submit(Lnet/minecraft/client/renderer/entity/state/LivingEntityRenderState;Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;Lnet/minecraft/client/renderer/state/level/CameraRenderState;)V",
            at = @At(value = "INVOKE",
                    target = "Lnet/minecraft/client/renderer/entity/LivingEntityRenderer;shouldRenderLayers(Lnet/minecraft/client/renderer/entity/state/LivingEntityRenderState;)Z"))
    private boolean ruthenium$skipEmptyLayers(LivingEntityRenderer<?, ?, ?> renderer, LivingEntityRenderState state, Operation<Boolean> original) {
        if (!original.call(renderer, state)) {
            return false;
        }
        if (!Initializer.CONFIG.emptyArmorStandLayers || renderer.getClass() != ArmorStandRenderer.class
                || !(state instanceof ArmorStandRenderState armorStand)) {
            return true;
        }
        return !(isEmpty(armorStand) && hasVanillaLayers(this.layers));
    }

    private static boolean isEmpty(ArmorStandRenderState state) {
        return state.headEquipment.isEmpty() && state.chestEquipment.isEmpty() && state.legsEquipment.isEmpty() && state.feetEquipment.isEmpty()
                && state.rightHandItemState.isEmpty() && state.leftHandItemState.isEmpty() && state.headItem.isEmpty() && state.wornHeadType == null;
    }

    private static boolean hasVanillaLayers(List<RenderLayer<?, ?>> layers) {
        return layers.size() == 4 && layers.get(0).getClass() == HumanoidArmorLayer.class && layers.get(1).getClass() == ItemInHandLayer.class
                && layers.get(2).getClass() == WingsLayer.class && layers.get(3).getClass() == CustomHeadLayer.class;
    }
}
