package dev.ruthenium.trace;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import dev.ruthenium.trace.mixin.KeyMappingAccessor;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.network.Connection;
import net.minecraft.network.PacketListener;
import net.minecraft.network.ProtocolInfo;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.BundlePacket;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.phys.Vec3;

/**
 * Writes a golden trace: one JSON object per line (format {@code ruthenium-trace/1}, see
 * docs/golden-traces.md).
 *
 * <p>Records: {@code frame} (start of a client frame), {@code tick} (inputs at the start of a
 * client tick), {@code state} (player state at the end of the tick), {@code out} (every
 * serverbound packet with its encoded bytes), {@code recv} (a clientbound packet arriving, on the
 * network thread) and {@code handle} (a clientbound packet being processed on the game thread;
 * with bytes only for the packets that affect the local player, so no chunk or registry data is
 * ever written).
 *
 * <p>Floats and doubles are written with Java's shortest round-trip representation, so parsing
 * the text directly as f32 / f64 gives the exact value.
 */
public final class Recorder {
    /** Clientbound packets recorded with their bytes. */
    private static final Set<String> HANDLED_WITH_BYTES = Set.of(
            "minecraft:player_position", "minecraft:player_rotation", "minecraft:set_entity_motion",
            "minecraft:explode", "minecraft:update_attributes", "minecraft:update_mob_effect",
            "minecraft:remove_mob_effect", "minecraft:player_abilities", "minecraft:set_health",
            "minecraft:game_event", "minecraft:ticking_state", "minecraft:ticking_step",
            "minecraft:set_held_slot", "minecraft:keep_alive", "minecraft:ping", "minecraft:damage_event",
            "minecraft:hurt_animation", "minecraft:entity_event", "minecraft:set_entity_data",
            "minecraft:teleport_entity", "minecraft:entity_position_sync", "minecraft:block_changed_ack",
            "minecraft:block_update", "minecraft:section_blocks_update", "minecraft:set_cursor_item",
            "minecraft:container_set_slot", "minecraft:container_set_content", "minecraft:set_player_inventory",
            "minecraft:add_entity", "minecraft:remove_entities", "minecraft:move_entity_pos",
            "minecraft:move_entity_pos_rot", "minecraft:move_entity_rot", "minecraft:rotate_head",
            "minecraft:swing_animation", "minecraft:set_equipment", "minecraft:sound");

    private static final Gson GSON = new GsonBuilder().serializeSpecialFloatingPointValues().disableHtmlEscaping().create();
    private static final Object LOCK = new Object();

    private static BufferedWriter out;
    private static int tick = -1;
    private static long frame = -1;
    private static boolean inTick;
    private static Thread gameThread;
    private static volatile ProtocolInfo<?> clientOutbound;
    private static volatile ProtocolInfo<?> clientInbound;

    private Recorder() {
    }

    /** Starts a trace. Call on the game thread. */
    public static void begin(Path file, JsonObject header) {
        synchronized (LOCK) {
            try {
                Files.createDirectories(file.toAbsolutePath().getParent());
                out = Files.newBufferedWriter(file);
            } catch (IOException e) {
                throw new UncheckedIOException(e);
            }
            tick = -1;
            frame = -1;
            gameThread = Thread.currentThread();
            header.addProperty("k", "header");
            write(header);
        }
    }

    public static void end() {
        synchronized (LOCK) {
            if (out != null) {
                JsonObject o = record("end");
                o.addProperty("ticks", tick + 1);
                write(o);
                try {
                    out.close();
                } catch (IOException e) {
                    throw new UncheckedIOException(e);
                }
                out = null;
            }
        }
    }

