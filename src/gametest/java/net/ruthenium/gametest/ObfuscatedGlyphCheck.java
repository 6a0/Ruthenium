package net.ruthenium.gametest;

import it.unimi.dsi.fastutil.ints.Int2ObjectMap;
import it.unimi.dsi.fastutil.ints.IntList;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.font.FontManager;
import net.minecraft.client.gui.font.FontSet;
import net.minecraft.client.gui.font.glyphs.BakedGlyph;
import net.minecraft.client.gui.font.glyphs.SpecialGlyphs;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.minecraft.util.Mth;
import net.minecraft.util.RandomSource;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.LazyGlyphWidthsHolder;
import net.ruthenium.render.text.LazyGlyphWidths;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;

/**
 * Checks the {@code lazyGlyphWidths} switch ({@code -Pbench.glyphWidthCheck=true} in the footprint scene):
 * <ul>
 *     <li>{@link #screenshots}: in the world, an obfuscated title over a plain subtitle, two screenshots apart;</li>
 *     <li>{@link #run}: at the title screen, the glyphs by width of every font set built by vanilla (switch off) and
 *     lazily (switch on) are equal, code point order included; with the switch on nothing is built by a reload, eight
 *     threads asking for the table at once all get the same one, and random glyphs have the width they replace.</li>
 * </ul>
 */
final class ObfuscatedGlyphCheck {
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Obfuscated Glyph Check");
    private static final String SAMPLE = "Hello, World! 0123 WMil.";

    private ObfuscatedGlyphCheck() {
    }

    static void screenshots(ClientGameTestContext context, String label, String runId) {
        context.runOnClient(mc -> {
            mc.gui.hud.setTimes(0, 400, 0);
            mc.gui.hud.setSubtitle(Component.literal(SAMPLE));
            mc.gui.hud.setTitle(Component.literal(SAMPLE).withStyle(ChatFormatting.OBFUSCATED));
        });
        context.waitTicks(5);
        context.takeScreenshot(label + "-obfuscated1-" + runId);
        context.waitTicks(5);
        context.takeScreenshot(label + "-obfuscated2-" + runId);
        context.runOnClient(mc -> mc.gui.hud.clearTitles());
    }

    static void run(ClientGameTestContext context) {
        boolean previous = Initializer.CONFIG.lazyGlyphWidths;
        try {
            Initializer.CONFIG.lazyGlyphWidths = false;
            FootprintBenchmark.reload(context);
            Map<Identifier, Map<Integer, int[]>> vanilla = context.computeOnClient(mc -> {
                Map<Identifier, Map<Integer, int[]>> tables = new HashMap<>();
                fontSets(mc).forEach((id, fontSet) -> {
                    check(((LazyGlyphWidthsHolder) fontSet).ruthenium$lazyGlyphWidths() == null, id + ": lazy table with the switch off");
                    tables.put(id, copy(vanillaTable(fontSet)));
                });
                return tables;
            });

            Initializer.CONFIG.lazyGlyphWidths = true;
            FootprintBenchmark.reload(context);
            context.runOnClient(mc -> {
                Map<Identifier, FontSet> fontSets = fontSets(mc);
                check(fontSets.keySet().equals(vanilla.keySet()), "font sets changed: " + fontSets.keySet() + " vs " + vanilla.keySet());
                for (Map.Entry<Identifier, FontSet> entry : fontSets.entrySet()) {
                    LazyGlyphWidths lazy = ((LazyGlyphWidthsHolder) entry.getValue()).ruthenium$lazyGlyphWidths();
                    check(lazy != null, entry.getKey() + ": no lazy table");
                    check(!lazy.isBuilt(), entry.getKey() + ": lazy table built by the reload");
                    check(vanillaTable(entry.getValue()).isEmpty(), entry.getKey() + ": vanilla table filled with the switch on");
                }

                LazyGlyphWidths defaultTable = ((LazyGlyphWidthsHolder) fontSets.get(Minecraft.DEFAULT_FONT)).ruthenium$lazyGlyphWidths();
                long start = System.nanoTime();
                Int2ObjectMap<IntList> concurrent = buildConcurrently(defaultTable);
                LOGGER.info("Default font's table built in {} ms by 8 threads at once", "%.1f".formatted((System.nanoTime() - start) / 1.0e6));
                check(defaultTable.widths() == concurrent, "a second table after the concurrent build");

                int codePoints = 0;
                for (Map.Entry<Identifier, FontSet> entry : fontSets.entrySet()) {
                    Map<Integer, int[]> lazy = copy(((LazyGlyphWidthsHolder) entry.getValue()).ruthenium$lazyGlyphWidths().widths());
                    Map<Integer, int[]> expected = vanilla.get(entry.getKey());
                    check(lazy.keySet().equals(expected.keySet()), entry.getKey() + ": widths " + lazy.keySet() + " vs " + expected.keySet());
                    for (Map.Entry<Integer, int[]> width : expected.entrySet()) {
                        check(Arrays.equals(width.getValue(), lazy.get(width.getKey())), entry.getKey() + ": code points of width " + width.getKey() + " differ");
                        codePoints += width.getValue().length;
                    }
                }
                LOGGER.info("Lazy tables equal vanilla's for {} font sets ({} code points)", fontSets.size(), codePoints);

                FontSet fontSet = fontSets.get(Minecraft.DEFAULT_FONT);
                RandomSource random = RandomSource.create(1);
                StringBuilder widths = new StringBuilder();
                SAMPLE.codePoints().distinct().filter(cp -> cp != ' ').forEach(cp -> {
                    int width = Mth.ceil(fontSet.source(false).getGlyph(cp).info().getAdvance(false));
                    Set<BakedGlyph> seen = Collections.newSetFromMap(new IdentityHashMap<>());
                    for (int i = 0; i < 64; i++) {
                        BakedGlyph glyph = fontSet.getRandomGlyph(random, width);
                        check(glyph.info() != SpecialGlyphs.MISSING, "missing random glyph for '" + Character.toString(cp) + "'");
                        check(Mth.ceil(glyph.info().getAdvance(false)) == width, "random glyph for '" + Character.toString(cp)
                                + "' is " + glyph.info().getAdvance(false) + " wide, not " + width);
                        seen.add(glyph);
                    }
                    check(seen.size() > 1, "only one random glyph for width " + width);
                    widths.append(Character.toString(cp)).append('=').append(width).append(':').append(seen.size()).append(' ');
                });
                LOGGER.info("Random glyphs (char=width:distinct of 64): {}", widths);
            });
            LOGGER.info("Obfuscated glyph check passed");
        } finally {
            Initializer.CONFIG.lazyGlyphWidths = previous;
        }
    }

