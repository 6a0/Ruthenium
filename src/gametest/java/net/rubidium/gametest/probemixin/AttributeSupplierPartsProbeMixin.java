package net.rubidium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.core.Holder;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.ai.attributes.AttributeInstance;
import net.minecraft.world.entity.ai.attributes.AttributeSupplier;
import net.rubidium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

import java.util.function.Consumer;

@Mixin(AttributeSupplier.class)
public class AttributeSupplierPartsProbeMixin {
    @WrapMethod(method = "createInstance")
    private AttributeInstance countCreate(Consumer<AttributeInstance> onDirty, Holder<Attribute> attribute, Operation<AttributeInstance> original) {
        long start = System.nanoTime();
        AttributeInstance instance = original.call(onDirty, attribute);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.PARTS.add("AttributeSupplier.createInstance " + attribute.getRegisteredName() + (instance == null ? " (null)" : ""),
                    System.nanoTime() - start, 0);
        }
        return instance;
    }
}
