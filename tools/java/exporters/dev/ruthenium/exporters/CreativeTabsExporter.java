/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Proxy;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Optional;
import java.util.stream.Stream;

import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.component.DataComponentInitializers;
import net.minecraft.core.component.DataComponentPatch;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.NbtIo;
import net.minecraft.nbt.NbtOps;
import net.minecraft.nbt.Tag;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.CreativeModeTab;
import net.minecraft.world.item.Instrument;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.item.alchemy.PotionContents;
import net.minecraft.world.item.enchantment.Enchantment;
import net.minecraft.world.entity.decoration.painting.PaintingVariant;
import net.minecraft.world.level.ItemLike;
import net.minecraft.world.level.block.entity.BannerPatternLayers;

/**
 * The creative inventory's tabs ({@code CreativeModeTabs}) as the client builds them,
 * for the parts that don't come from the server's registries.
 *
 * <p>Each tab's generator is run with a registry lookup that answers the enchantment,
 * painting variant and instrument registries with nothing and notes where they were
 * asked for: those parts the client builds from the server's registries, and the
 * file marks their place. Everything else the generator gives (with every feature
 * flag on and with the operator's permission) is written as it is, with the tooltip
 * lines the creative search reads. The ominous banner is written without its
 * patterns, which name banner pattern registry entries.
 *
 * <p>Format {@code creative_tabs} v1, after the header:
 * <ul>
 * <li>u32 item count, then per item in registry order: u8 count and the names of the
 *     feature flags it requires.</li>
 * <li>u32 tab count, then per tab in registry order: string key; u8 row (0 top, 1
 *     bottom); u8 column; u8 type (0 category, 1 inventory, 2 hotbar, 3 search); u8
 *     flags (1 aligned right, 2 shows its title, 4 scrolls); its title (a text
 *     component); string background texture; u32 icon item; u8 1 if the entries need
 *     the operator's permission; u32 entry count, and each entry: u8 kind, then by kind
 *     <ul>
 *     <li>0, a stack: u32 item, the patch, u8 visibility (0 parent and search tabs, 1
 *         parent only, 2 search only), u8 count and the names of further feature flags
 *         it needs (a potion's), u32 count and its tooltip lines;</li>
 *     <li>1, the enchanted books (all enchantments at their highest level for the tab,
 *         every level for the search tab);</li>
 *     <li>2, the paintings: u8 1 for the placeable ones, 0 for the others;</li>
 *     <li>3, the instruments: u32 item, string the instrument tag;</li>
 *     <li>4, the ominous banner: u32 item, the patch without its patterns, u8 where its
 *         patterns go in the patch, u8 visibility, u32 count and its tooltip lines, u8
 *         count and each pattern: string key, u8 dye colour id.</li>
 *     </ul></li>
 * <li>Vanilla's entries, for those the server sends without data: u32 count, each
 *     enchantment: string key, u32 max level, its description; u32 count, each painting
 *     variant: string key, u8 width, u8 height, u8 has title and the title, u8 has
 *     author and the author; u32 count, each instrument: string key, its description.</li>
 * </ul>
 * A patch is u32 byte count and {@code DataComponentPatch.STREAM_CODEC}'s bytes; a text
 * component is u32 byte count and the component as network NBT.
 */
final class CreativeTabsExporter {
    static final String FILE = "creative_tabs.bin";

    private CreativeTabsExporter() {}

    /** A dynamic part asked for at this place in a tab's list. */
    private record Marker(ResourceKey<?> registry, int at) {}

    /** What a generator gave: stacks (with visibility) and markers. */
    private static final class Recorder {
        final List<Object[]> entries = new ArrayList<>();
        final List<Marker> markers = new ArrayList<>();
    }

