/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.io.IOException;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.lang.reflect.ParameterizedType;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.syncher.EntityDataAccessor;
import net.minecraft.network.syncher.EntityDataSerializers;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityDimensions;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EntityTypes;

/**
 * What the client needs to know of every entity type to track it: its default size,
 * how often the server sends its position, which classes it is, the ids of its
 * synched-data fields and the size of its young.
 *
 * <p>Format {@code entity_dimensions} v2, after the header: u32 count, then per type
 * in registry order: u32 network id, string key, f32 width, f32 height, f32 eye
 * height, u8 fixed (1 if the size ignores scale), u32 update interval
 * ({@code Integer.MAX_VALUE} for none); u8 count and the simple names of the type's
 * class and its superclasses up to {@code Entity}, nearest first; u32 count and, for
 * each synched-data field those classes declare, string {@code Class.FIELD}, u8 id,
 * u8 serializer id; then u8 1, f32 width, f32 height, u8 fixed if one of the classes
 * declares a static {@code BABY_DIMENSIONS} (the nearest), else u8 0.
 *
 * <p>A type's class is the type argument of its {@code EntityTypes} constant; a type
 * without one gets no classes, fields or baby size.
 */
final class EntityDimensionsExporter {
    static final String FILE = "entity_dimensions.bin";

    private EntityDimensionsExporter() {}

    static void export(Path file) throws IOException, ReflectiveOperationException {
        Map<EntityType<?>, Class<?>> classes = typeClasses();
        ExportWriter w = new ExportWriter("entity_dimensions", 2);
        w.u32(BuiltInRegistries.ENTITY_TYPE.size());
        for (EntityType<?> type : BuiltInRegistries.ENTITY_TYPE) {
            EntityDimensions d = type.getDimensions();
            w.u32(BuiltInRegistries.ENTITY_TYPE.getId(type));
            w.string(BuiltInRegistries.ENTITY_TYPE.getKey(type).toString());
            w.f32(d.width());
            w.f32(d.height());
            w.f32(d.eyeHeight());
            w.u8(d.fixed() ? 1 : 0);
            w.u32(type.updateInterval());

            List<Class<?>> chain = new ArrayList<>();
            for (Class<?> c = classes.get(type); c != null && Entity.class.isAssignableFrom(c); c = c.getSuperclass()) {
                chain.add(c);
            }
            w.u8(chain.size());
            for (Class<?> c : chain) {
                w.string(c.getSimpleName());
            }

            ExportWriter fields = new ExportWriter(null, 0);
            int count = 0;
            EntityDimensions baby = null;
            for (Class<?> c : chain) {
                for (Field f : c.getDeclaredFields()) {
                    if (!Modifier.isStatic(f.getModifiers())) {
                        continue;
                    }
                    if (EntityDataAccessor.class.isAssignableFrom(f.getType())) {
                        f.setAccessible(true);
                        EntityDataAccessor<?> a = (EntityDataAccessor<?>) f.get(null);
                        fields.string(c.getSimpleName() + "." + f.getName());
                        fields.u8(a.id());
                        fields.u8(EntityDataSerializers.getSerializedId(a.serializer()));
                        count++;
                    } else if (baby == null && f.getName().equals("BABY_DIMENSIONS")
                            && f.getType() == EntityDimensions.class) {
                        f.setAccessible(true);
                        baby = (EntityDimensions) f.get(null);
                    }
                }
            }
            w.u32(count);
            w.bytes(fields.payload());
            if (baby != null) {
                w.u8(1);
                w.f32(baby.width());
                w.f32(baby.height());
                w.u8(baby.fixed() ? 1 : 0);
            } else {
                w.u8(0);
            }
        }
        w.writeTo(file);
    }

    /** Each type's class, from the type argument of its constant in EntityTypes. */
    private static Map<EntityType<?>, Class<?>> typeClasses() throws IllegalAccessException {
        Map<EntityType<?>, Class<?>> out = new IdentityHashMap<>();
        for (Field f : EntityTypes.class.getFields()) {
            if (Modifier.isStatic(f.getModifiers()) && f.getType() == EntityType.class
                    && f.getGenericType() instanceof ParameterizedType p
                    && p.getActualTypeArguments()[0] instanceof Class<?> c) {
                out.put((EntityType<?>) f.get(null), c);
            }
        }
        return out;
    }
}
