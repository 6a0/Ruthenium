package net.thallium.mixin.world;

import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.BlockPos;
import net.minecraft.util.RandomSource;
import net.minecraft.world.attribute.EnvironmentAttribute;
import net.minecraft.world.attribute.EnvironmentAttributeSystem;
import net.thallium.Initializer;
import net.thallium.world.AnimateTick;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** See {@link AnimateTick} */
@Mixin(ClientLevel.class)
public class ClientLevelAnimateTickM {
    @Inject(method = "animateTick", at = @At("HEAD"))
    private void beginAnimateTick(int x, int y, int z, CallbackInfo ci) {
        AnimateTick.begin((ClientLevel) (Object) this, x, y, z);
    }

    @Redirect(method = "doAnimateTick", at = @At(value = "INVOKE", target = "Lnet/minecraft/util/RandomSource;nextInt(I)I"))
    private int nextInt(RandomSource random, int bound) {
        return Initializer.CONFIG.fastAnimateTick ? AnimateTick.nextInt(random, bound) : random.nextInt(bound);
    }

    @Redirect(method = "doAnimateTick", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/attribute/EnvironmentAttributeSystem;getValue(Lnet/minecraft/world/attribute/EnvironmentAttribute;Lnet/minecraft/core/BlockPos;)Ljava/lang/Object;"))
    private Object ambientParticles(EnvironmentAttributeSystem attributes, EnvironmentAttribute<?> attribute, BlockPos pos) {
        return Initializer.CONFIG.fastAnimateTick
                ? AnimateTick.getValue((ClientLevel) (Object) this, attributes, attribute, pos)
                : attributes.getValue(attribute, pos);
    }
}