    private static HolderLookup.Provider recording(HolderLookup.Provider real, Recorder rec) {
        return new HolderLookup.Provider() {
            @Override
            public Stream<ResourceKey<? extends Registry<?>>> listRegistryKeys() {
                return real.listRegistryKeys();
            }

            @Override
            public <T> Optional<? extends HolderLookup.RegistryLookup<T>> lookup(ResourceKey<? extends Registry<? extends T>> key) {
                Object k = key;
                if (k == Registries.ENCHANTMENT || k == Registries.PAINTING_VARIANT || k == Registries.INSTRUMENT) {
                    rec.markers.add(new Marker(key, rec.entries.size()));
                    return Optional.empty();
                }
                return real.lookup(key);
            }
        };
    }

    static void export(Path file) throws Exception {
        HolderLookup.Provider lookup = net.minecraft.data.registries.VanillaRegistries.createWorldLookup();
        BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(lookup).forEach(DataComponentInitializers.PendingComponents::apply);
        RegistryAccess access = RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);
        // The world lookup's tags can't be read (they are bound only once a data pack
        // loads), and an item's enchantments' tooltip reads #tooltip_order even when it
        // has none: here every tag reads as missing, which is what a stack with no
        // enchantments reads through to.
        HolderLookup.Provider noTags = new HolderLookup.Provider() {
            @Override
            public Stream<ResourceKey<? extends Registry<?>>> listRegistryKeys() {
                return lookup.listRegistryKeys();
            }

            @Override
            @SuppressWarnings("unchecked")
            public <T> Optional<? extends HolderLookup.RegistryLookup<T>> lookup(ResourceKey<? extends Registry<? extends T>> key) {
                return lookup.lookup(key).map(real -> (HolderLookup.RegistryLookup<T>) Proxy.newProxyInstance(
                    HolderLookup.RegistryLookup.class.getClassLoader(),
                    new Class<?>[] {HolderLookup.RegistryLookup.class},
                    (proxy, method, args) -> {
                        if (method.getName().equals("get") && args != null && args.length == 1
                            && args[0] instanceof net.minecraft.tags.TagKey<?>) {
                            return Optional.empty();
                        }
                        return method.invoke(real, args);
                    }));
            }
        };
        Item.TooltipContext tooltips = Item.TooltipContext.of(noTags);
        TooltipFlag flag = TooltipFlag.Default.NORMAL.asCreative();
        FeatureFlagSet all = FeatureFlags.REGISTRY.allFlags();

        Class<?> outputClass = Class.forName("net.minecraft.world.item.CreativeModeTab$Output");
        Object[] visibilities = Class.forName("net.minecraft.world.item.CreativeModeTab$TabVisibility").getEnumConstants();
        Field generatorField = CreativeModeTab.class.getDeclaredField("displayItemsGenerator");
        generatorField.setAccessible(true);
        Class<?> generatorClass = Class.forName("net.minecraft.world.item.CreativeModeTab$DisplayItemsGenerator");
        var accept = generatorClass.getMethod("accept", CreativeModeTab.ItemDisplayParameters.class, outputClass);
        accept.setAccessible(true);

        ExportWriter w = new ExportWriter("creative_tabs", 1);
        w.u32(BuiltInRegistries.ITEM.size());
        for (Item item : BuiltInRegistries.ITEM) {
            flags(w, item.requiredFeatures());
        }

