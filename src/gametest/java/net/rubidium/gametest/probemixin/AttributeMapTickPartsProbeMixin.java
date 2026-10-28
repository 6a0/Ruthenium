package net.rubidium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.world.entity.ai.attributes.AttributeInstance;
import net.minecraft.world.entity.ai.attributes.AttributeMap;
import net.rubidium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

/** Counts attribute modifications on the client (each one makes the entity's next tick refresh its dirty attributes) */
@Mixin(AttributeMap.class)
public class AttributeMapTickPartsProbeMixin {
    @WrapMethod(method = "onAttributeModified")
    private void countModified(AttributeInstance instance, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(instance);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("AttributeMap.onAttributeModified " + instance.getAttribute().getRegisteredName(),
                    System.nanoTime() - start, 0);
        }
    }
}
