import jdk.jfr.consumer.RecordedEvent;
import jdk.jfr.consumer.RecordedFrame;
import jdk.jfr.consumer.RecordedStackTrace;
import jdk.jfr.consumer.RecordingFile;

import java.nio.file.Path;
import java.time.LocalTime;
import java.time.ZoneId;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * JFR execution samples of one thread, read with the JDK's consumer API (seconds instead of the minutes jfr_tree.py
 * needs for a 1 ms profile's JSON).
 * <p>
 * Usage: java scripts/JfrTree.java <file.jfr> <thread> [--from HH:MM:SS] [--to HH:MM:SS] [--under Class.method] [--depth N] [--min PERCENT]
 * <p>
 * Prints the inclusive share of each method over the thread's samples in the window. With --under, only samples
 * whose stack contains that method count, and it also prints the callees directly below its outermost occurrence
 * (inclusive and self), N levels deep (default 2), as a tree.
 */
public class JfrTree {
    public static void main(String[] args) throws Exception {
        Path file = Path.of(args[0]);
        String thread = args[1];
        LocalTime from = null, to = null;
        String under = null;
        int depth = 2;
        for (int i = 2; i < args.length; i += 2) {
            switch (args[i]) {
                case "--from" -> from = LocalTime.parse(args[i + 1]);
                case "--to" -> to = LocalTime.parse(args[i + 1]);
                case "--under" -> under = args[i + 1];
                case "--depth" -> depth = Integer.parseInt(args[i + 1]);
                case "--min" -> minShare = Double.parseDouble(args[i + 1]) / 100.0;
                default -> throw new IllegalArgumentException(args[i]);
            }
        }
        int total = 0, matched = 0;
        Map<String, Integer> inclusive = new HashMap<>();
        Map<String, Integer> self = new HashMap<>();
        Node root = new Node("<under>");
        for (RecordedEvent event : RecordingFile.readAllEvents(file)) {
            if (!event.getEventType().getName().equals("jdk.ExecutionSample")) {
                continue;
            }
            var sampled = event.getThread("sampledThread");
            if (sampled == null || !thread.equals(sampled.getJavaName())) {
                continue;
            }
            LocalTime time = LocalTime.ofInstant(event.getStartTime(), ZoneId.systemDefault());
            if (from != null && time.isBefore(from) || to != null && !time.isBefore(to)) {
                continue;
            }
            total++;
            RecordedStackTrace stack = event.getStackTrace();
            if (stack == null) {
                continue;
            }
            List<RecordedFrame> frames = stack.getFrames();
            List<String> names = new ArrayList<>(frames.size());
            for (RecordedFrame frame : frames) {
                String type = frame.getMethod().getType().getName();
                names.add(type.substring(type.lastIndexOf('.') + 1) + "." + frame.getMethod().getName());
            }
            int at = -1;
            if (under != null) {
                for (int i = names.size() - 1; i >= 0; i--) {
                    if (names.get(i).equals(under)) {
                        at = i;
                        break;
                    }
                }
                if (at < 0) {
                    continue;
                }
            }
            matched++;
            Set<String> seen = new HashSet<>();
            for (String name : names) {
                if (seen.add(name)) {
                    inclusive.merge(name, 1, Integer::sum);
                }
            }
            if (!names.isEmpty()) {
                self.merge(names.getFirst(), 1, Integer::sum);
            }
            if (at >= 0) {
                Node node = root;
                node.count++;
                for (int i = at - 1, d = 0; d < depth; i--, d++) {
                    if (i < 0) {
                        node.self++;
                        break;
                    }
                    node = node.children.computeIfAbsent(names.get(i), Node::new);
                    node.count++;
                    if (i == 0) {
                        node.self++;
                        break;
                    }
                }
            }
        }
        System.out.printf("%s: %d samples in window, %d matched%n", thread, total, matched);
        print("inclusive", inclusive, total, 60);
        print("self (top frame)", self, total, 30);
        if (under != null) {
            System.out.println("tree under " + under + " (% of window, self = sample's top frame)");
            root.print("", total);
        }
    }

    static void print(String title, Map<String, Integer> counts, int total, int limit) {
        System.out.println(title + ":");
        counts.entrySet().stream().sorted((a, b) -> b.getValue() - a.getValue()).limit(limit)
                .forEach(e -> System.out.printf("%6.2f%%  %s%n", 100.0 * e.getValue() / total, e.getKey()));
    }

    /** Smallest share of the window a tree node needs to be printed (--min PERCENT, default 0.2) */
    static double minShare = 0.002;

    static final class Node {
        final String name;
        int count, self;
        final Map<String, Node> children = new HashMap<>();

        Node(String name) {
            this.name = name;
        }

        void print(String indent, int total) {
            System.out.printf("%s%6.2f%% (self %5.2f%%) %s%n", indent, 100.0 * count / total, 100.0 * self / total, name);
            children.values().stream().sorted((a, b) -> b.count - a.count).filter(c -> c.count >= minShare * total)
                    .forEach(c -> c.print(indent + "  ", total));
        }
    }
}
