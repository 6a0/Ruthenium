/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.io.IOException;
import java.lang.reflect.Method;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;

/**
 * What the client's item use needs to know of every item that its components don't
 * say: which classes it is (a bow, a throwable, a block item and so on) and which of
 * the use methods they override.
 *
 * <p>Format {@code item_classes} v1, after the header: u32 count, then per item in
 * registry order: u32 network id, string key; u8 count and the simple names of the
 * item's class and its superclasses up to {@code Item}, nearest first; u8 flags (1: a
 * class below {@code Item} declares {@code use}, 2: {@code useOn}, 4:
 * {@code interactLivingEntity}, 8: {@code releaseUsing}, 16: the item is a
 * {@code BlockItem}).
 */
final class ItemClassExporter {
    static final String FILE = "item_classes.bin";

    private ItemClassExporter() {}

    static void export(Path file) throws IOException {
        ExportWriter w = new ExportWriter("item_classes", 1);
        w.u32(BuiltInRegistries.ITEM.size());
        for (Item item : BuiltInRegistries.ITEM) {
            w.u32(BuiltInRegistries.ITEM.getId(item));
            w.string(BuiltInRegistries.ITEM.getKey(item).toString());
            List<Class<?>> chain = new ArrayList<>();
            for (Class<?> c = item.getClass(); c != null && Item.class.isAssignableFrom(c); c = c.getSuperclass()) {
                chain.add(c);
            }
            w.u8(chain.size());
            int flags = 0;
            for (Class<?> c : chain) {
                w.string(c.getSimpleName());
                if (c == Item.class) continue;
                for (Method m : c.getDeclaredMethods()) {
                    if (m.isSynthetic() || m.isBridge()) continue;
                    switch (m.getName()) {
                        case "use" -> flags |= 1;
                        case "useOn" -> flags |= 2;
                        case "interactLivingEntity" -> flags |= 4;
                        case "releaseUsing" -> flags |= 8;
                        default -> { }
                    }
                }
            }
            if (item instanceof BlockItem) flags |= 16;
            w.u8(flags);
        }
        w.writeTo(file);
    }
}
