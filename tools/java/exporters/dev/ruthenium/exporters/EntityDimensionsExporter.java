/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.io.IOException;
import java.nio.file.Path;

import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.EntityDimensions;
import net.minecraft.world.entity.EntityType;

/**
 * Default dimensions of every entity type.
 *
 * <p>Format {@code entity_dimensions} v1, after the header: u32 count, then per
 * type in registry order: u32 network id, string key, f32 width, f32 height,
 * f32 eye height, u8 fixed (1 if the size ignores scale).
 */
final class EntityDimensionsExporter {
    static final String FILE = "entity_dimensions.bin";

    private EntityDimensionsExporter() {}

    static void export(Path file) throws IOException {
        ExportWriter w = new ExportWriter("entity_dimensions", 1);
        w.u32(BuiltInRegistries.ENTITY_TYPE.size());
        for (EntityType<?> type : BuiltInRegistries.ENTITY_TYPE) {
            EntityDimensions d = type.getDimensions();
            w.u32(BuiltInRegistries.ENTITY_TYPE.getId(type));
            w.string(BuiltInRegistries.ENTITY_TYPE.getKey(type).toString());
            w.f32(d.width());
            w.f32(d.height());
            w.f32(d.eyeHeight());
            w.u8(d.fixed() ? 1 : 0);
        }
        w.writeTo(file);
    }
}
