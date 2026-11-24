/*
 * Ruthenium exporters: dump code-defined game data from the player's own
 * installed client jar into Ruthenium's binary caches.
 *
 * Original Ruthenium code, GPL-3.0-or-later. The bundle built from this folder
 * must contain only dev/ruthenium/ classes (checked by `cargo xtask exporters`).
 */
package dev.ruthenium.exporters;

import java.nio.file.Files;
import java.nio.file.Path;

import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;

public final class Main {
    private Main() {}

    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            System.err.println("usage: dev.ruthenium.exporters.Main <output folder>");
            System.exit(2);
        }
        Path out = Path.of(args[0]);
        Files.createDirectories(out);

        // The same start-up the data generator performs before reading registries.
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        EntityDimensionsExporter.export(out.resolve(EntityDimensionsExporter.FILE));
        BlockPhysicsExporter.export(out.resolve(BlockPhysicsExporter.FILE));
        ItemClassExporter.export(out.resolve(ItemClassExporter.FILE));
        BlockBehaviourExporter.export(out.resolve(BlockBehaviourExporter.FILE));
        BlockRenderExporter.export(out.resolve(BlockRenderExporter.FILE));
    }
}