        w.u32(BuiltInRegistries.CREATIVE_MODE_TAB.size());
        for (CreativeModeTab tab : BuiltInRegistries.CREATIVE_MODE_TAB) {
            w.string(BuiltInRegistries.CREATIVE_MODE_TAB.getKey(tab).toString());
            w.u8(tab.row().ordinal());
            w.u8(tab.column());
            w.u8(tab.getType().ordinal());
            w.u8((tab.isAlignedRight() ? 1 : 0) | (tab.showTitle() ? 2 : 0) | (tab.canScroll() ? 4 : 0));
            text(w, tab.getDisplayName(), lookup);
            w.string(tab.getBackgroundTexture().toString());
            w.u32(BuiltInRegistries.ITEM.getId(tab.getIconItem().getItem()));
            Object generator = generatorField.get(tab);
            Recorder with = run(generator, accept, outputClass, visibilities, all, true, lookup);
            Recorder without = run(generator, accept, outputClass, visibilities, all, false, lookup);
            boolean permission;
            if (without.entries.isEmpty() && without.markers.isEmpty()) {
                permission = !with.entries.isEmpty() || !with.markers.isEmpty();
            } else if (without.entries.size() == with.entries.size() && without.markers.equals(with.markers)) {
                permission = false;
            } else {
                throw new IllegalStateException("tab " + tab.getDisplayName().getString() + " is partly for operators");
            }
            w.u8(permission ? 1 : 0);
            String key = BuiltInRegistries.CREATIVE_MODE_TAB.getKey(tab).getPath();
            List<byte[]> out = new ArrayList<>();
            int m = 0;
            for (int i = 0; i <= with.entries.size(); i++) {
                while (m < with.markers.size() && with.markers.get(m).at() == i) {
                    ResourceKey<?> registry = with.markers.get(m++).registry();
                    ExportWriter e = new ExportWriter(null, 0);
                    if (registry == Registries.ENCHANTMENT) {
                        // CreativeModeTabs.java:1503: the highest level for the tab, every
                        // level for the search tab.
                        e.u8(1);
                    } else if (registry == Registries.PAINTING_VARIANT) {
                        // CreativeModeTabs.java:856 (functional: the placeable ones) and
                        // 1638 (op: the others).
                        e.u8(2);
                        e.u8(key.equals("op_blocks") ? 0 : 1);
                    } else {
                        // CreativeModeTabs.java:1159: goat horns of #goat_horns.
                        e.u8(3);
                        e.u32(BuiltInRegistries.ITEM.getId(Items.GOAT_HORN));
                        e.string("minecraft:goat_horns");
                    }
                    out.add(e.payload());
                }
                if (i == with.entries.size()) break;
                ItemStack stack = (ItemStack) with.entries.get(i)[0];
                int visibility = (Integer) with.entries.get(i)[1];
                if (stack.getCount() != 1) throw new IllegalStateException("a stack of " + stack.getCount());
                ExportWriter e = new ExportWriter(null, 0);
                BannerPatternLayers patterns = stack.getComponentsPatch().split().added().get(DataComponents.BANNER_PATTERNS);
                if (patterns != null) {
                    e.u8(4);
                    e.u32(BuiltInRegistries.ITEM.getId(stack.getItem()));
                    DataComponentPatch patch = stack.getComponentsPatch();
                    int at = 0;
                    // The order STREAM_CODEC writes them in (`fastIterable` of the patch's map).
                    Field mapField = DataComponentPatch.class.getDeclaredField("map");
                    mapField.setAccessible(true);
                    for (Object type : ((java.util.Map<?, ?>) mapField.get(patch)).keySet()) {
                        if (type == DataComponents.BANNER_PATTERNS) break;
                        at++;
                    }
                    patch(e, patch.forget(t -> t == DataComponents.BANNER_PATTERNS), access);
                    e.u8(at);
                    e.u8(visibility);
                    lines(e, stack.getTooltipLines(tooltips, null, flag), lookup);
                    e.u8(patterns.layers().size());
                    for (BannerPatternLayers.Layer layer : patterns.layers()) {
                        e.string(layer.pattern().unwrapKey().orElseThrow().identifier().toString());
                        e.u8(layer.color().getId());
                    }
                } else {
                    e.u8(0);
                    e.u32(BuiltInRegistries.ITEM.getId(stack.getItem()));
                    patch(e, stack.getComponentsPatch(), access);
                    e.u8(visibility);
                    PotionContents potion = stack.get(DataComponents.POTION_CONTENTS);
                    FeatureFlagSet extra = potion != null && potion.potion().isPresent()
                        ? potion.potion().get().value().requiredFeatures() : FeatureFlagSet.of();
                    flags(e, extra);
                    lines(e, stack.getTooltipLines(tooltips, null, flag), lookup);
                }
                out.add(e.payload());
            }
            w.u32(out.size());
            for (byte[] b : out) w.bytes(b);
        }

