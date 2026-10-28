package net.ruthenium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.rendertype.RenderTypes;
import net.minecraft.resources.Identifier;
import net.ruthenium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

@Mixin(RenderTypes.class)
public class RenderTypesPartsProbeMixin {
    @WrapMethod(method = "entityCutout(Lnet/minecraft/resources/Identifier;Z)Lnet/minecraft/client/renderer/rendertype/RenderType;")
    private static RenderType timeEntityCutout(Identifier texture, boolean affectsOutline, Operation<RenderType> original) {
        long start = System.nanoTime();
        RenderType type = original.call(texture, affectsOutline);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.PARTS.add("RenderTypes.entityCutout", System.nanoTime() - start, 0);
        }
        return type;
    }
}
