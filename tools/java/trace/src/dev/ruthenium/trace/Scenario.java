package dev.ruthenium.trace;

import java.util.List;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestServerContext;

/**
 * A scripted scenario: the commands that build its test area, where the player starts, and the
 * inputs to play. Areas are built in a flat world from commands, so a trace's header fully
 * describes its world. Scripts may run server commands while the trace records (entities
 * summoned then arrive in the trace as packets).
 *
 * @param start x, y, z, yaw, pitch
 */
public record Scenario(String name, String description, List<String> build, double[] start, Script script) {
    @FunctionalInterface
    public interface Script {
        void run(ClientGameTestContext ctx, TestServerContext server);
    }
}
