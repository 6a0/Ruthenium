package net.ruthenium.render.entity;

import net.minecraft.util.Mth;

/**
 * {@code CrossbowItem.getChargeDuration} for an empty stack, which humanoid extraction asks for every humanoid each frame
 * ({@code HumanoidMobRenderer.extractHumanoidRenderState}, with the entity's use item, empty unless it is using one).
 * Vanilla computes {@code Mth.floor(EnchantmentHelper.modifyCrossbowChargingTime(stack, entity, 1.25F) * 20.0F)}, and an
 * empty stack has no components ({@code ItemStack.getComponents} returns {@code DataComponentMap.EMPTY} when
 * {@code isEmpty()}), so the enchantment iteration visits nothing and the answer is {@code floor(max(0, 1.25) * 20) = 25};
 * the iteration allocated an iterator per call (6.8% of the mixed scene's render thread allocation).
 * <p>
 * Mod compatibility: a mod that hooks {@code EnchantmentHelper.modifyCrossbowChargingTime}, {@code runIterationOnItem} or
 * {@code CrossbowItem.getChargeDuration} to change the charge time of empty stacks is bypassed during extraction (switch
 * {@code emptyCrossbowCharge}); non-empty stacks keep vanilla's call.
 */
public final class EmptyChargeDuration {
    /** Vanilla's arithmetic on the base time with no enchantment applied */
    public static final int VALUE = Mth.floor(Math.max(0.0F, 1.25F) * 20.0F);

    private EmptyChargeDuration() {
    }
}
