import java.io.IOException;
import java.io.RandomAccessFile;
import java.nio.ByteBuffer;
import java.nio.MappedByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Shortest strong-reference paths from GC roots to instances of a class in an HPROF heap dump (e.g. the footprint
 * scene's {@code -Pbench.footprintHeapDump=left}). Reference.referent fields are not followed, so weak and soft
 * references don't count as retaining.
 * <pre>
 * java -Xmx3g scripts/HprofPath.java dump.hprof net.minecraft.client.multiplayer.ClientLevel [paths|holders] [skip,fields]
 * </pre>
 * Prints up to {@code paths} (3) paths, one per instance found first by a breadth-first walk from the roots. With
 * {@code holders} it instead groups every reachable instance by the last 4 steps of its path (class.field) and prints
 * the 20 most common, i.e. what keeps most of them alive. Fields named in {@code skip} are not followed (to find the
 * next path once one is known).
 */
public final class HprofPath {
    private static final long WINDOW = 1L << 30;
    /** -Dreferent=true also follows Reference.referent (to classify softly or weakly reachable objects) */
    private static final boolean FOLLOW_REFERENT = Boolean.getBoolean("referent");

    private final MappedByteBuffer[] windows;
    private final long fileSize;
    private int idSize;

    private final Map<Long, String> strings = new HashMap<>();
    private final Map<Long, Long> classNames = new HashMap<>();
    private final Map<Long, ClassInfo> classes = new HashMap<>();
    private final List<long[]> roots = new ArrayList<>();
    private final java.util.Set<String> skipFields = new java.util.HashSet<>();
    private final LongIntMap index = new LongIntMap(1 << 24);
    private long[] ids = new long[1 << 22];
    private long[] offsets = new long[1 << 22];
    private byte[] kinds = new byte[1 << 22];
    private int count;

    private static final byte INSTANCE = 1, OBJ_ARRAY = 2, PRIM_ARRAY = 3, CLASS = 4;

    record Field(long name, int type) {
    }

    static final class ClassInfo {
        long id, superId, loaderId;
        Field[] fields;
        long[] staticRefs;
        long[] staticNames;
    }

    private HprofPath(String path) throws IOException {
        try (RandomAccessFile file = new RandomAccessFile(path, "r"); FileChannel channel = file.getChannel()) {
            fileSize = channel.size();
            windows = new MappedByteBuffer[(int) ((fileSize + WINDOW - 1) / WINDOW)];
            for (int i = 0; i < windows.length; i++) {
                long start = i * WINDOW;
                // Overlap the next window by 64 KB so small reads never straddle two mappings
                windows[i] = channel.map(FileChannel.MapMode.READ_ONLY, start, Math.min(WINDOW + 65536, fileSize - start));
            }
        }
    }

    private int u1(long pos) {
        return windows[(int) (pos / WINDOW)].get((int) (pos % WINDOW)) & 0xFF;
    }

    private int u2(long pos) {
        return (u1(pos) << 8) | u1(pos + 1);
    }

    private int u4(long pos) {
        ByteBuffer w = windows[(int) (pos / WINDOW)];
        int off = (int) (pos % WINDOW);
        return off + 4 <= w.limit() ? w.getInt(off) : (u2(pos) << 16) | u2(pos + 2);
    }

    private long u8(long pos) {
        return ((long) u4(pos) << 32) | (u4(pos + 4) & 0xFFFFFFFFL);
    }

    private long id(long pos) {
        return idSize == 8 ? u8(pos) : u4(pos) & 0xFFFFFFFFL;
    }

    private int typeSize(int type) {
        return switch (type) {
            case 2 -> idSize;
            case 4, 8 -> 1;
            case 5, 9 -> 2;
            case 6, 10 -> 4;
            case 7, 11 -> 8;
            default -> throw new IllegalStateException("type " + type);
        };
    }

    private void add(long id, long offset, byte kind) {
        if (count == ids.length) {
            ids = java.util.Arrays.copyOf(ids, count * 2);
            offsets = java.util.Arrays.copyOf(offsets, count * 2);
            kinds = java.util.Arrays.copyOf(kinds, count * 2);
        }
        ids[count] = id;
        offsets[count] = offset;
        kinds[count] = kind;
        index.put(id, count++);
    }

    private void parse() {
        long pos = 0;
        while (u1(pos) != 0) {
            pos++;
        }
        pos++;
        idSize = u4(pos);
        pos += 12;
        while (pos < fileSize) {
            int tag = u1(pos);
            long length = u4(pos + 5) & 0xFFFFFFFFL;
            long body = pos + 9;
            switch (tag) {
                case 0x01 -> {
                    byte[] bytes = new byte[(int) (length - idSize)];
                    for (int i = 0; i < bytes.length; i++) {
                        bytes[i] = (byte) u1(body + idSize + i);
                    }
                    strings.put(id(body), new String(bytes, StandardCharsets.UTF_8));
                }
                case 0x02 -> classNames.put(id(body + 4), id(body + 8 + idSize));
                case 0x0C, 0x1C -> parseHeap(body, body + length);
                default -> {
                }
            }
            pos = body + length;
        }
    }

    private void parseHeap(long pos, long end) {
        while (pos < end) {
            int tag = u1(pos++);
            switch (tag) {
                case 0xFF, 0x05, 0x07 -> { roots.add(new long[]{id(pos), tag}); pos += idSize; }
                case 0x01 -> { roots.add(new long[]{id(pos), tag}); pos += 2L * idSize; }
                case 0x02, 0x03, 0x08 -> { roots.add(new long[]{id(pos), tag}); pos += idSize + 8; }
                case 0x04, 0x06 -> { roots.add(new long[]{id(pos), tag}); pos += idSize + 4; }
                case 0x20 -> pos = parseClass(pos);
                case 0x21 -> {
                    add(id(pos), pos, INSTANCE);
                    pos += idSize + 4 + idSize;
                    pos += 4 + (u4(pos) & 0xFFFFFFFFL);
                }
                case 0x22 -> {
                    add(id(pos), pos, OBJ_ARRAY);
                    long n = u4(pos + idSize + 4) & 0xFFFFFFFFL;
                    pos += idSize + 8 + idSize + n * idSize;
                }
                case 0x23 -> {
                    add(id(pos), pos, PRIM_ARRAY);
                    long n = u4(pos + idSize + 4) & 0xFFFFFFFFL;
                    pos += idSize + 9 + n * typeSize(u1(pos + idSize + 8));
                }
                default -> throw new IllegalStateException("heap tag 0x" + Integer.toHexString(tag) + " at " + pos);
            }
        }
    }

    private long parseClass(long pos) {
        ClassInfo info = new ClassInfo();
        info.id = id(pos);
        add(info.id, pos, CLASS);
        info.superId = id(pos + idSize + 4);
        info.loaderId = id(pos + 2L * idSize + 4);
        pos += 7L * idSize + 4 + 4;
        int constants = u2(pos);
        pos += 2;
        for (int i = 0; i < constants; i++) {
            pos += 2;
            pos += 1 + typeSize(u1(pos));
        }
        int statics = u2(pos);
        pos += 2;
        List<Long> refs = new ArrayList<>(), names = new ArrayList<>();
        for (int i = 0; i < statics; i++) {
            long name = id(pos);
            int type = u1(pos + idSize);
            if (type == 2) {
                refs.add(id(pos + idSize + 1));
                names.add(name);
            }
            pos += idSize + 1 + typeSize(type);
        }
        info.staticRefs = refs.stream().mapToLong(Long::longValue).toArray();
        info.staticNames = names.stream().mapToLong(Long::longValue).toArray();
        int fields = u2(pos);
        pos += 2;
        info.fields = new Field[fields];
        for (int i = 0; i < fields; i++) {
            info.fields[i] = new Field(id(pos), u1(pos + idSize));
            pos += idSize + 1;
        }
        classes.put(info.id, info);
        return pos;
    }

    private String className(long classId) {
        Long name = classNames.get(classId);
        return name == null ? "?" : strings.getOrDefault(name, "?").replace('/', '.');
    }

    private boolean isReference(long classId) {
        for (ClassInfo c = classes.get(classId); c != null; c = classes.get(c.superId)) {
            if (className(c.id).equals("java.lang.ref.Reference")) {
                return true;
            }
        }
        return false;
    }

    /** Calls the visitor for each outgoing reference of object i: (target id, edge label) */
    private void forEachRef(int i, java.util.function.BiConsumer<Long, String> visitor) {
        long pos = offsets[i];
        switch (kinds[i]) {
            case INSTANCE -> {
                long classId = id(pos + idSize + 4);
                visitor.accept(classId, "<class>");
                boolean reference = isReference(classId);
                long data = pos + 2L * idSize + 8;
                for (ClassInfo c = classes.get(classId); c != null; c = classes.get(c.superId)) {
                    for (Field f : c.fields) {
                        if (f.type == 2) {
                            String name = strings.getOrDefault(f.name, "?");
                            long target = id(data);
                            if (target != 0 && !(reference && name.equals("referent") && !FOLLOW_REFERENT) && !skipFields.contains(name)) {
                                visitor.accept(target, name);
                            }
                        }
                        data += typeSize(f.type);
                    }
                }
            }
            case OBJ_ARRAY -> {
                long n = u4(pos + idSize + 4) & 0xFFFFFFFFL;
                long data = pos + 2L * idSize + 8;
                for (long k = 0; k < n; k++) {
                    long target = id(data + k * idSize);
                    if (target != 0) {
                        visitor.accept(target, "[" + k + "]");
                    }
                }
            }
            case CLASS -> {
                ClassInfo c = classes.get(ids[i]);
                if (c.superId != 0) visitor.accept(c.superId, "<super>");
                if (c.loaderId != 0) visitor.accept(c.loaderId, "<loader>");
                for (int k = 0; k < c.staticRefs.length; k++) {
                    if (c.staticRefs[k] != 0) visitor.accept(c.staticRefs[k], "static " + strings.getOrDefault(c.staticNames[k], "?"));
                }
            }
            default -> {
            }
        }
    }

    /** Class name of an instance or object array, or the JVM descriptor ("[B", "[I", ...) of a primitive array */
    private String typeName(int i) {
        long pos = offsets[i];
        return switch (kinds[i]) {
            case INSTANCE -> className(id(pos + idSize + 4));
            case OBJ_ARRAY -> className(id(pos + idSize + 8));
            case PRIM_ARRAY -> "[" + "????ZCFDBSIJ".charAt(u1(pos + idSize + 8));
            default -> "class";
        };
    }

    private String describe(int i) {
        long pos = offsets[i];
        return switch (kinds[i]) {
            case INSTANCE -> className(id(pos + idSize + 4));
            case OBJ_ARRAY -> className(id(pos + idSize + 8)) + " (" + (u4(pos + idSize + 4) & 0xFFFFFFFFL) + ")";
            case CLASS -> "class " + className(ids[i]);
            default -> "primitive array " + typeName(i);
        };
    }

    /** Owner of an edge for grouping: "Class.field" (arrays as their type) */
    private String step(int[] parent, String[] edge, int j) {
        String owner = parent[j] >= 0 ? describe(parent[j]).replaceAll(" \\(\\d+\\)", "") : "";
        String label = edge[j].startsWith("[") ? "[]" : edge[j];
        return owner.substring(owner.lastIndexOf('.') + 1) + "." + label;
    }

    private void holders(String target) {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        Map<String, Integer> groups = new HashMap<>();
        int total = 0;
        for (int i = 0; i < count; i++) {
            if (parent[i] != -2 && typeName(i).equals(target)) {
                StringBuilder key = new StringBuilder();
                int j = i;
                for (int k = 0; k < 4 && j >= 0 && parent[j] >= 0; k++, j = parent[j]) {
                    key.insert(0, " > " + step(parent, edge, j));
                }
                groups.merge(key.toString(), 1, Integer::sum);
                total++;
            }
        }
        System.out.println(total + " reachable " + target + ", by the last steps of their shortest path:");
        groups.entrySet().stream().sorted(Map.Entry.<String, Integer>comparingByValue().reversed()).limit(20)
                .forEach(e -> System.out.printf("%9d %s%n", e.getValue(), e.getKey()));
    }

    /** Breadth-first walk from the roots filling parent/edge (-2: unreached); stops after maxFound targets if given */
    private void walk(int[] parent, String[] edge, int maxFound, String target) {
        java.util.Arrays.fill(parent, -2);
        order = new int[count];
        orderCount = 0;
        ArrayDeque<Integer> queue = new ArrayDeque<>();
        for (long[] root : roots) {
            int i = index.get(root[0]);
            if (i >= 0 && parent[i] == -2) {
                parent[i] = -1;
                edge[i] = "root 0x" + Long.toHexString(root[1]);
                queue.add(i);
            }
        }
        while (!queue.isEmpty()) {
            int i = queue.poll();
            order[orderCount++] = i;
            forEachRef(i, (targetId, label) -> {
                int j = index.get(targetId);
                if (j >= 0 && parent[j] == -2) {
                    parent[j] = i;
                    edge[j] = label;
                    queue.add(j);
                }
            });
        }
    }

    private void paths(String target, int maxPaths) {
        int[] parent = new int[count];
        String[] edge = new String[count];
        java.util.Arrays.fill(parent, -2);
        ArrayDeque<Integer> queue = new ArrayDeque<>();
        for (long[] root : roots) {
            int i = index.get(root[0]);
            if (i >= 0 && parent[i] == -2) {
                parent[i] = -1;
                edge[i] = "root 0x" + Long.toHexString(root[1]);
                queue.add(i);
            }
        }
        int found = 0;
        while (!queue.isEmpty() && found < maxPaths) {
            int i = queue.poll();
            if (kinds[i] == INSTANCE && className(id(offsets[i] + idSize + 4)).equals(target)) {
                found++;
                System.out.println("== Path " + found + " to " + target + " @" + Long.toHexString(ids[i]));
                List<String> lines = new ArrayList<>();
                for (int j = i; j >= 0; j = parent[j]) {
                    lines.add(edge[j] + " -> " + describe(j));
                }
                for (int k = lines.size() - 1; k >= 0; k--) {
                    System.out.println("  " + lines.get(k));
                }
                continue;
            }
            forEachRef(i, (targetId, label) -> {
                int j = index.get(targetId);
                if (j >= 0 && parent[j] == -2) {
                    parent[j] = i;
                    edge[j] = label;
                    queue.add(j);
                }
            });
        }
        if (found == 0) {
            System.out.println("No strongly reachable " + target);
        }
    }

    // ---------------------------------------------------------------------------------------------------------------
    // Size analyses (uncommitted investigation additions): shallow sizes grouped by holder and by client/server side.
    //   sizes <type> [depth]   all reachable instances of a type, bytes grouped by side and the last `depth` steps that
    //                          are not JDK/library classes (the array's own holder step is always kept)
    //   light                  DataLayer byte[]s: uniform (one nibble value) vs mixed, per side and holder
    //   palettes               PalettedContainer$Data: bits, palette size, distinct values actually used, repack sizes
    //   dups <type> [depth]    bytes in reachable primitive arrays whose content repeats, per side and holder
    // ---------------------------------------------------------------------------------------------------------------

    private int[] order;
    private int orderCount;
    private final Map<Long, Byte> sideByClass = new HashMap<>();
    private static final String[] SIDES = {"other", "client", "server"};

    private long classIdOf(int i) {
        return switch (kinds[i]) {
            case INSTANCE -> id(offsets[i] + idSize + 4);
            case OBJ_ARRAY -> id(offsets[i] + idSize + 8);
            case CLASS -> ids[i];
            default -> 0;
        };
    }

    private byte ownSide(int i) {
        long classId = classIdOf(i);
        if (classId == 0) return 0;
        return sideByClass.computeIfAbsent(classId, c -> {
            String n = className(c);
            if (n.startsWith("net.minecraft.server.") || n.equals("net.minecraft.client.server.IntegratedServer")) return (byte) 2;
            if (n.startsWith("net.minecraft.client.") || n.startsWith("net.ruthenium.") || n.startsWith("com.mojang.blaze3d.")) return (byte) 1;
            return (byte) 0;
        });
    }

    /** Side of every reached object: server if any ancestor on its shortest path is server code, else client if any is client code */
    private byte[] sides(int[] parent) {
        byte[] side = new byte[count];
        for (int k = 0; k < orderCount; k++) {
            int i = order[k];
            byte s = parent[i] >= 0 ? side[parent[i]] : 0;
            side[i] = (byte) Math.max(s, ownSide(i));
        }
        return side;
    }

    private static boolean isLibrary(String name) {
        return name.startsWith("java.") || name.startsWith("jdk.") || name.startsWith("sun.") || name.startsWith("it.unimi.")
                || name.startsWith("com.google.") || name.startsWith("[") || name.startsWith("class java.");
    }

    /** side | the last `depth` non-library steps of i's path (its own step always) */
    private String key(int[] parent, String[] edge, byte[] side, int i, int depth) {
        StringBuilder key = new StringBuilder();
        int j = i;
        int kept = 0;
        boolean first = true;
        while (j >= 0 && parent[j] >= 0 && kept < depth) {
            String owner = describe(parent[j]);
            if (first || !isLibrary(owner)) {
                key.insert(0, " > " + step(parent, edge, j));
                kept++;
            }
            first = false;
            j = parent[j];
        }
        return SIDES[side[i]] + " |" + key;
    }

    private long shallowSize(int i) {
        long pos = offsets[i];
        long bytes = switch (kinds[i]) {
            case PRIM_ARRAY -> 16 + (u4(pos + idSize + 4) & 0xFFFFFFFFL) * typeSize(u1(pos + idSize + 8));
            case OBJ_ARRAY -> 16 + (u4(pos + idSize + 4) & 0xFFFFFFFFL) * 4;
            case INSTANCE -> {
                long s = 12;
                for (ClassInfo c = classes.get(id(pos + idSize + 4)); c != null; c = classes.get(c.superId)) {
                    for (Field f : c.fields) s += f.type == 2 ? 4 : typeSize(f.type);
                }
                yield s;
            }
            default -> 0;
        };
        return (bytes + 7) & ~7L;
    }

    private int arrayLength(int i) {
        return u4(offsets[i] + idSize + 4);
    }

    private long arrayData(int i) {
        return offsets[i] + idSize + 9;
    }

    /** Raw value of a named instance field (object id for references), searching the class chain; Long.MIN_VALUE if absent */
    private long field(int i, String name) {
        long pos = offsets[i];
        long data = pos + 2L * idSize + 8;
        for (ClassInfo c = classes.get(id(pos + idSize + 4)); c != null; c = classes.get(c.superId)) {
            for (Field f : c.fields) {
                if (strings.getOrDefault(f.name, "?").equals(name)) {
                    return switch (f.type) {
                        case 2 -> id(data);
                        case 4, 8 -> u1(data);
                        case 5, 9 -> u2(data);
                        case 6, 10 -> u4(data);
                        case 7, 11 -> u8(data);
                        default -> throw new IllegalStateException();
                    };
                }
                data += typeSize(f.type);
            }
        }
        return Long.MIN_VALUE;
    }

    private int indexOf(long id) {
        return id == 0 || id == Long.MIN_VALUE ? -1 : index.get(id);
    }

    private static final class Group {
        long count, bytes, extra1, extra2;
    }

    private static void printGroups(Map<String, Group> groups, int limit, String extraHeader) {
        groups.entrySet().stream().sorted((a, b) -> Long.compare(b.getValue().bytes, a.getValue().bytes)).limit(limit)
                .forEach(e -> System.out.printf("%9d %9.2f MB%s %s%n", e.getValue().count, e.getValue().bytes / 1048576.0,
                        extraHeader == null ? "" : String.format(" %9.2f %9.2f", e.getValue().extra1 / 1048576.0, e.getValue().extra2 / 1048576.0),
                        e.getKey()));
    }

    private void sizes(String target, int depth) {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        byte[] side = sides(parent);
        Map<String, Group> groups = new HashMap<>();
        long[] bySide = new long[3];
        long[] countBySide = new long[3];
        long total = 0, unreachable = 0;
        for (int i = 0; i < count; i++) {
            if (kinds[i] == CLASS || !typeName(i).equals(target)) continue;
            long size = shallowSize(i);
            if (parent[i] == -2) {
                unreachable += size;
                continue;
            }
            total += size;
            bySide[side[i]] += size;
            countBySide[side[i]]++;
            Group g = groups.computeIfAbsent(key(parent, edge, side, i, depth), k -> new Group());
            g.count++;
            g.bytes += size;
        }
        System.out.printf("%s: %.2f MB reachable (%.2f MB unreachable)%n", target, total / 1048576.0, unreachable / 1048576.0);
        for (int s = 0; s < 3; s++) System.out.printf("  %-6s %9d %9.2f MB%n", SIDES[s], countBySide[s], bySide[s] / 1048576.0);
        printGroups(groups, 45, null);
    }

    private void light() {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        byte[] side = sides(parent);
        // key -> count, bytes, uniform bytes (extra1), uniform 15 bytes (extra2)
        Map<String, Group> groups = new HashMap<>();
        long layers = 0, nullData = 0;
        for (int i = 0; i < count; i++) {
            if (parent[i] == -2 || kinds[i] != INSTANCE || !typeName(i).equals("net.minecraft.world.level.chunk.DataLayer")) continue;
            layers++;
            int d = indexOf(field(i, "data"));
            String k = key(parent, edge, side, i, 3);
            Group g = groups.computeIfAbsent(k, x -> new Group());
            if (d < 0) {
                nullData++;
                g.count++;
                continue;
            }
            g.count++;
            long size = shallowSize(d);
            g.bytes += size;
            int n = arrayLength(d);
            long p = arrayData(d);
            int first = u1(p);
            boolean uniform = true;
            for (int b = 1; b < n && uniform; b++) uniform = u1(p + b) == first;
            if (uniform) {
                g.extra1 += size;
                if (first == 0xFF) g.extra2 += size;
            }
        }
        System.out.printf("DataLayer: %d reachable, %d with null data%n", layers, nullData);
        System.out.println("    count   data MB  uniform  of it 15  holder");
        printGroups(groups, 30, "");
    }

    /** The most repeated DataLayer contents per side: copies, nibble histogram (values 0-15) */
    private void lightDups() {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        byte[] side = sides(parent);
        Map<String, int[]> copies = new HashMap<>();
        Map<String, int[]> histos = new HashMap<>();
        for (int i = 0; i < count; i++) {
            if (parent[i] == -2 || kinds[i] != INSTANCE || !typeName(i).equals("net.minecraft.world.level.chunk.DataLayer")) continue;
            int d = indexOf(field(i, "data"));
            if (d < 0) continue;
            String k = key(parent, edge, side, i, 3);
            String kind = SIDES[side[i]] + (k.contains("Sky") ? " sky" : " block");
            long p = arrayData(d);
            int n = arrayLength(d);
            byte[] b = new byte[n];
            for (int x = 0; x < n; x++) b[x] = (byte) u1(p + x);
            String h = kind + " " + java.util.Arrays.hashCode(b) + ":" + java.util.HexFormat.of().formatHex(b, 0, 64);
            copies.computeIfAbsent(h, x -> new int[1])[0]++;
            histos.computeIfAbsent(h, x -> {
                int[] hist = new int[16];
                for (byte v : b) { hist[v & 15]++; hist[(v >> 4) & 15]++; }
                return hist;
            });
        }
        copies.entrySet().stream().filter(e -> e.getValue()[0] > 1).sorted((a, c) -> c.getValue()[0] - a.getValue()[0]).limit(25).forEach(e -> {
            int[] hist = histos.get(e.getKey());
            StringBuilder sb = new StringBuilder();
            for (int v = 0; v < 16; v++) if (hist[v] > 0) sb.append(v).append(':').append(hist[v]).append(' ');
            System.out.printf("%6d copies  %s  nibbles %s%n", e.getValue()[0], e.getKey().substring(0, e.getKey().indexOf(' ', e.getKey().indexOf(' ') + 1)), sb);
        });
    }

    /** Bytes under each reachable ASM ClassNode (objects whose nearest ClassNode ancestor it is), grouped by name prefix */
    private void classNodes(int prefixParts) {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        int[] owner = new int[count];
        java.util.Arrays.fill(owner, -1);
        Map<String, Group> groups = new HashMap<>();
        for (int k = 0; k < orderCount; k++) {
            int i = order[k];
            if (kinds[i] == INSTANCE && typeName(i).equals("org.objectweb.asm.tree.ClassNode")) owner[i] = i;
            else if (parent[i] >= 0) owner[i] = owner[parent[i]];
            if (owner[i] < 0 || kinds[i] == CLASS) continue;
            String name = stringValue(field(owner[i], "name"));
            String[] parts = name == null ? new String[]{"?"} : name.split("/");
            String key = String.join("/", java.util.Arrays.copyOf(parts, Math.min(prefixParts, parts.length))) + "  via " + step(parent, edge, owner[i]);
            Group g = groups.computeIfAbsent(key, x -> new Group());
            if (owner[i] == i) g.count++;
            g.bytes += shallowSize(i);
        }
        System.out.println("ClassNodes, bytes under them (count = nodes)");
        printGroups(groups, 25, null);
    }

    /**
     * Live bytes by side, and the estimated saving of -XX:+UseCompactObjectHeaders (8 byte headers: instances and
     * arrays of 4-byte-or-smaller elements shrink by 4 bytes before 8 byte alignment; long[]/double[] keep a 16 byte base)
     */
    private void overview() {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        byte[] side = sides(parent);
        long[] bySide = new long[3];
        long total = 0, compact = 0;
        Map<String, Group> saving = new HashMap<>();
        for (int i = 0; i < count; i++) {
            if (parent[i] == -2 || kinds[i] == CLASS) continue;
            long size = shallowSize(i);
            total += size;
            bySide[side[i]] += size;
            long c;
            if (kinds[i] == PRIM_ARRAY) {
                int es = typeSize(u1(offsets[i] + idSize + 8));
                long n = arrayLength(i);
                c = es == 8 ? size : ((12 + n * es + 7) & ~7L);
            } else if (kinds[i] == OBJ_ARRAY) {
                c = (12 + arrayLength(i) * 4L + 7) & ~7L;
            } else {
                c = Math.max(8, (size - 4 + 7) & ~7L);
                // shallowSize rounded 12 + fields up; recompute from fields for the 8 byte header
                long fields = 0;
                for (ClassInfo ci = classes.get(id(offsets[i] + idSize + 4)); ci != null; ci = classes.get(ci.superId)) {
                    for (Field f : ci.fields) fields += f.type == 2 ? 4 : typeSize(f.type);
                }
                c = (8 + fields + 7) & ~7L;
                if (c < 16 && fields > 0) c = Math.max(c, 16); // keep it conservative for tiny objects
            }
            compact += c;
            Group g = saving.computeIfAbsent(typeName(i), x -> new Group());
            g.count++;
            g.bytes += size - c;
        }
        System.out.printf("reachable %.2f MB: other %.2f, client %.2f, server %.2f%n", total / 1048576.0, bySide[0] / 1048576.0,
                bySide[1] / 1048576.0, bySide[2] / 1048576.0);
        System.out.printf("with compact object headers ~%.2f MB (saves ~%.2f MB); by type:%n", compact / 1048576.0, (total - compact) / 1048576.0);
        printGroups(saving, 15, null);
    }

    /** Objects the walk does not reach, by type and (for arrays) length */
    private void unreached() {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        Map<String, Group> groups = new HashMap<>();
        for (int i = 0; i < count; i++) {
            if (parent[i] != -2 || kinds[i] == CLASS) continue;
            String k = typeName(i) + (kinds[i] == PRIM_ARRAY || kinds[i] == OBJ_ARRAY ? " len=" + arrayLength(i) : "");
            Group g = groups.computeIfAbsent(k, x -> new Group());
            g.count++;
            g.bytes += shallowSize(i);
        }
        printGroups(groups, 25, null);
    }

    private static int ceilLog2(int n) {
        return n <= 1 ? 0 : 32 - Integer.numberOfLeadingZeros(n - 1);
    }

    private static long storageBytes(int bits, int entries) {
        if (bits == 0) return 0;
        int perLong = 64 / bits;
        long longs = (entries + perLong - 1) / perLong;
        return 16 + 8 * longs;
    }

    private void palettes() {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        byte[] side = sides(parent);
        // key -> count, current long[] bytes, bytes after a vanilla repack with only the used values (extra1),
        // bytes with 1-3 bit linear palettes allowed for block states too (extra2)
        Map<String, Group> groups = new HashMap<>();
        Map<String, Group> bySideKind = new HashMap<>();
        java.util.BitSet used = new java.util.BitSet();
        for (int i = 0; i < count; i++) {
            if (parent[i] == -2 || kinds[i] != INSTANCE || !typeName(i).equals("net.minecraft.world.level.chunk.PalettedContainer$Data")) continue;
            int st = indexOf(field(i, "storage"));
            int pal = indexOf(field(i, "palette"));
            if (st < 0 || pal < 0) continue;
            String stName = typeName(st), palName = typeName(pal);
            int entries = (int) field(st, "size");
            boolean states = entries == 4096;
            int bits = 0, distinct = 1, paletteSize = 1;
            long cur = 0;
            if (stName.endsWith("SimpleBitStorage")) {
                bits = (int) field(st, "bits");
                int arr = indexOf(field(st, "data"));
                cur = shallowSize(arr);
                int perLong = 64 / bits;
                long mask = (1L << bits) - 1;
                long p = arrayData(arr);
                used.clear();
                int n = arrayLength(arr);
                for (int w = 0, e = 0; w < n && e < entries; w++) {
                    long word = u8(p + 8L * w);
                    for (int s = 0; s < perLong && e < entries; s++, e++) used.set((int) ((word >>> (s * bits)) & mask));
                }
                distinct = used.cardinality();
                if (palName.endsWith("LinearPalette")) paletteSize = (int) field(pal, "size");
                else if (palName.endsWith("HashMapPalette")) paletteSize = (int) field(indexOf(field(pal, "values")), "size");
                else paletteSize = -1;
            }
            int vanillaBits, idealBits = ceilLog2(distinct);
            if (states) {
                vanillaBits = distinct <= 1 ? 0 : distinct <= 16 ? 4 : idealBits <= 8 ? idealBits : 15;
                if (idealBits > 8) idealBits = 15;
                if (bits > 8) { vanillaBits = bits; idealBits = bits; } // global palette: ids, not indices
            } else {
                vanillaBits = distinct <= 1 ? 0 : idealBits <= 3 ? idealBits : 7;
                if (bits > 3) { vanillaBits = bits; idealBits = bits; }
                idealBits = Math.min(idealBits, vanillaBits);
            }
            long vanilla = storageBytes(vanillaBits, entries), ideal = storageBytes(idealBits, entries);
            String palShort = palName.substring(palName.lastIndexOf('.') + 1);
            String k = SIDES[side[i]] + " " + (states ? "states" : "biomes") + " " + palShort + " bits=" + bits
                    + (paletteSize >= 0 && bits > 0 ? " palette=" + (paletteSize <= 4 ? String.valueOf(paletteSize) : paletteSize <= 16 ? "5-16" : ">16") : "")
                    + (bits > 0 ? " used=" + (distinct <= 4 ? String.valueOf(distinct) : distinct <= 8 ? "5-8" : distinct <= 16 ? "9-16" : ">16") : "");
            Group g = groups.computeIfAbsent(k, x -> new Group());
            g.count++; g.bytes += cur; g.extra1 += vanilla; g.extra2 += ideal;
            Group h = bySideKind.computeIfAbsent(SIDES[side[i]] + " " + (states ? "states" : "biomes") + " via " + key(parent, edge, side, i, 2), x -> new Group());
            h.count++; h.bytes += cur; h.extra1 += vanilla; h.extra2 += ideal;
        }
        System.out.println("    count   long[] MB repack MB  ideal MB  kind");
        printGroups(groups, 40, "");
        System.out.println("By side and holder:");
        printGroups(bySideKind, 20, "");
    }

    private void dups(String target, int depth) {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        byte[] side = sides(parent);
        Map<String, Long> firstByContent = new HashMap<>();
        Map<String, Group> groups = new HashMap<>();
        Map<String, int[]> sidesByContent = new HashMap<>();
        long total = 0, dupBytes = 0;
        java.security.MessageDigest md;
        try {
            md = java.security.MessageDigest.getInstance("SHA-1");
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
        byte[] buf = new byte[1 << 16];
        for (int i = 0; i < count; i++) {
            if (parent[i] == -2 || kinds[i] != PRIM_ARRAY || !typeName(i).equals(target)) continue;
            long size = shallowSize(i);
            total += size;
            long p = arrayData(i);
            long n = (long) arrayLength(i) * typeSize(u1(offsets[i] + idSize + 8));
            md.reset();
            for (long off = 0; off < n; ) {
                int len = (int) Math.min(buf.length, n - off);
                for (int b = 0; b < len; b++) buf[b] = (byte) u1(p + off + b);
                md.update(buf, 0, len);
                off += len;
            }
            String h = java.util.HexFormat.of().formatHex(md.digest()) + ":" + n;
            int[] sd = sidesByContent.computeIfAbsent(h, x -> new int[3]);
            sd[side[i]]++;
            if (firstByContent.putIfAbsent(h, (long) i) != null) {
                dupBytes += size;
                Group g = groups.computeIfAbsent(key(parent, edge, side, i, depth), x -> new Group());
                g.count++;
                g.bytes += size;
            }
        }
        long crossSide = 0;
        System.out.printf("%s: %.2f MB reachable, %.2f MB in repeated contents (beyond the first copy)%n", target, total / 1048576.0, dupBytes / 1048576.0);
        printGroups(groups, 30, null);
    }

    /**
     * Shallow bytes of all objects whose shortest path passes through a step ("Owner.field", substring match, comma
     * separated alternatives; the first match nearest the root wins), by type: an approximation of retained size.
     */
    private void subtree(String patterns, int rows) {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        byte[] side = sides(parent);
        String[] pats = patterns.split(",");
        int[] mark = new int[count];
        java.util.Arrays.fill(mark, -1);
        for (int k = 0; k < orderCount; k++) {
            int i = order[k];
            if (parent[i] >= 0 && mark[parent[i]] >= 0) {
                mark[i] = mark[parent[i]];
                continue;
            }
            if (parent[i] < 0) continue;
            String s = step(parent, edge, i);
            for (int p = 0; p < pats.length; p++) {
                if (s.contains(pats[p])) {
                    mark[i] = p;
                    break;
                }
            }
        }
        for (int p = 0; p < pats.length; p++) {
            Map<String, Group> byType = new HashMap<>();
            long total = 0;
            long[] bySide = new long[3];
            for (int i = 0; i < count; i++) {
                if (mark[i] != p || kinds[i] == CLASS) continue;
                long size = shallowSize(i);
                total += size;
                bySide[side[i]] += size;
                Group g = byType.computeIfAbsent(typeName(i), x -> new Group());
                g.count++;
                g.bytes += size;
            }
            System.out.printf("== %s: %.2f MB (other %.2f, client %.2f, server %.2f)%n", pats[p], total / 1048576.0,
                    bySide[0] / 1048576.0, bySide[1] / 1048576.0, bySide[2] / 1048576.0);
            printGroups(byType, rows, null);
        }
    }

    private String stringValue(long stringId) {
        int s = indexOf(stringId);
        if (s < 0) return null;
        int v = indexOf(field(s, "value"));
        if (v < 0) return null;
        int n = arrayLength(v);
        byte[] b = new byte[n];
        for (int k = 0; k < n; k++) b[k] = (byte) u1(arrayData(v) + k);
        return field(s, "coder") == 0 ? new String(b, StandardCharsets.ISO_8859_1) : new String(b, StandardCharsets.UTF_16LE);
    }

    /** Prints a String field (or the size of a Map field) of every instance of a class */
    private void fieldValues(String target, String fieldName) {
        for (int i = 0; i < count; i++) {
            if (kinds[i] != INSTANCE || !typeName(i).equals(target)) continue;
            long v = field(i, fieldName);
            int j = indexOf(v);
            String text = j >= 0 && typeName(j).equals("java.lang.String") ? stringValue(v) : j >= 0 ? describe(j) + (field(j, "size") != Long.MIN_VALUE ? " size=" + field(j, "size") : "") : String.valueOf(v);
            System.out.println(Long.toHexString(ids[i]) + " " + fieldName + "=" + text);
        }
    }

    /** Full shortest paths of the first instances of a type whose sizes key (at depth) contains a substring */
    private void example(String target, String substring, int depth, int max) {
        int[] parent = new int[count];
        String[] edge = new String[count];
        walk(parent, edge, -1, null);
        byte[] side = sides(parent);
        int found = 0;
        for (int i = 0; i < count && found < max; i++) {
            if (parent[i] == -2 || kinds[i] == CLASS || !typeName(i).equals(target)) continue;
            if (!key(parent, edge, side, i, depth).contains(substring)) continue;
            found++;
            System.out.println("== " + describe(i) + " " + shallowSize(i) + " bytes");
            List<String> lines = new ArrayList<>();
            for (int j = i; j >= 0; j = parent[j]) lines.add(edge[j] + " -> " + describe(j));
            for (int k = lines.size() - 1; k >= 0; k--) System.out.println("  " + lines.get(k));
        }
    }

    public static void main(String[] args) throws IOException {
        HprofPath dump = new HprofPath(args[0]);
        long start = System.nanoTime();
        dump.parse();
        System.out.printf("%d objects, %d classes, %d roots, parsed in %.1f s%n", dump.count, dump.classes.size(),
                dump.roots.size(), (System.nanoTime() - start) / 1e9);
        String mode = args.length > 2 ? args[2] : "";
        if (args[1].equals("light")) {
            dump.light();
            return;
        }
        if (args[1].equals("classnodes")) {
            dump.classNodes(args.length > 2 ? Integer.parseInt(args[2]) : 3);
            return;
        }
        if (args[1].equals("overview")) {
            dump.overview();
            return;
        }
        if (args[1].equals("unreached")) {
            dump.unreached();
            return;
        }
        if (args[1].equals("lightdups")) {
            dump.lightDups();
            return;
        }
        if (args[1].equals("palettes")) {
            dump.palettes();
            return;
        }
        if (args[1].equals("fields")) {
            for (String f : args[3].split(",")) dump.fieldValues(args[2], f);
            return;
        }
        if (args[1].equals("subtree")) {
            dump.subtree(args[2], args.length > 3 ? Integer.parseInt(args[3]) : 8);
            return;
        }
        if (args.length > 3 && !mode.equals("sizes") && !mode.equals("dups")) {
            dump.skipFields.addAll(List.of(args[3].split(",")));
        }
        if (mode.equals("example")) {
            dump.example(args[1], args[3], args.length > 4 ? Integer.parseInt(args[4]) : 3, args.length > 5 ? Integer.parseInt(args[5]) : 2);
        } else if (mode.equals("holders")) {
            dump.holders(args[1]);
        } else if (mode.equals("sizes")) {
            dump.sizes(args[1], args.length > 3 ? Integer.parseInt(args[3]) : 4);
        } else if (mode.equals("dups")) {
            dump.dups(args[1], args.length > 3 ? Integer.parseInt(args[3]) : 3);
        } else {
            dump.paths(args[1], args.length > 2 ? Integer.parseInt(args[2]) : 3);
        }
    }

    /** Open addressing map from object id to index; -1 when absent */
    static final class LongIntMap {
        private long[] keys;
        private int[] values;
        private int size;

        LongIntMap(int capacity) {
            keys = new long[capacity];
            values = new int[capacity];
        }

        private static int slot(long key, int mask) {
            long h = key * 0x9E3779B97F4A7C15L;
            return (int) (h >>> 32) & mask;
        }

        void put(long key, int value) {
            if (size * 2 >= keys.length) {
                long[] oldKeys = keys;
                int[] oldValues = values;
                keys = new long[oldKeys.length * 2];
                values = new int[oldKeys.length * 2];
                size = 0;
                for (int i = 0; i < oldKeys.length; i++) {
                    if (oldKeys[i] != 0) {
                        put(oldKeys[i], oldValues[i]);
                    }
                }
            }
            int mask = keys.length - 1;
            int s = slot(key, mask);
            while (keys[s] != 0 && keys[s] != key) {
                s = (s + 1) & mask;
            }
            if (keys[s] == 0) {
                size++;
            }
            keys[s] = key;
            values[s] = value;
        }

        int get(long key) {
            int mask = keys.length - 1;
            for (int s = slot(key, mask); keys[s] != 0; s = (s + 1) & mask) {
                if (keys[s] == key) {
                    return values[s];
                }
            }
            return -1;
        }
    }
}
