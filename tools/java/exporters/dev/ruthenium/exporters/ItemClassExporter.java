/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
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
 * <p>Format {@code item_classes} v2, after the header: u32 count, then per item in
 * registry order: u32 network id, string key; u8 count and the simple names of the
 * item's class and its superclasses up to {@code Item}, nearest first; u8 flags (1: a
 * class below {@code Item} declares {@code use}, 2: {@code useOn}, 4:
 * {@code interactLivingEntity}, 8: {@code releaseUsing}, 16: the item is a
 * {@code BlockItem}); with flag 16, u32 the block it places (a block registry id);
 * then u16 count and that many pairs of strings: the {@code Class.field} and value of
 * each instance field the item's classes declare below {@code Item} (written as the
 * block exporter writes them: a standing-and-wall item's wall block and attachment
 * direction, a bucket's fluid, an entity item's type).
 */
final class ItemClassExporter {
    static final String FILE = "item_classes.bin";

    private ItemClassExporter() {}

    static void export(Path file) throws Exception {
        ExportWriter w = new ExportWriter("item_classes", 2);
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
            if (item instanceof BlockItem blockItem) {
                w.u32(BuiltInRegistries.BLOCK.getId(blockItem.getBlock()));
            }
            List<String[]> fields = new ArrayList<>();
            for (Class<?> c : chain) {
                if (c == Item.class) continue;
                for (Field f : c.getDeclaredFields()) {
                    if (Modifier.isStatic(f.getModifiers()) || f.isSynthetic()) continue;
                    f.setAccessible(true);
                    fields.add(new String[] {c.getSimpleName() + "." + f.getName(), BlockBehaviourExporter.render(f.get(item), 0)});
                }
            }
            w.u8(fields.size() & 0xFF);
            w.u8(fields.size() >>> 8);
            for (String[] f : fields) {
                w.string(f[0]);
                w.string(f[1]);
            }
        }
        w.writeTo(file);
    }
}
