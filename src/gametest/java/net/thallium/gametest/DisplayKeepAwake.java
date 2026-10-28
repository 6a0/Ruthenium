package net.thallium.gametest;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;

/**
 * Keeps the displays (and system) awake while the benchmark runs, so Windows' display timeout can't blank the
 * screens mid-run. Uses SetThreadExecutionState with ES_CONTINUOUS, which lasts until the calling thread exits;
 * nothing is changed in the user's power settings.
 * <p>
 * Changing the power plan instead (powercfg /setactive) crashed the game on startup on a hybrid-graphics laptop.
 */
final class DisplayKeepAwake {
    private static final Logger LOGGER = LoggerFactory.getLogger("Thallium Benchmark");

    private static final int ES_SYSTEM_REQUIRED = 0x00000001;
    private static final int ES_DISPLAY_REQUIRED = 0x00000002;
    private static final int ES_CONTINUOUS = 0x80000000;

    private DisplayKeepAwake() {
    }

    /**
     * Must be called on a thread that lives for the whole benchmark (the client thread).
     */
    static void enable() {
        if (!System.getProperty("os.name").toLowerCase().contains("windows")) {
            return;
        }

        try {
            SymbolLookup kernel32 = SymbolLookup.libraryLookup("kernel32", Arena.global());
            MethodHandle setThreadExecutionState = Linker.nativeLinker().downcallHandle(
                    kernel32.find("SetThreadExecutionState").orElseThrow(),
                    FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.JAVA_INT));

            int previous = (int) setThreadExecutionState.invokeExact(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
            if (previous == 0) {
                LOGGER.warn("SetThreadExecutionState failed; displays may turn off during the benchmark");
            } else {
                LOGGER.info("Keeping displays awake for the benchmark");
            }
        } catch (Throwable t) {
            LOGGER.warn("Could not keep the displays awake; they may turn off during the benchmark", t);
        }
    }
}