    private static Int2ObjectMap<IntList> buildConcurrently(LazyGlyphWidths table) {
        int threads = 8;
        ExecutorService executor = Executors.newFixedThreadPool(threads);
        try {
            CountDownLatch start = new CountDownLatch(1);
            List<Future<Int2ObjectMap<IntList>>> results = new ArrayList<>();
            for (int i = 0; i < threads; i++) {
                results.add(executor.submit(() -> {
                    start.await();
                    return table.widths();
                }));
            }
            start.countDown();
            Int2ObjectMap<IntList> first = results.getFirst().get();
            for (Future<Int2ObjectMap<IntList>> result : results) {
                check(result.get() == first, "threads got different tables");
            }
            return first;
        } catch (Exception e) {
            throw new RuntimeException(e);
        } finally {
            executor.shutdownNow();
        }
    }

    @SuppressWarnings("unchecked")
    private static Map<Identifier, FontSet> fontSets(Minecraft mc) {
        try {
            Field managerField = Minecraft.class.getDeclaredField("fontManager");
            managerField.setAccessible(true);
            FontManager manager = (FontManager) managerField.get(mc);
            Field setsField = FontManager.class.getDeclaredField("fontSets");
            setsField.setAccessible(true);
            return new HashMap<>((Map<Identifier, FontSet>) setsField.get(manager));
        } catch (ReflectiveOperationException e) {
            throw new RuntimeException(e);
        }
    }

    @SuppressWarnings("unchecked")
    private static Int2ObjectMap<IntList> vanillaTable(FontSet fontSet) {
        try {
            Field field = FontSet.class.getDeclaredField("glyphsByWidth");
            field.setAccessible(true);
            return (Int2ObjectMap<IntList>) field.get(fontSet);
        } catch (ReflectiveOperationException e) {
            throw new RuntimeException(e);
        }
    }

    private static Map<Integer, int[]> copy(Int2ObjectMap<IntList> table) {
        Map<Integer, int[]> copy = new TreeMap<>();
        table.int2ObjectEntrySet().forEach(entry -> copy.put(entry.getIntKey(), entry.getValue().toIntArray()));
        return copy;
    }

    private static void check(boolean condition, String message) {
        if (!condition) {
            throw new AssertionError(message);
        }
    }
}
