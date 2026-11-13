package net.ruthenium;

import java.lang.management.GarbageCollectorMXBean;
import java.lang.management.ManagementFactory;

/**
 * Warns at startup about ZGC on a small heap. ZGC has no compressed references (objects are about 25% bigger) and
 * collects while the game runs, so it needs free heap and free cores: with little of either it stalls the threads
 * that allocate, which shows as stutter. A mod can't change the collector, so this only logs.
 */
final class HeapCheck {
	/** Below this maximum heap the warning is logged; the vanilla launcher's 2 GB default stays above it */
	private static final long SMALL_HEAP_BYTES = 1536L << 20;

	private HeapCheck() {
	}

	static void run() {
		long maxHeap = Runtime.getRuntime().maxMemory();
		if (maxHeap == Long.MAX_VALUE || maxHeap >= SMALL_HEAP_BYTES || !usesZgc()) {
			return;
		}

		Initializer.LOGGER.warn("The Java heap is small ({} MB) and the garbage collector is ZGC, which needs more free memory than "
				+ "other collectors and can cause stutter here. Consider adding the JVM arguments -XX:+UseG1GC "
				+ "-XX:+UseCompactObjectHeaders in your launcher, or giving the game more memory.", maxHeap >> 20);
	}

	private static boolean usesZgc() {
		for (GarbageCollectorMXBean collector : ManagementFactory.getGarbageCollectorMXBeans()) {
			if (collector.getName().startsWith("ZGC")) {
				return true;
			}
		}
		return false;
	}
}
