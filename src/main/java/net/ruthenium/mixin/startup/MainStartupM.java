package net.ruthenium.mixin.startup;

import com.mojang.datafixers.DSL;
import net.minecraft.CrashReport;
import net.minecraft.client.main.Main;
import net.minecraft.util.datafix.DataFixers;
import net.ruthenium.Initializer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import java.util.Set;
import java.util.concurrent.CompletableFuture;

/**
 * Two steps of the client's main method that run on the main thread before the bootstrap (registries, blocks, items)
 * although nothing there needs their results, so they move to background threads and the bootstrap starts right away:
 * <ul>
 *     <li>Switch {@code backgroundSystemReport}: {@link CrashReport#preload} builds a throwaway crash report so its
 *     classes are loaded before an out-of-memory crash needs them. Its system report queries the hardware and the
 *     OS (OSHI), ~0.6-0.9 s on Windows. Loading those classes on another thread preloads them just as well.</li>
 *     <li>Switch {@code backgroundDataFixers}: the data fixers' class init builds every schema of every version
 *     (~0.5 s), then {@code optimize} prepares the level summary rules on its own thread. The class init now runs on
 *     a background thread too. Main still joins the same future before creating the game window, as vanilla does, and
 *     the bootstrap's entity and block entity types, which look their types up in the current schema, wait for the
 *     class init through the JVM's class init lock if they get there first. The data fixers' init only builds DFU
 *     types from names (no game registries), so it can't wait on a class the bootstrap is initializing.</li>
 * </ul>
 */
@Mixin(Main.class)
public class MainStartupM {
    @Redirect(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/CrashReport;preload()V"))
    private static void ruthenium$preloadInBackground() {
        if (!Initializer.config().backgroundSystemReport) {
            CrashReport.preload();
            return;
        }
        Thread thread = new Thread(() -> {
            try {
                CrashReport.preload();
            } catch (Throwable t) {
                Initializer.LOGGER.warn("Crash report preload failed", t);
            }
        }, "Crash Report Preload");
        thread.setDaemon(true);
        thread.start();
    }

    @Redirect(method = "main", at = @At(value = "INVOKE", target = "Lnet/minecraft/util/datafix/DataFixers;optimize(Ljava/util/Set;)Ljava/util/concurrent/CompletableFuture;"))
    private static CompletableFuture<?> ruthenium$dataFixersInBackground(Set<DSL.TypeReference> types) {
        if (!Initializer.config().backgroundDataFixers) {
            return DataFixers.optimize(types);
        }
        CompletableFuture<CompletableFuture<?>> built = new CompletableFuture<>();
        Thread thread = new Thread(() -> {
            try {
                built.complete(DataFixers.optimize(types));
            } catch (Throwable t) {
                built.completeExceptionally(t);
            }
        }, "Datafixer Build");
        thread.setDaemon(true);
        thread.start();
        return built.thenCompose(optimization -> optimization.thenApply(result -> (Object) result));
    }
}
