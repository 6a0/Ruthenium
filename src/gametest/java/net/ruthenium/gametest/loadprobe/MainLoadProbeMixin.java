package net.ruthenium.gametest.loadprobe;

import net.minecraft.client.main.Main;
import net.ruthenium.gametest.LoadProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Startup milestones of the client's main method (see {@link LoadProbe}) */
@Mixin(Main.class)
public class MainLoadProbeMixin {
    @Inject(method = "main", at = @At("HEAD"))
    private static void ruthenium$head(String[] args, CallbackInfo ci) {
        LoadProbe.mark("Main.main");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/util/datafix/DataFixers;optimize(Ljava/util/Set;)Ljava/util/concurrent/CompletableFuture;"))
    private static void ruthenium$beforeDfu(String[] args, CallbackInfo ci) {
        LoadProbe.mark("before DataFixers.optimize (and its class init)");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/util/datafix/DataFixers;optimize(Ljava/util/Set;)Ljava/util/concurrent/CompletableFuture;", shift = At.Shift.AFTER))
    private static void ruthenium$afterDfu(String[] args, CallbackInfo ci) {
        LoadProbe.mark("after DataFixers.optimize");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/CrashReport;preload()V"))
    private static void ruthenium$beforePreload(String[] args, CallbackInfo ci) {
        LoadProbe.mark("before CrashReport.preload");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/CrashReport;preload()V", shift = At.Shift.AFTER))
    private static void ruthenium$afterPreload(String[] args, CallbackInfo ci) {
        LoadProbe.mark("after CrashReport.preload");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/server/Bootstrap;bootStrap()V"))
    private static void ruthenium$beforeBootstrap(String[] args, CallbackInfo ci) {
        LoadProbe.mark("before Bootstrap.bootStrap");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/server/Bootstrap;bootStrap()V", shift = At.Shift.AFTER))
    private static void ruthenium$afterBootstrap(String[] args, CallbackInfo ci) {
        LoadProbe.mark("after Bootstrap.bootStrap");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/ClientBootstrap;bootstrap()V", shift = At.Shift.AFTER))
    private static void ruthenium$afterClientBootstrap(String[] args, CallbackInfo ci) {
        LoadProbe.mark("after ClientBootstrap.bootstrap");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/server/Bootstrap;validate()V", shift = At.Shift.AFTER))
    private static void ruthenium$afterValidate(String[] args, CallbackInfo ci) {
        LoadProbe.mark("after Bootstrap.validate");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Ljava/util/concurrent/CompletableFuture;join()Ljava/lang/Object;", ordinal = 0))
    private static void ruthenium$beforeDfuJoin(String[] args, CallbackInfo ci) {
        LoadProbe.mark("before the DFU optimization join");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Ljava/util/concurrent/CompletableFuture;join()Ljava/lang/Object;", ordinal = 0, shift = At.Shift.AFTER))
    private static void ruthenium$afterDfuJoin(String[] args, CallbackInfo ci) {
        LoadProbe.mark("after the DFU optimization join");
    }

    @Inject(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/Minecraft;run()V"))
    private static void ruthenium$beforeRun(String[] args, CallbackInfo ci) {
        LoadProbe.mark("Minecraft constructed, run loop starts");
    }
}
