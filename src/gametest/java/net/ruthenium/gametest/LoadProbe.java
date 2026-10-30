package net.ruthenium.gametest;

import java.lang.management.ManagementFactory;

/**
 * Load time milestones ({@code -Pbench.loadProbe=true}): each mark prints the JVM uptime in milliseconds, so startup
 * steps can be read off the log and matched to JFR samples ({@code scripts/LoadJfr.java} takes uptime windows). Uses
 * System.out: the marks in {@code Main} run before logging is set up.
 */
public final class LoadProbe {
    private LoadProbe() {
    }

    public static void mark(String what) {
        System.out.println("[LoadProbe] " + ManagementFactory.getRuntimeMXBean().getUptime() + " ms " + what
                + " (" + Thread.currentThread().getName() + ")");
    }
}
