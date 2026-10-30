package net.ruthenium.mixin.render;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.renderpearl.api.commands.CommandEncoder;
import com.mojang.renderpearl.api.textures.GpuTexture;
import net.minecraft.client.renderer.GameRenderer;
import net.ruthenium.render.FrameClear;
import org.joml.Vector4fc;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** {@link FrameClear}: the frame's first clear of the main target is deferred when the level is drawn */
@Mixin(GameRenderer.class)
public class GameRendererM {
    @WrapOperation(method = "render", at = @At(value = "INVOKE",
            target = "Lcom/mojang/renderpearl/api/commands/CommandEncoder;clearColorAndDepthTextures(Lcom/mojang/renderpearl/api/textures/GpuTexture;Lorg/joml/Vector4fc;Lcom/mojang/renderpearl/api/textures/GpuTexture;D)V"))
    private void ruthenium$deferFrameClear(CommandEncoder encoder, GpuTexture color, Vector4fc clearColor, GpuTexture depth, double clearDepth,
                                           Operation<Void> original) {
        if (!((GameRenderer) (Object) this).gameRenderState().shouldRenderLevel || !FrameClear.defer(color, clearColor, depth, clearDepth)) {
            original.call(encoder, color, clearColor, depth, clearDepth);
        }
    }

    @WrapOperation(method = "render3dHud", at = @At(value = "INVOKE",
            target = "Lcom/mojang/renderpearl/api/commands/CommandEncoder;clearDepthTexture(Lcom/mojang/renderpearl/api/textures/GpuTexture;D)V"))
    private void ruthenium$deferHandDepthClear(CommandEncoder encoder, GpuTexture depth, double clearDepth, Operation<Void> original) {
        // Only the main target's depth: the GUI clears that one again (the hud target's depth is read by post effects)
        if (depth != ((GameRenderer) (Object) this).mainRenderTarget().getDepthTexture() || !FrameClear.deferDepth(depth, clearDepth)) {
            original.call(encoder, depth, clearDepth);
        }
    }

    /** Nothing used the target this frame (it can't happen in vanilla): clear it as vanilla would have */
    @Inject(method = "render", at = @At("RETURN"))
    private void ruthenium$flushFrameClear(CallbackInfo ci) {
        FrameClear.flush();
    }
}