    private static void write(JsonObject o) {
        try {
            out.write(GSON.toJson(o));
            out.write('\n');
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }

    private static JsonObject record(String kind) {
        JsonObject o = new JsonObject();
        o.addProperty("k", kind);
        return o;
    }

    public static void protocol(ProtocolInfo<?> protocol) {
        if (protocol.flow() == PacketFlow.SERVERBOUND) {
            clientOutbound = protocol;
        } else {
            clientInbound = protocol;
        }
    }

    public static void frameStart() {
        synchronized (LOCK) {
            if (out != null) {
                frame++;
                JsonObject o = record("frame");
                o.addProperty("f", frame);
                write(o);
            }
        }
    }

    /** Keys the game reads as queued presses (clicks); movement keys are only read as held. */
    private static boolean readsClicks(Options options, KeyMapping key) {
        return key != options.keyUp && key != options.keyDown && key != options.keyLeft && key != options.keyRight
                && key != options.keyJump && key != options.keyShift && key != options.keySprint;
    }

    /** Keys the inputs are read from: movement, actions and the hotbar. */
    private static List<KeyMapping> keys(Options options) {
        List<KeyMapping> keys = new ArrayList<>(List.of(
                options.keyUp, options.keyDown, options.keyLeft, options.keyRight, options.keyJump,
                options.keyShift, options.keySprint, options.keyAttack, options.keyUse, options.keyPickItem,
                options.keyDrop, options.keySwapOffhand, options.keyInventory));
        keys.addAll(List.of(options.keyHotbarSlots));
        return keys;
    }

    public static void tickStart(Minecraft mc) {
        synchronized (LOCK) {
            if (out == null) {
                return;
            }
            tick++;
            inTick = true;
            JsonObject o = record("tick");
            o.addProperty("n", tick);
            o.addProperty("f", frame);
            JsonArray down = new JsonArray();
            JsonObject clicks = new JsonObject();
            for (KeyMapping key : keys(mc.options)) {
                if (key.isDown()) {
                    down.add(key.getName());
                }
                int count = ((KeyMappingAccessor) key).ruthenium$clickCount();
                if (count > 0 && readsClicks(mc.options, key)) {
                    clicks.addProperty(key.getName(), count);
                }
            }
            o.add("down", down);
            o.add("clicks", clicks);
            LocalPlayer p = mc.player;
            if (p != null) {
                o.addProperty("yRot", p.getYRot());
                o.addProperty("xRot", p.getXRot());
                o.addProperty("slot", p.getInventory().getSelectedSlot());
            }
            write(o);
        }
    }

    public static void tickEnd(Minecraft mc) {
        synchronized (LOCK) {
            inTick = false;
            if (out == null || mc.player == null) {
                return;
            }
            LocalPlayer p = mc.player;
            JsonObject o = record("state");
            o.addProperty("n", tick);
            if (mc.level != null) {
                o.addProperty("gameTime", mc.level.getGameTime());
            }
            o.add("pos", vec(p.position()));
            o.add("vel", vec(p.getDeltaMovement()));
            o.addProperty("yRot", p.getYRot());
            o.addProperty("xRot", p.getXRot());
            o.addProperty("onGround", p.onGround());
            o.addProperty("hColl", p.horizontalCollision);
            o.addProperty("minorHColl", p.minorHorizontalCollision);
            o.addProperty("vColl", p.verticalCollision);
            o.addProperty("vCollBelow", p.verticalCollisionBelow);
            o.addProperty("fall", p.fallDistance);
            o.addProperty("sprinting", p.isSprinting());
            o.addProperty("shiftDown", p.isShiftKeyDown());
            o.addProperty("pose", p.getPose().name());
            o.addProperty("flying", p.getAbilities().flying);
            o.addProperty("inWater", p.isInWater());
            Input in = p.input.keyPresses;
            JsonArray input = new JsonArray();
            for (var e : List.of(
                    java.util.Map.entry("forward", in.forward()), java.util.Map.entry("backward", in.backward()),
                    java.util.Map.entry("left", in.left()), java.util.Map.entry("right", in.right()),
                    java.util.Map.entry("jump", in.jump()), java.util.Map.entry("shift", in.shift()),
                    java.util.Map.entry("sprint", in.sprint()))) {
                if (e.getValue()) {
                    input.add(e.getKey());
                }
            }
            o.add("input", input);
            JsonArray move = new JsonArray();
            move.add(p.input.getMoveVector().x);
            move.add(p.input.getMoveVector().y);
            o.add("move", move);
            o.addProperty("health", p.getHealth());
            o.addProperty("food", p.getFoodData().getFoodLevel());
            write(o);
        }
    }

    private static JsonArray vec(Vec3 v) {
        JsonArray a = new JsonArray();
        a.add(v.x);
        a.add(v.y);
        a.add(v.z);
        return a;
    }

    private static String thread() {
        return Thread.currentThread() == gameThread ? "game" : "net";
    }

    private static JsonObject packetRecord(String kind, ProtocolInfo<?> protocol, Packet<?> packet) {
        JsonObject o = record(kind);
        o.addProperty("n", tick);
        o.addProperty("f", frame);
        o.addProperty("thread", thread());
        if (protocol != null) {
            o.addProperty("proto", protocol.id().id());
        }
        o.addProperty("name", packet.type().id().toString());
        return o;
    }

    /** Every serverbound packet, with its bytes (packet id first, as on the wire before framing). */
    public static void sent(Connection connection, Packet<?> packet) {
        if (connection.getSending() != PacketFlow.SERVERBOUND) {
            return;
        }
        synchronized (LOCK) {
            if (out == null) {
                return;
            }
            ProtocolInfo<?> protocol = clientOutbound;
            JsonObject o = packetRecord("out", protocol, packet);
            o.addProperty("inTick", inTick && Thread.currentThread() == gameThread);
            o.addProperty("hex", encode(protocol, packet));
            write(o);
        }
    }

    public static void received(Connection connection, Packet<?> packet) {
        if (connection.getReceiving() != PacketFlow.CLIENTBOUND) {
            return;
        }
        synchronized (LOCK) {
            if (out != null) {
                write(packetRecord("recv", clientInbound, packet));
            }
        }
    }

    public static void handled(PacketListener listener, Packet<?> packet) {
        if (listener.flow() != PacketFlow.CLIENTBOUND) {
            return;
        }
        synchronized (LOCK) {
            if (out == null) {
                return;
            }
            handledOne(packet, false);
            if (packet instanceof BundlePacket<?> bundle) {
                for (Packet<?> sub : bundle.subPackets()) {
                    handledOne(sub, true);
                }
            }
        }
    }

    private static void handledOne(Packet<?> packet, boolean inBundle) {
        ProtocolInfo<?> protocol = clientInbound;
        JsonObject o = packetRecord("handle", protocol, packet);
        if (inBundle) {
            o.addProperty("bundled", true);
        }
        String name = packet.type().id().toString();
        if (HANDLED_WITH_BYTES.contains(name)) {
            o.addProperty("hex", encode(protocol, packet));
        }
        write(o);
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    private static String encode(ProtocolInfo<?> protocol, Packet<?> packet) {
        if (protocol == null) {
            return null;
        }
        ByteBuf buf = Unpooled.buffer();
        try {
            StreamCodec codec = protocol.codec();
            codec.encode(buf, packet);
            return ByteBufUtil.hexDump(buf);
        } catch (RuntimeException e) {
            return null;
        } finally {
            buf.release();
        }
    }
}