        var enchantments = lookup.lookupOrThrow(Registries.ENCHANTMENT).listElements().toList();
        w.u32(enchantments.size());
        for (var e : enchantments) {
            Enchantment v = e.value();
            w.string(e.key().identifier().toString());
            w.u32(v.getMaxLevel());
            text(w, v.description(), lookup);
        }
        var paintings = lookup.lookupOrThrow(Registries.PAINTING_VARIANT).listElements().toList();
        w.u32(paintings.size());
        for (var p : paintings) {
            PaintingVariant v = p.value();
            w.string(p.key().identifier().toString());
            w.u8(v.width());
            w.u8(v.height());
            w.u8(v.title().isPresent() ? 1 : 0);
            if (v.title().isPresent()) text(w, v.title().get(), lookup);
            w.u8(v.author().isPresent() ? 1 : 0);
            if (v.author().isPresent()) text(w, v.author().get(), lookup);
        }
        var instruments = lookup.lookupOrThrow(Registries.INSTRUMENT).listElements().toList();
        w.u32(instruments.size());
        for (var i : instruments) {
            Instrument v = i.value();
            w.string(i.key().identifier().toString());
            text(w, v.description(), lookup);
        }
        w.writeTo(file);
    }

    private static Recorder run(Object generator, java.lang.reflect.Method accept, Class<?> outputClass, Object[] visibilities,
                                FeatureFlagSet features, boolean permission, HolderLookup.Provider lookup) throws Exception {
        Recorder rec = new Recorder();
        InvocationHandler h = (proxy, method, args) -> {
            switch (method.getName()) {
                case "accept" -> {
                    Object what = args[0];
                    ItemStack stack = what instanceof ItemStack s ? s : new ItemStack((ItemLike) what);
                    int visibility = args.length > 1 ? ((Enum<?>) args[1]).ordinal() : 0;
                    rec.entries.add(new Object[] {stack, visibility});
                }
                case "acceptAll" -> {
                    int visibility = args.length > 1 ? ((Enum<?>) args[1]).ordinal() : 0;
                    for (Object s : (Collection<?>) args[0]) rec.entries.add(new Object[] {s, visibility});
                }
                case "hashCode" -> { return System.identityHashCode(proxy); }
                case "equals" -> { return proxy == args[0]; }
                case "toString" -> { return "recorder"; }
                default -> throw new IllegalStateException(method.getName());
            }
            return null;
        };
        Object output = Proxy.newProxyInstance(outputClass.getClassLoader(), new Class<?>[] {outputClass}, h);
        accept.invoke(generator, new CreativeModeTab.ItemDisplayParameters(features, permission, recording(lookup, rec)), output);
        return rec;
    }

    private static void flags(ExportWriter w, FeatureFlagSet set) {
        var names = FeatureFlags.REGISTRY.toNames(set).stream().map(Object::toString).sorted().toList();
        w.u8(names.size());
        for (String n : names) w.string(n);
    }

    private static void patch(ExportWriter w, DataComponentPatch patch, RegistryAccess access) {
        ByteBuf raw = Unpooled.buffer();
        DataComponentPatch.STREAM_CODEC.encode(new RegistryFriendlyByteBuf(raw, access), patch);
        byte[] b = new byte[raw.readableBytes()];
        raw.readBytes(b);
        w.u32(b.length);
        w.bytes(b);
    }

    private static void lines(ExportWriter w, List<Component> lines, HolderLookup.Provider lookup) throws Exception {
        w.u32(lines.size());
        for (Component c : lines) text(w, c, lookup);
    }

    static void text(ExportWriter w, Component c, HolderLookup.Provider lookup) throws Exception {
        Tag tag = ComponentSerialization.CODEC.encodeStart(lookup.createSerializationContext(NbtOps.INSTANCE), c).getOrThrow();
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        NbtIo.writeAnyTag(tag, new DataOutputStream(bytes));
        byte[] b = bytes.toByteArray();
        w.u32(b.length);
        w.bytes(b);
    }
}
