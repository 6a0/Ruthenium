package net.ruthenium.gametest.loadprobe;

import net.minecraft.client.Minecraft;
import net.ruthenium.gametest.LoadProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Startup milestones of the {@link Minecraft} constructor (see {@link LoadProbe}) */
@Mixin(Minecraft.class)
public class MinecraftLoadProbeMixin {
    @Inject(method = "<init>", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/resources/ClientPackSource;getVanillaPack()Lnet/minecraft/server/packs/VanillaPackResources;"))
    private void ruthenium$start(CallbackInfo ci) {
        LoadProbe.mark("Minecraft.<init>: pack sources made");
    }

    @Inject(method = "<init>", at = @At(value = "INVOKE", target = "Lcom/mojang/blaze3d/systems/RenderSystem;initBackendSystem()Lnet/minecraft/util/TimeSource$NanoTimeSource;"))
    private void ruthenium$beforeBackend(CallbackInfo ci) {
        LoadProbe.mark("Minecraft.<init>: options loaded, before initBackendSystem");
    }

    @Inject(method = "<init>", at = @At(value = "INVOKE", target = "Lcom/mojang/blaze3d/systems/RenderSystem;initRenderer(Lcom/mojang/renderpearl/api/device/GpuDevice;)V"))
    private void ruthenium$deviceCreated(CallbackInfo ci) {
        LoadProbe.mark("Minecraft.<init>: device created");
    }

    @Inject(method = "<init>", at = @At(value = "INVOKE", target = "Lcom/mojang/blaze3d/platform/Window;setIcon(Lnet/minecraft/server/packs/PackMetadataResources;Lcom/mojang/blaze3d/platform/IconSet;)V"))
    private void ruthenium$windowCreated(CallbackInfo ci) {
        LoadProbe.mark("Minecraft.<init>: window created");
    }

    @Inject(method = "<init>", at = @At(value = "INVOKE", target = "Lnet/minecraft/server/packs/repository/PackRepository;reload()V"))
    private void ruthenium$beforePacks(CallbackInfo ci) {
        LoadProbe.mark("Minecraft.<init>: before the pack repository reload");
    }

    @Inject(method = "<init>", at = @At(value = "INVOKE", target = "Lnet/minecraft/server/packs/repository/PackRepository;reload()V", shift = At.Shift.AFTER))
    private void ruthenium$afterPacks(CallbackInfo ci) {
        LoadProbe.mark("Minecraft.<init>: after the pack repository reload");
    }

    @Inject(method = "<init>", at = @At(value = "INVOKE", target = "Lnet/minecraft/server/packs/resources/ReloadableResourceManager;createReload(Ljava/util/concurrent/Executor;Ljava/util/concurrent/Executor;Ljava/util/concurrent/CompletableFuture;Ljava/util/List;)Lnet/minecraft/server/packs/resources/ReloadInstance;"))
    private void ruthenium$beforeReload(CallbackInfo ci) {
        LoadProbe.mark("Minecraft.<init>: initial resource reload starts");
    }

    @Inject(method = "<init>", at = @At("RETURN"))
    private void ruthenium$end(CallbackInfo ci) {
        LoadProbe.mark("Minecraft.<init> returns");
    }
}
