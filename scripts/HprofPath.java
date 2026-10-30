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
                            if (target != 0 && !(reference && name.equals("referent")) && !skipFields.contains(name)) {
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

    public static void main(String[] args) throws IOException {
        HprofPath dump = new HprofPath(args[0]);
        long start = System.nanoTime();
        dump.parse();
        System.out.printf("%d objects, %d classes, %d roots, parsed in %.1f s%n", dump.count, dump.classes.size(),
                dump.roots.size(), (System.nanoTime() - start) / 1e9);
        if (args.length > 3) {
            dump.skipFields.addAll(List.of(args[3].split(",")));
        }
        if (args.length > 2 && args[2].equals("holders")) {
            dump.holders(args[1]);
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
