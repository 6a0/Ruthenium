package net.ruthenium.mixin.render.entity;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.entity.HumanoidMobRenderer;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.item.ItemStack;
import net.ruthenium.Initializer;
import net.ruthenium.render.entity.EmptyChargeDuration;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/** Humanoids that aren't using an item skip the crossbow charge time's enchantment iteration ({@link EmptyChargeDuration}) */
@Mixin(HumanoidMobRenderer.class)
public class HumanoidMobRendererM {
    @WrapOperation(method = "extractHumanoidRenderState", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/item/CrossbowItem;getChargeDuration(Lnet/minecraft/world/item/ItemStack;Lnet/minecraft/world/entity/LivingEntity;)I"))
    private static int ruthenium$emptyChargeDuration(ItemStack stack, LivingEntity entity, Operation<Integer> original) {
        if (Initializer.CONFIG.emptyCrossbowCharge && stack.isEmpty()) {
            return EmptyChargeDuration.VALUE;
        }
        return original.call(stack, entity);
    }
}
