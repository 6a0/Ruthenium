package net.thallium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import net.minecraft.client.renderer.item.ItemModelResolver;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.world.entity.ItemOwner;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.thallium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

@Mixin(ItemModelResolver.class)
public class ItemModelResolverPartsProbeMixin {
    @WrapMethod(method = "updateForTopItem")
    private void timeUpdate(ItemStackRenderState output, ItemStack item, ItemDisplayContext context, Level level, ItemOwner owner, int seed,
                            Operation<Void> original) {
        long start = System.nanoTime();
        original.call(output, item, context, level, owner, seed);
        EntityCostProbe.PARTS.add(item.isEmpty() ? "ItemModelResolver.updateForTopItem (empty)" : "ItemModelResolver.updateForTopItem " + context,
                System.nanoTime() - start, 0);
    }
}
