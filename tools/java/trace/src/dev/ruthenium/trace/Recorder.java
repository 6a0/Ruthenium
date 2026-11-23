package dev.ruthenium.trace;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.blaze3d.platform.InputConstants;
import dev.ruthenium.trace.mixin.KeyMappingAccessor;
import dev.ruthenium.trace.mixin.LivingEntityAccessor;
import dev.ruthenium.trace.mixin.MultiPlayerGameModeAccessor;
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
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.network.Connection;
import net.minecraft.network.PacketListener;
import net.minecraft.network.ProtocolInfo;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.BundlePacket;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.EntityHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;

/**
 * Writes a golden trace: one JSON object per line (format {@code ruthenium-trace/1}, see
 * docs/golden-traces.md).
 *
 * <p>Records: {@code frame} (start of a client frame), {@code tick} (inputs at the start of a
 * client tick), {@code state} (player state at the end of the tick), {@code out} (every
 * serverbound packet with its encoded bytes), {@code recv} (a clientbound packet arriving, on the
 * network thread), {@code handle} (a clientbound packet being processed on the game thread;
 * with bytes only for the packets that affect the local player, so no chunk or registry data is
 * ever written) and {@code set} (a block the client level sets: predicted, acknowledged or
 * updated by the server).
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
            "minecraft:block_update", "minecraft:section_blocks_update", "minecraft:block_event", "minecraft:set_cursor_item",
            "minecraft:container_set_slot", "minecraft:container_set_content", "minecraft:set_player_inventory",
            "minecraft:add_entity", "minecraft:remove_entities", "minecraft:move_entity_pos",
            "minecraft:move_entity_pos_rot", "minecraft:move_entity_rot", "minecraft:rotate_head",
            "minecraft:swing_animation", "minecraft:set_equipment", "minecraft:sound",
            "minecraft:set_passengers", "minecraft:set_player_team", "minecraft:player_info_update",
            "minecraft:cooldown", "minecraft:player_combat_kill", "minecraft:respawn", "minecraft:move_vehicle",
            "minecraft:initialize_border", "minecraft:set_border_center", "minecraft:set_border_lerp_size",
            "minecraft:set_border_size", "minecraft:set_border_warning_delay", "minecraft:set_border_warning_distance");

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
            // The keyboard keys physically held (what closing a screen reads back into the
            // key mappings, `KeyMapping.setAll`).
            JsonArray physical = new JsonArray();
            for (KeyMapping key : keys(mc.options)) {
                InputConstants.Key bound = ((KeyMappingAccessor) key).ruthenium$key();
                if (bound.getType() == InputConstants.Type.KEYBOARD && bound.getValue() != InputConstants.UNKNOWN.getValue()
                        && InputConstants.isKeyDown(bound.getValue())) {
                    physical.add(key.getName());
                }
            }
            o.add("physical", physical);
            LocalPlayer p = mc.player;
            if (p != null) {
                o.addProperty("yRot", p.getYRot());
                o.addProperty("xRot", p.getXRot());
                o.addProperty("slot", p.getInventory().getSelectedSlot());
            }
            write(o);
        }
    }

    /**
     * A block the client level sets (a {@code set} record): where, the state, the update flags
     * and limit, and whether an action's prediction is running. Game thread only.
     */
    public static void blockSet(BlockPos pos, net.minecraft.world.level.block.state.BlockState state, int flags,
            int limit, boolean predicting) {
        synchronized (LOCK) {
            if (out == null || Thread.currentThread() != gameThread) {
                return;
            }
            JsonObject o = record("set");
            o.addProperty("n", tick);
            o.addProperty("inTick", inTick);
            JsonArray at = new JsonArray();
            at.add(pos.getX());
            at.add(pos.getY());
            at.add(pos.getZ());
            o.add("pos", at);
            o.addProperty("state", Block.getId(state));
            o.addProperty("flags", flags);
            o.addProperty("limit", limit);
            o.addProperty("predicting", predicting);
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
            o.addProperty("yHeadRot", p.getYHeadRot());
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
            // Gliding (the shared flag), and a riptide spin: the server's living flag and the
            // client's ticks left.
            o.addProperty("fallFlying", p.isFallFlying());
            o.addProperty("autoSpin", p.isAutoSpinAttack());
            o.addProperty("spinTicks", ((LivingEntityAccessor) p).ruthenium$autoSpinAttackTicks());
            // Riding: the vehicle, and the root vehicle's state (what the client moves
            // itself when the player controls it); the horse jump's charge.
            net.minecraft.world.entity.Entity vehicle = p.getVehicle();
            if (vehicle != null) {
                net.minecraft.world.entity.Entity root = p.getRootVehicle();
                o.addProperty("vehicle", vehicle.getId());
                o.addProperty("vRoot", root.getId());
                o.add("vPos", vec(root.position()));
                o.add("vVel", vec(root.getDeltaMovement()));
                o.addProperty("vYRot", root.getYRot());
                o.addProperty("vXRot", root.getXRot());
                o.addProperty("vOnGround", root.onGround());
                o.addProperty("vHColl", root.horizontalCollision);
                o.addProperty("vVColl", root.verticalCollision);
                o.addProperty("vControlled", root.isLocalInstanceAuthoritative());
            }
            o.addProperty("jumpScale", p.getJumpRidingScale());
            o.addProperty("handsBusy", p.isHandsBusy());
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
            // Dying: the ticks since, and whether the client has taken the player out of
            // the level; whether the player has loaded into the level (not after a
            // respawn until the level is ready); the screen open, by class.
            o.addProperty("deathTime", p.deathTime);
            o.addProperty("removed", p.isRemoved());
            if (mc.getConnection() != null) {
                o.addProperty("loaded", mc.getConnection().hasClientLoaded());
            }
            o.addProperty("screen", mc.gui.screen() == null ? "" : mc.gui.screen().getClass().getSimpleName());
            o.addProperty("food", p.getFoodData().getFoodLevel());
            o.addProperty("saturation", p.getFoodData().getSaturationLevel());
            // Item use and the hands.
            o.addProperty("using", p.isUsingItem());
            if (p.isUsingItem()) {
                o.addProperty("useHand", p.getUsedItemHand().name());
                o.addProperty("useItem", stack(p.getUseItem()));
                o.addProperty("useRemaining", p.getUseItemRemainingTicks());
            }
            o.addProperty("main", stack(p.getMainHandItem()));
            o.addProperty("off", stack(p.getOffhandItem()));
            // What the crosshair is on, as this tick's pick found it.
            HitResult hit = mc.hitResult;
            o.addProperty("hit", hit == null ? "NONE" : hit.getType().name());
            if (hit instanceof BlockHitResult b && b.getType() == HitResult.Type.BLOCK) {
                JsonArray pos = new JsonArray();
                pos.add(b.getBlockPos().getX());
                pos.add(b.getBlockPos().getY());
                pos.add(b.getBlockPos().getZ());
                o.add("hitBlock", pos);
                o.addProperty("hitFace", b.getDirection().getSerializedName());
                o.add("hitAt", vec(b.getLocation()));
            } else if (hit instanceof EntityHitResult e) {
                o.addProperty("hitEntity", e.getEntity().getId());
                o.add("hitAt", vec(e.getLocation()));
            }
            // Breaking: the block (kept after breaking ends), its state now, the progress,
            // the ticks spent and the delay before the next block.
            MultiPlayerGameMode gameMode = mc.gameMode;
            if (gameMode != null && mc.level != null) {
                MultiPlayerGameModeAccessor a = (MultiPlayerGameModeAccessor) gameMode;
                BlockPos at = a.ruthenium$destroyBlockPos();
                o.addProperty("destroying", gameMode.isDestroying());
                JsonArray pos = new JsonArray();
                pos.add(at.getX());
                pos.add(at.getY());
                pos.add(at.getZ());
                o.add("destroyPos", pos);
                o.addProperty("destroyState", Block.getId(mc.level.getBlockState(at)));
                o.addProperty("destroyProgress", a.ruthenium$destroyProgress());
                o.addProperty("destroyTicks", a.ruthenium$destroyTicks());
                o.addProperty("destroyDelay", a.ruthenium$destroyDelay());
            }
            write(o);
        }
    }

    /** An item stack as {@code id*count}, empty as "". */
    private static String stack(ItemStack s) {
        return s.isEmpty() ? "" : BuiltInRegistries.ITEM.getKey(s.getItem()) + "*" + s.getCount();
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
