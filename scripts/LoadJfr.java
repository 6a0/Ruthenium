import jdk.jfr.consumer.RecordedEvent;
import jdk.jfr.consumer.RecordedFrame;
import jdk.jfr.consumer.RecordedStackTrace;
import jdk.jfr.consumer.RecordedThread;
import jdk.jfr.consumer.RecordingFile;

import java.nio.file.Path;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * Load time breakdowns from a JFR recording (startup, resource reloads, joins), by JVM uptime windows in milliseconds
 * (the footprint scene logs the uptime of each reload and join; JFR's recording starts with the JVM).
 * <p>
 * Usage: java scripts/LoadJfr.java <file.jfr> <command> [args]
 * <ul>
 *     <li>{@code threads FROM TO}: per thread, Java and native execution samples, parked, monitor and sleep time.</li>
 *     <li>{@code phases THREAD FROM TO}: samples of a thread (Java and native) by phase (the innermost frame that
 *     matches a phase rule) and by kind (class loading, Mixin, Fabric Loader, or the code itself), in ms at the
 *     sampling period. THREAD may be a prefix ending in '*' (e.g. {@code Worker-Main*}).</li>
 *     <li>{@code timeline THREAD FROM TO BUCKET}: per bucket of BUCKET ms, the thread's samples, its top phase and its
 *     parked time.</li>
 *     <li>{@code parks THREAD FROM TO}: where the thread parked or waited (top Minecraft frame), with total ms.</li>
 *     <li>{@code top THREAD FROM TO [N]}: the N methods with most inclusive samples (default 40), Minecraft and
 *     library frames only.</li>
 * </ul>
 * The profile setting samples Java code every 10 ms (20 ms for {@code -PbenchJfr}'s native samples), so a sample is
 * worth about 10 ms of one thread.
 */
public class LoadJfr {
    /** Phase rules, most specific first among overlapping ones: the innermost (closest to the top) matching frame wins */
    private static final String[][] PHASES = {
            {"net.fabricmc.loader.impl.launch.knot.Knot.init", "fabric:knot-init"},
            {"net.fabricmc.loader.impl.FabricLoaderImpl.load", "fabric:discover+resolve"},
            {"net.fabricmc.loader.impl.FabricLoaderImpl.setup", "fabric:setup"},
            {"net.fabricmc.loader.impl.entrypoint.EntrypointUtils.invoke", "mod entrypoints"},
            {"org.spongepowered.asm.mixin.transformer.MixinProcessor.selectConfigs", "mixin:prepare configs"},
            {"org.spongepowered.asm.launch.MixinBootstrap", "mixin:bootstrap"},
            {"net.minecraft.util.datafix.DataFixers.<clinit>", "DFU: build schemas"},
            {"net.minecraft.util.datafix.DataFixers.optimize", "DFU: optimize"},
            {"com.mojang.datafixers.DataFixerUpper.lambda$optimize", "DFU: optimize"},
            {"com.mojang.datafixers.DataFixerUpper.getRule", "DFU: rule"},
            {"net.minecraft.server.Bootstrap.bootStrap", "Bootstrap (registries)"},
            {"net.minecraft.client.ClientBootstrap.bootstrap", "ClientBootstrap"},
            {"net.minecraft.server.Bootstrap.validate", "Bootstrap.validate"},
            {"net.minecraft.client.main.Main.main", "Main (other)"},
            {"net.minecraft.client.Minecraft.<init>", "Minecraft.<init> (other)"},
            {"net.minecraft.client.Options.<init>", "Options"},
            {"com.mojang.blaze3d.vulkan", "Vulkan backend"},
            {"com.mojang.blaze3d.opengl", "OpenGL backend"},
            {"com.mojang.blaze3d.platform.Window", "Window"},
            {"net.minecraft.client.renderer.ShaderManager", "shaders"},
            {"net.minecraft.client.resources.model.ModelManager", "models"},
            {"net.minecraft.client.resources.model.ModelBakery", "models: bake"},
            {"net.minecraft.client.resources.model.ModelDiscovery", "models: discover"},
            {"net.minecraft.client.resources.model.BlockStateModelLoader", "models: blockstates"},
            {"net.minecraft.client.resources.model.ClientItemInfoLoader", "models: items"},
            {"net.minecraft.client.renderer.texture.SpriteLoader", "atlas: load"},
            {"net.minecraft.client.renderer.texture.Stitcher", "atlas: stitch"},
            {"net.minecraft.client.renderer.texture.SpriteContents", "atlas: sprite"},
            {"net.minecraft.client.renderer.texture.MipmapGenerator", "atlas: mipmaps"},
            {"net.minecraft.client.resources.model.AtlasManager", "atlas: manager"},
            {"net.minecraft.client.renderer.texture.TextureAtlas", "atlas: upload"},
            {"net.minecraft.client.sounds", "sounds"},
            {"com.mojang.blaze3d.audio", "sounds: openal"},
            {"net.minecraft.client.gui.font", "fonts"},
            {"net.minecraft.client.resources.language", "language"},
            {"net.minecraft.client.renderer.texture.TextureManager", "textures"},
            {"net.minecraft.client.resources.SplashManager", "splashes"},
            {"net.minecraft.client.renderer.block.model.BlockModelShaper", "models: shaper"},
            {"net.minecraft.client.renderer.entity.EntityRenderDispatcher", "entity renderers"},
            {"net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher", "block entity renderers"},
            {"net.minecraft.client.model.geom.EntityModelSet", "entity models"},
            {"net.minecraft.client.model.geom.LayerDefinitions", "entity models: layers"},
            {"net.minecraft.server.packs.resources.SimpleReloadInstance", "reload instance (other)"},
            {"net.minecraft.server.packs.resources.ProfiledReloadInstance", "reload instance (other)"},
            {"com.mojang.renderpearl.frontend.shaders", "shaders: compile"},
            {"org.lwjgl.util.shaderc", "shaders: compile"},
            {"com.mojang.renderpearl", "renderpearl"},
            {"net.minecraft.server.packs.repository.PackRepository", "packs: repository"},
            {"net.minecraft.server.packs.resources.MultiPackResourceManager", "packs: resource manager"},
            {"net.minecraft.server.packs.resources.FallbackResourceManager", "packs: list/get resources"},
            {"net.minecraft.server.packs", "packs"},
            {"net.fabricmc.fabric.impl.resource", "fabric resource loader"},
            {"net.minecraft.client.gui.screens.TitleScreen", "title screen"},
            {"net.minecraft.client.renderer.GameRenderer.render", "render"},
            {"net.minecraft.client.Minecraft.runTick", "runTick (other)"},
            {"net.minecraft.server.MinecraftServer.loadLevel", "server: loadLevel"},
            {"net.minecraft.server.MinecraftServer.createLevels", "server: createLevels"},
            {"net.minecraft.server.MinecraftServer.prepareLevels", "server: prepareLevels"},
            {"net.minecraft.server.WorldLoader", "server: WorldLoader"},
            {"net.minecraft.server.ReloadableServerResources", "server: datapack reload"},
            {"net.minecraft.world.item.crafting.RecipeManager", "recipes"},
            {"net.minecraft.tags", "tags"},
            {"net.minecraft.world.level.levelgen", "worldgen"},
            {"net.minecraft.world.level.chunk.storage", "chunk storage"},
            {"net.minecraft.server.level.ChunkMap", "server: ChunkMap"},
            {"net.minecraft.server.level.ServerChunkCache", "server: chunk cache"},
            {"net.minecraft.world.level.lighting", "lighting"},
            {"net.minecraft.client.renderer.chunk", "section compile"},
            {"net.minecraft.client.multiplayer.ClientPacketListener", "client packets"},
            {"net.minecraft.network", "network"},
            {"net.minecraft.util.datafix", "DFU (minecraft)"},
            {"com.mojang.datafixers", "DFU (library)"},
            {"com.mojang.serialization", "codecs"},
            {"net.ruthenium", "ruthenium"},
    };

    private static final String[][] KINDS = {
            {"org.spongepowered.asm", "mixin"},
            {"com.llamalad7.mixinextras", "mixin"},
            {"net.fabricmc.loader.impl.launch.knot.KnotClassDelegate", "classload"},
            {"net.fabricmc.loader.impl.launch.knot.KnotClassLoader", "classload"},
            {"java.lang.ClassLoader.defineClass", "classload"},
            {"java.lang.ClassLoader.loadClass", "classload"},
            {"jdk.internal.loader", "classload"},
            {"java.lang.invoke", "invoke/lambda"},
            {"net.fabricmc.loader", "fabric loader"},
            {"net.fabricmc.accesswidener", "fabric loader"},
            {"net.fabricmc.tinyremapper", "fabric loader"},
            {"java.util.zip", "zip"},
            {"jdk.nio.zipfs", "zip"},
    };

    public static void main(String[] args) throws Exception {
        Path file = Path.of(args[0]);
        String command = args[1];
        List<RecordedEvent> events = new ArrayList<>();
        Instant jvmStart = null;
        for (RecordedEvent e : RecordingFile.readAllEvents(file)) {
            String name = e.getEventType().getName();
            if (name.equals("jdk.JVMInformation")) {
                jvmStart = e.getInstant("jvmStartTime");
            }
            switch (name) {
                case "jdk.ExecutionSample", "jdk.NativeMethodSample", "jdk.ThreadPark", "jdk.JavaMonitorEnter",
                     "jdk.JavaMonitorWait", "jdk.ThreadSleep" -> events.add(e);
                default -> {
                }
            }
        }
        if (jvmStart == null) {
            throw new IllegalStateException("No jdk.JVMInformation event");
        }
        switch (command) {
            case "threads" -> threads(events, jvmStart, ms(args[2]), ms(args[3]));
            case "phases" -> phases(events, jvmStart, args[2], ms(args[3]), ms(args[4]));
            case "timeline" -> timeline(events, jvmStart, args[2], ms(args[3]), ms(args[4]), Long.parseLong(args[5]));
            case "parks" -> parks(events, jvmStart, args[2], ms(args[3]), ms(args[4]));
            case "top" -> top(events, jvmStart, args[2], ms(args[3]), ms(args[4]), args.length > 5 ? Integer.parseInt(args[5]) : 40);
            case "tree" -> tree(events, jvmStart, args[2], ms(args[3]), ms(args[4]), args[5], Integer.parseInt(args[6]),
                    args.length > 7 ? Double.parseDouble(args[7]) : 1.0);
            default -> throw new IllegalArgumentException(command);
        }
    }

    private static long ms(String s) {
        return Long.parseLong(s);
    }

    private static long uptime(Instant jvmStart, Instant t) {
        return Duration.between(jvmStart, t).toMillis();
    }

    private static RecordedThread threadOf(RecordedEvent e) {
        String name = e.getEventType().getName();
        if (name.equals("jdk.ExecutionSample") || name.equals("jdk.NativeMethodSample")) {
            return e.getThread("sampledThread");
        }
        return e.getThread();
    }

    private static boolean matches(String pattern, RecordedThread t) {
        if (t == null || t.getJavaName() == null) {
            return false;
        }
        return pattern.endsWith("*") ? t.getJavaName().startsWith(pattern.substring(0, pattern.length() - 1)) : t.getJavaName().equals(pattern);
    }

    private static String threadGroupName(String name) {
        return name == null ? "?" : name.replaceAll("-\\d+$", "-N").replaceAll("#\\d+$", "#N");
    }

    /** Overlap in ms of an event [start, end) with the window */
    private static double overlap(Instant jvmStart, RecordedEvent e, long from, long to) {
        long s = uptime(jvmStart, e.getStartTime());
        long end = uptime(jvmStart, e.getEndTime());
        return Math.max(0, Math.min(end, to) - Math.max(s, from));
    }

    private static void threads(List<RecordedEvent> events, Instant jvmStart, long from, long to) {
        Map<String, double[]> rows = new TreeMap<>();
        for (RecordedEvent e : events) {
            RecordedThread t = threadOf(e);
            if (t == null) {
                continue;
            }
            String key = threadGroupName(t.getJavaName());
            double[] row = rows.computeIfAbsent(key, _ -> new double[6]);
            String name = e.getEventType().getName();
            if (name.endsWith("Sample")) {
                long time = uptime(jvmStart, e.getStartTime());
                if (time < from || time >= to) {
                    continue;
                }
                row[name.equals("jdk.ExecutionSample") ? 0 : 1]++;
            } else {
                double o = overlap(jvmStart, e, from, to);
                switch (name) {
                    case "jdk.ThreadPark" -> row[2] += o;
                    case "jdk.JavaMonitorEnter" -> row[3] += o;
                    case "jdk.JavaMonitorWait" -> row[4] += o;
                    default -> row[5] += o;
                }
            }
        }
        System.out.printf("window %d..%d ms (%d ms)%n", from, to, to - from);
        System.out.printf("%-45s %8s %8s %9s %9s %9s %9s%n", "thread", "java", "native", "park ms", "monitor", "wait", "sleep");
        rows.entrySet().stream()
                .filter(en -> en.getValue()[0] + en.getValue()[1] > 0 || en.getValue()[2] + en.getValue()[3] + en.getValue()[4] > 50)
                .sorted((a, b) -> Double.compare(b.getValue()[0] + b.getValue()[1], a.getValue()[0] + a.getValue()[1]))
                .forEach(en -> {
                    double[] r = en.getValue();
                    System.out.printf("%-45s %8.0f %8.0f %9.0f %9.0f %9.0f %9.0f%n", en.getKey(), r[0], r[1], r[2], r[3], r[4], r[5]);
                });
    }

    static String frameName(RecordedFrame f) {
        return f.getMethod().getType().getName() + "." + f.getMethod().getName();
    }

    static String phaseOf(RecordedStackTrace stack) {
        if (stack == null) {
            return "?";
        }
        // innermost match: from the top frame down
        for (RecordedFrame f : stack.getFrames()) {
            String n = frameName(f);
            for (String[] rule : PHASES) {
                if (n.startsWith(rule[0])) {
                    return rule[1];
                }
            }
        }
        List<RecordedFrame> frames = stack.getFrames();
        return frames.isEmpty() ? "?" : "other: " + frameName(frames.get(frames.size() - 1));
    }

    static String kindOf(RecordedStackTrace stack) {
        if (stack == null) {
            return "?";
        }
        for (RecordedFrame f : stack.getFrames()) {
            String n = frameName(f);
            if (n.endsWith(".<clinit>")) {
                return "code";
            }
            if (n.startsWith("net.minecraft") || n.startsWith("com.mojang") || n.startsWith("net.ruthenium")) {
                // a Minecraft frame above any loader frame: the code itself
                return "code";
            }
            for (String[] rule : KINDS) {
                if (n.startsWith(rule[0])) {
                    return rule[1];
                }
            }
        }
        return "code";
    }

    private static void phases(List<RecordedEvent> events, Instant jvmStart, String thread, long from, long to) {
        Map<String, int[]> byPhase = new HashMap<>();
        Map<String, int[]> byKind = new HashMap<>();
        Map<String, Map<String, int[]>> kindByPhase = new HashMap<>();
        int java = 0, nat = 0;
        for (RecordedEvent e : events) {
            String name = e.getEventType().getName();
            if (!name.endsWith("Sample") || !matches(thread, threadOf(e))) {
                continue;
            }
            long time = uptime(jvmStart, e.getStartTime());
            if (time < from || time >= to) {
                continue;
            }
            boolean isJava = name.equals("jdk.ExecutionSample");
            if (isJava) {
                java++;
            } else {
                nat++;
            }
            String phase = phaseOf(e.getStackTrace());
            String kind = isJava ? kindOf(e.getStackTrace()) : "native";
            byPhase.computeIfAbsent(phase, _ -> new int[1])[0]++;
            byKind.computeIfAbsent(kind, _ -> new int[1])[0]++;
            kindByPhase.computeIfAbsent(phase, _ -> new HashMap<>()).computeIfAbsent(kind, _ -> new int[1])[0]++;
        }
        System.out.printf("%s, window %d..%d ms: %d Java samples, %d native samples%n", thread, from, to, java, nat);
        System.out.println("by kind:");
        print(byKind, java + nat);
        System.out.println("by phase (kinds):");
        int all = java + nat;
        byPhase.entrySet().stream().sorted((a, b) -> b.getValue()[0] - a.getValue()[0]).forEach(en -> {
            StringBuilder kinds = new StringBuilder();
            kindByPhase.get(en.getKey()).entrySet().stream().sorted((a, b) -> b.getValue()[0] - a.getValue()[0])
                    .forEach(k -> kinds.append(k.getKey()).append(' ').append(k.getValue()[0]).append(", "));
            System.out.printf("  %6d %5.1f%%  %-40s %s%n", en.getValue()[0], 100.0 * en.getValue()[0] / Math.max(1, all), en.getKey(), kinds);
        });
    }

    private static void print(Map<String, int[]> map, int total) {
        map.entrySet().stream().sorted((a, b) -> b.getValue()[0] - a.getValue()[0]).forEach(en ->
                System.out.printf("  %6d %5.1f%%  %s%n", en.getValue()[0], 100.0 * en.getValue()[0] / Math.max(1, total), en.getKey()));
    }

    private static void timeline(List<RecordedEvent> events, Instant jvmStart, String thread, long from, long to, long bucket) {
        int n = (int) ((to - from + bucket - 1) / bucket);
        int[] samples = new int[n];
        double[] parked = new double[n];
        List<Map<String, int[]>> phases = new ArrayList<>();
        for (int i = 0; i < n; i++) {
            phases.add(new HashMap<>());
        }
        for (RecordedEvent e : events) {
            if (!matches(thread, threadOf(e))) {
                continue;
            }
            String name = e.getEventType().getName();
            if (name.endsWith("Sample")) {
                long time = uptime(jvmStart, e.getStartTime());
                if (time < from || time >= to) {
                    continue;
                }
                int b = (int) ((time - from) / bucket);
                samples[b]++;
                String phase = phaseOf(e.getStackTrace());
                String kind = name.equals("jdk.ExecutionSample") ? kindOf(e.getStackTrace()) : "native";
                phases.get(b).computeIfAbsent(phase + (kind.equals("code") ? "" : " [" + kind + "]"), _ -> new int[1])[0]++;
            } else {
                long s = uptime(jvmStart, e.getStartTime());
                long end = uptime(jvmStart, e.getEndTime());
                for (int b = 0; b < n; b++) {
                    long bs = from + b * bucket, be = bs + bucket;
                    parked[b] += Math.max(0, Math.min(end, be) - Math.max(s, bs));
                }
            }
        }
        for (int b = 0; b < n; b++) {
            StringBuilder top = new StringBuilder();
            phases.get(b).entrySet().stream().sorted((x, y) -> y.getValue()[0] - x.getValue()[0]).limit(4)
                    .forEach(en -> top.append(en.getKey()).append(' ').append(en.getValue()[0]).append("; "));
            System.out.printf("%6d %4d samples %5.0f ms blocked  %s%n", from + b * bucket, samples[b], parked[b], top);
        }
    }

    private static void parks(List<RecordedEvent> events, Instant jvmStart, String thread, long from, long to) {
        Map<String, double[]> sites = new LinkedHashMap<>();
        for (RecordedEvent e : events) {
            String name = e.getEventType().getName();
            if (name.endsWith("Sample") || !matches(thread, threadOf(e))) {
                continue;
            }
            double o = overlap(jvmStart, e, from, to);
            if (o <= 0) {
                continue;
            }
            String site = "?";
            RecordedStackTrace stack = e.getStackTrace();
            if (stack != null) {
                StringBuilder sb = new StringBuilder();
                int k = 0;
                for (RecordedFrame f : stack.getFrames()) {
                    String fn = frameName(f);
                    if (fn.startsWith("java.") || fn.startsWith("jdk.") || fn.startsWith("sun.")) {
                        continue;
                    }
                    sb.append(k == 0 ? "" : " < ").append(fn.substring(fn.lastIndexOf('.', fn.lastIndexOf('.') - 1) + 1));
                    if (++k == 3) {
                        break;
                    }
                }
                site = sb.toString();
            }
            double[] row = sites.computeIfAbsent(name.substring(4) + " " + site, _ -> new double[2]);
            row[0] += o;
            row[1]++;
        }
        sites.entrySet().stream().sorted((a, b) -> Double.compare(b.getValue()[0], a.getValue()[0])).limit(25)
                .forEach(en -> System.out.printf("  %8.0f ms %5.0fx  %s%n", en.getValue()[0], en.getValue()[1], en.getKey()));
    }

    /** Callee tree below the outermost frame starting with {@code under} ("" for the thread's root), N levels deep */
    private static void tree(List<RecordedEvent> events, Instant jvmStart, String thread, long from, long to, String under, int depth, double minPercent) {
        Map<String, Object[]> root = new TreeMap<>();
        int[] total = {0};
        int all = 0;
        for (RecordedEvent e : events) {
            String name = e.getEventType().getName();
            if (!name.endsWith("Sample") || !matches(thread, threadOf(e))) {
                continue;
            }
            long time = uptime(jvmStart, e.getStartTime());
            if (time < from || time >= to || e.getStackTrace() == null) {
                continue;
            }
            all++;
            List<RecordedFrame> frames = e.getStackTrace().getFrames();
            int start = under.isEmpty() ? frames.size() : -1;
            if (!under.isEmpty()) {
                for (int i = frames.size() - 1; i >= 0; i--) {
                    if (frameName(frames.get(i)).startsWith(under)) {
                        start = i;
                        break;
                    }
                }
                if (start < 0) {
                    continue;
                }
            }
            total[0]++;
            Map<String, Object[]> level = root;
            for (int i = start - 1, d = 0; i >= 0 && d < depth; i--, d++) {
                String fn = frameName(frames.get(i));
                Object[] node = level.computeIfAbsent(fn, _ -> new Object[]{new int[1], new TreeMap<String, Object[]>()});
                ((int[]) node[0])[0]++;
                @SuppressWarnings("unchecked") Map<String, Object[]> next = (Map<String, Object[]>) node[1];
                level = next;
            }
        }
        System.out.printf("%s %d..%d: %d samples, %d under %s%n", thread, from, to, all, total[0], under);
        printTree(root, total[0], "  ", minPercent);
    }

    @SuppressWarnings("unchecked")
    private static void printTree(Map<String, Object[]> level, int total, String indent, double minPercent) {
        level.entrySet().stream().sorted((a, b) -> ((int[]) b.getValue()[0])[0] - ((int[]) a.getValue()[0])[0]).forEach(en -> {
            int n = ((int[]) en.getValue()[0])[0];
            if (100.0 * n / Math.max(1, total) < minPercent) {
                return;
            }
            System.out.printf("%s%5d %5.1f%%  %s%n", indent, n, 100.0 * n / Math.max(1, total), en.getKey());
            printTree((Map<String, Object[]>) en.getValue()[1], total, indent + "  ", minPercent);
        });
    }

    private static void top(List<RecordedEvent> events, Instant jvmStart, String thread, long from, long to, int limit) {
        Map<String, int[]> inclusive = new HashMap<>();
        Map<String, int[]> self = new HashMap<>();
        int total = 0;
        for (RecordedEvent e : events) {
            String name = e.getEventType().getName();
            if (!name.endsWith("Sample") || !matches(thread, threadOf(e))) {
                continue;
            }
            long time = uptime(jvmStart, e.getStartTime());
            if (time < from || time >= to || e.getStackTrace() == null) {
                continue;
            }
            total++;
            java.util.Set<String> seen = new java.util.HashSet<>();
            List<RecordedFrame> frames = e.getStackTrace().getFrames();
            if (!frames.isEmpty()) {
                self.computeIfAbsent(frameName(frames.get(0)), _ -> new int[1])[0]++;
            }
            for (RecordedFrame f : frames) {
                String fn = frameName(f);
                if (seen.add(fn)) {
                    inclusive.computeIfAbsent(fn, _ -> new int[1])[0]++;
                }
            }
        }
        System.out.printf("%s %d..%d: %d samples%n", thread, from, to, total);
        int t = total;
        System.out.println("inclusive:");
        inclusive.entrySet().stream().sorted((a, b) -> b.getValue()[0] - a.getValue()[0]).limit(limit)
                .forEach(en -> System.out.printf("  %6d %5.1f%%  %s%n", en.getValue()[0], 100.0 * en.getValue()[0] / t, en.getKey()));
        System.out.println("self:");
        self.entrySet().stream().sorted((a, b) -> b.getValue()[0] - a.getValue()[0]).limit(20)
                .forEach(en -> System.out.printf("  %6d %5.1f%%  %s%n", en.getValue()[0], 100.0 * en.getValue()[0] / t, en.getKey()));
    }
}
