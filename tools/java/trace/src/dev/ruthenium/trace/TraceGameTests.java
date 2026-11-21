package dev.ruthenium.trace;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;
import java.util.Set;
import java.util.stream.Collectors;
import net.fabricmc.fabric.api.client.gametest.v1.FabricClientGameTest;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestServerContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.minecraft.SharedConstants;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.worldselection.WorldCreationUiState;

/**
 * Runs every scenario in one flat survival world and writes a trace per scenario to {@code
 * ruthenium.trace.out}. {@code ruthenium.trace.only} (comma-separated names) limits the run.
 */
public final class TraceGameTests implements FabricClientGameTest {
    @Override
    public void runTest(ClientGameTestContext ctx) {
        Path dir = Path.of(System.getProperty("ruthenium.trace.out", "ruthenium-traces"));
        Set<String> only = Arrays.stream(System.getProperty("ruthenium.trace.only", "").split(","))
                .filter(s -> !s.isBlank())
                .collect(Collectors.toSet());
        try (TestSingleplayerContext sp = ctx.worldBuilder()
                .adjustSettings(s -> s.setGameMode(WorldCreationUiState.SelectedGameMode.SURVIVAL))
                .create()) {
            sp.getConnection().waitForChunksRender();
            for (Scenario scenario : Scenarios.ALL) {
                if (only.isEmpty() || only.contains(scenario.name())) {
                    run(ctx, sp, scenario, dir);
                }
            }
        }
    }

    private static void releaseAll(ClientGameTestContext ctx) {
        ctx.runOnClient(mc -> {
            for (KeyMapping key : mc.options.keyMappings) {
                key.setDown(false);
            }
        });
    }

    private static void run(ClientGameTestContext ctx, TestSingleplayerContext sp, Scenario scenario, Path dir) {
        TestServerContext server = sp.getServer();
        for (String command : Scenarios.RESET) {
            server.runCommand(command);
        }
        for (String command : scenario.build()) {
            server.runCommand(command);
        }
        double[] s = scenario.start();
        String tp = "tp @a " + s[0] + " " + s[1] + " " + s[2] + " " + s[3] + " " + s[4];
        server.runCommand(tp);
        releaseAll(ctx);
        ctx.getInput().pressKey(o -> o.keyHotbarSlots[0]);
        ctx.waitTicks(20);
        sp.getConnection().waitForClientboundPackets();
        sp.getConnection().waitForServerboundPackets();
        ctx.waitTicks(5);

        ctx.runOnClient(mc -> Recorder.begin(dir.resolve(scenario.name() + ".jsonl"), header(mc, scenario, tp)));
        try {
            scenario.script().run(ctx);
        } finally {
            ctx.runOnClient(mc -> Recorder.end());
            releaseAll(ctx);
        }
        ctx.waitTicks(5);
    }

    private static JsonArray strings(List<String> values) {
        JsonArray a = new JsonArray();
        values.forEach(a::add);
        return a;
    }

    private static JsonObject header(Minecraft mc, Scenario scenario, String tp) {
        JsonObject o = new JsonObject();
        o.addProperty("format", "ruthenium-trace/1");
        o.addProperty("game", SharedConstants.getCurrentVersion().name());
        o.addProperty("protocol", SharedConstants.getProtocolVersion());
        o.addProperty("scenario", scenario.name());
        o.addProperty("description", scenario.description());
        JsonObject world = new JsonObject();
        world.addProperty("preset", "minecraft:flat");
        world.addProperty("seed", 1);
        world.addProperty("note", "Fabric client gametest defaults: flat world, no structures, time and weather frozen, no mob spawning");
        o.add("world", world);
        o.add("reset", strings(Scenarios.RESET));
        o.add("build", strings(scenario.build()));
        o.addProperty("teleport", tp);
        JsonObject options = new JsonObject();
        options.addProperty("toggleSprint", mc.options.toggleSprint().get());
        options.addProperty("toggleCrouch", mc.options.toggleCrouch().get());
        options.addProperty("autoJump", mc.options.autoJump().get());
        options.addProperty("sensitivity", mc.options.sensitivity().get());
        o.add("options", options);
        if (mc.player != null) {
            o.addProperty("entityId", mc.player.getId());
        }
        o.addProperty("harness", "Fabric client gametest: one client tick per frame, one server tick per client tick");
        return o;
    }
}
