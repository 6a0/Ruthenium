package net.ruthenium.gametest.mixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.blaze3d.pipeline.RenderTarget;
import com.mojang.renderpearl.api.textures.GpuSampler;
import com.mojang.renderpearl.api.textures.GpuTextureView;
import net.ruthenium.gametest.GpuProbe;
import net.ruthenium.interfaces.PackedSectionDraws;
import net.ruthenium.render.chunk.TerrainOcclusion;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** {@link GpuProbe}: timestamps around TerrainOcclusion's occluder pass and its culling passes, and the visibility it reads back */
@Mixin(value = TerrainOcclusion.class, remap = false)
public class TerrainOcclusionMixin {
    @Inject(method = "execute", at = @At(value = "INVOKE", target = "Lcom/mojang/blaze3d/systems/RenderSystem;getDevice()Lcom/mojang/renderpearl/api/device/GpuDevice;"))
    private static void beforeOccluders(RenderTarget target, GpuSampler terrainSampler, GpuTextureView atlas, boolean wireframe, CallbackInfo ci) {
        if (GpuProbe.ENABLED) {
            GpuProbe.cullTimestamp(0);
        }
    }

    @Inject(method = "execute", at = @At(value = "INVOKE", target = "Lnet/ruthenium/render/chunk/TerrainOcclusion;ensureHiZ(Lcom/mojang/renderpearl/api/device/GpuDevice;II)V"))
    private static void afterOccluders(RenderTarget target, GpuSampler terrainSampler, GpuTextureView atlas, boolean wireframe, CallbackInfo ci) {
        if (GpuProbe.ENABLED) {
            GpuProbe.cullTimestamp(1);
        }
    }

    @Inject(method = "execute", at = @At(value = "INVOKE", target = "Lnet/ruthenium/render/chunk/TerrainOcclusion;ensureVisibility(Lcom/mojang/renderpearl/api/device/GpuDevice;I)V"))
    private static void afterHiZ(RenderTarget target, GpuSampler terrainSampler, GpuTextureView atlas, boolean wireframe, CallbackInfo ci) {
        if (GpuProbe.ENABLED) {
            GpuProbe.cullTimestamp(2);
        }
    }

    @Inject(method = "execute", at = @At(value = "FIELD", target = "Lnet/ruthenium/render/chunk/TerrainOcclusion;commandCount:I", ordinal = 0))
    private static void afterVisibility(RenderTarget target, GpuSampler terrainSampler, GpuTextureView atlas, boolean wireframe, CallbackInfo ci) {
        if (GpuProbe.ENABLED) {
            GpuProbe.cullTimestamp(3);
        }
    }

    @WrapOperation(method = "apply", at = @At(value = "INVOKE", target = "Lnet/ruthenium/interfaces/PackedSectionDraws;ruthenium$setOccluder(Z)V"))
    private static void countVisibility(PackedSectionDraws mesh, boolean occluder, Operation<Void> original) {
        if (GpuProbe.ENABLED) {
            GpuProbe.countVisibility(occluder);
        }
        original.call(mesh, occluder);
    }

    @Inject(method = "execute", at = @At(value = "INVOKE", target = "Lnet/ruthenium/render/chunk/TerrainOcclusion;readBack(Lcom/mojang/renderpearl/api/device/GpuDevice;Lcom/mojang/renderpearl/api/commands/CommandEncoder;I)V"))
    private static void afterCulling(RenderTarget target, GpuSampler terrainSampler, GpuTextureView atlas, boolean wireframe, CallbackInfo ci) {
        if (GpuProbe.ENABLED) {
            GpuProbe.cullTimestamp(4);
        }
    }
}
