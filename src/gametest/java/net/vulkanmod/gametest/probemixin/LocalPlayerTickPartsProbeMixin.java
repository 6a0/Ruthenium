package net.vulkanmod.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.resources.sounds.AmbientSoundHandler;
import net.vulkanmod.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

@Mixin(LocalPlayer.class)
public class LocalPlayerTickPartsProbeMixin {
    @WrapOperation(method = "tick", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/resources/sounds/AmbientSoundHandler;tick()V"))
    private void timeAmbient(AmbientSoundHandler handler, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(handler);
        EntityCostProbe.TICK_PARTS.add("LocalPlayer ambient " + handler.getClass().getSimpleName(), System.nanoTime() - start, 0);
    }

    @WrapMethod(method = "aiStep")
    private void timeAiStep(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        EntityCostProbe.TICK_PARTS.add("LocalPlayer.aiStep", System.nanoTime() - start, 0);
    }
}
