package net.vulkanmod.gametest.mixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.vulkanmod.gametest.TickProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/** Times the client's entity and block entity ticks at their call sites (mods may cancel inside them) */
@Mixin(Minecraft.class)
public class TickProbeMixin {
    @WrapOperation(method = "tick", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/multiplayer/ClientLevel;tickEntities()V"))
    private void timeEntities(ClientLevel level, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(level);
        TickProbe.addEntities(System.nanoTime() - start);
    }

    @WrapOperation(method = "tick", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/multiplayer/ClientLevel;tickBlockEntities()V"))
    private void timeBlockEntities(ClientLevel level, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(level);
        TickProbe.addBlockEntities(System.nanoTime() - start);
    }
}
