/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.RecordComponent;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LevelAccessor;
import net.minecraft.world.level.LevelReader;
import net.minecraft.world.level.ScheduledTickAccess;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.FireBlock;
import net.minecraft.world.level.block.SupportType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.InteractionHand;
import net.minecraft.util.RandomSource;

/**
 * What the client's placing and using of blocks needs to know of every block that its
 * states don't say: which classes it is, which of them decides each of the behaviours
 * placing and using run, the settings its constructor was given, and per state the
 * flags, the faces it supports other blocks on and the redstone power it gives.
 *
 * <p>Format {@code block_behaviour} v3, after the header:
 * <ul>
 *   <li>u8 instrument count, then per note block instrument in order its name and u8 1
 *       if it works above a note block (mob heads).</li>
 *   <li>u8 method count M, then M strings: the behaviours, in the order every block
 *       lists them.</li>
 *   <li>u32 block count, then per block in registry order: string key; u8 count and the
 *       simple names of the block's class and its superclasses up to
 *       {@code BlockBehaviour}, nearest first; M u8: for each behaviour, the index in
 *       that list of the class that declares it (255: none does, for the optional
 *       {@code mayPlaceOn} and {@code isValidBonemealTarget}); u32 the item the block gives
 *       ({@code asItem}, an item registry id); u8 fire's ignite odds and u8 its burn
 *       odds for the block; u16 count and that many pairs of strings: the
 *       {@code Class.field} and value of each instance field the block's classes
 *       declare below {@code Block}.</li>
 *   <li>u32 state count, then per state in id order: u32 flags (see the constants),
 *       u8 the note block instrument it gives ({@code instrument()}, an index in that
 *       list),
 *       u32 support faces (for each {@code Direction} in ordinal order three bits:
 *       {@code isFaceSturdy} as full, centre and rigid support, at the origin); with
 *       the signal-source flag, also six u8 {@code getSignal} and six u8
 *       {@code getDirectSignal} towards each direction, at the origin of an empty
 *       level.</li>
 * </ul>
 */
final class BlockBehaviourExporter {
    static final String FILE = "block_behaviour.bin";

    static final int REPLACEABLE = 1;
    static final int SOLID = 2;
    // 4 unused.
    static final int SIGNAL_SOURCE = 8;
    static final int REDSTONE_CONDUCTOR = 16;
    static final int COLLISION_FULL_BLOCK = 32;
    static final int BLOCK_ENTITY = 64;
    static final int LIQUID = 128;
    static final int SOLID_RENDER = 256;
    static final int CAN_OCCLUDE = 512;
    static final int IGNITED_BY_LAVA = 1024;
    static final int AIR = 2048;
    static final int RANDOMLY_TICKING = 4096;
    static final int ANALOG_OUTPUT = 8192;
    /**
     * The state suffocates by the default rule, which reads a data-driven tag: when it is in
     * {@code causes_suffocation} and its collision shape is a full block. Tags aren't bound
     * here, so the physics export's suffocating flag is set only for blocks given their own
     * rule.
     */
    static final int SUFFOCATES_BY_TAG = 16384;
    /** Bits 15 to 17: {@code getPistonPushReaction()}, as the enum's ordinal. */
    static final int PUSH_REACTION_SHIFT = 15;

    /** The behaviours, as a name and the parameter types that tell overloads apart. */
    private record Behaviour(String name, String method, boolean optional, Class<?>... params) {
        Behaviour(String name, String method, Class<?>... params) {
            this(name, method, false, params);
        }
    }

    private static final Behaviour[] BEHAVIOURS = {
        new Behaviour("getStateForPlacement", "getStateForPlacement", BlockPlaceContext.class),
        new Behaviour("updateShape", "updateShape", BlockState.class, LevelReader.class, ScheduledTickAccess.class,
            BlockPos.class, Direction.class, BlockPos.class, BlockState.class, RandomSource.class),
        new Behaviour("canSurvive", "canSurvive", BlockState.class, LevelReader.class, BlockPos.class),
        new Behaviour("canBeReplaced", "canBeReplaced", BlockState.class, BlockPlaceContext.class),
        new Behaviour("canBeReplacedByFluid", "canBeReplaced", BlockState.class, Fluid.class),
        new Behaviour("useItemOn", "useItemOn", ItemStack.class, BlockState.class, Level.class, BlockPos.class,
            Player.class, InteractionHand.class, BlockHitResult.class),
        new Behaviour("useWithoutItem", "useWithoutItem", BlockState.class, Level.class, BlockPos.class,
            Player.class, BlockHitResult.class),
        new Behaviour("updateIndirectNeighbourShapes", "updateIndirectNeighbourShapes", BlockState.class,
            LevelAccessor.class, BlockPos.class, int.class, int.class),
        new Behaviour("setPlacedBy", "setPlacedBy", Level.class, BlockPos.class, BlockState.class,
            LivingEntity.class, ItemStack.class),
        new Behaviour("attack", "attack", BlockState.class, Level.class, BlockPos.class, Player.class),
        // VegetationBlock's, which only plants declare.
        new Behaviour("mayPlaceOn", "mayPlaceOn", true, BlockState.class, BlockGetter.class, BlockPos.class),
        // BonemealableBlock's, which only bone-mealable blocks declare.
        new Behaviour("isValidBonemealTarget", "isValidBonemealTarget", true, LevelReader.class, BlockPos.class,
            BlockState.class, net.minecraft.world.level.block.BonemealSource.class),
    };

    private BlockBehaviourExporter() {}

    // isSolid and liquid are deprecated in the game for code that has better questions to ask; these are the flags themselves.
    @SuppressWarnings("deprecation")
    static void export(Path file) throws Exception {
        ExportWriter w = new ExportWriter("block_behaviour", 4);
        // The default suffocation rule is one shared object: every Properties starts with it.
        Field propertiesRule = BlockBehaviour.Properties.class.getDeclaredField("isSuffocating");
        propertiesRule.setAccessible(true);
        Object defaultRule = propertiesRule.get(BlockBehaviour.Properties.of());
        Field stateRule = BlockBehaviour.BlockStateBase.class.getDeclaredField("isSuffocating");
        stateRule.setAccessible(true);
        net.minecraft.world.level.block.state.properties.NoteBlockInstrument[] instruments =
            net.minecraft.world.level.block.state.properties.NoteBlockInstrument.values();
        w.u8(instruments.length);
        for (var i : instruments) {
            w.string(i.getSerializedName());
            w.u8(i.worksAboveNoteBlock() ? 1 : 0);
        }
        w.u8(BEHAVIOURS.length);
        for (Behaviour b : BEHAVIOURS) {
            w.string(b.name());
        }
        // Each behaviour must be declared somewhere, or a signature has changed.
        for (Behaviour b : BEHAVIOURS) {
            if (!b.optional() && declaring(BlockBehaviour.class, b) == null && declaring(Block.class, b) == null) {
                throw new IllegalStateException("no such behaviour: " + b.name());
            }
        }
        Method igniteOdds = FireBlock.class.getDeclaredMethod("getIgniteOdds", BlockState.class);
        igniteOdds.setAccessible(true);
        Method burnOdds = FireBlock.class.getDeclaredMethod("getBurnOdds", BlockState.class);
        burnOdds.setAccessible(true);

        w.u32(BuiltInRegistries.BLOCK.size());
        for (Block block : BuiltInRegistries.BLOCK) {
            w.string(BuiltInRegistries.BLOCK.getKey(block).toString());
            List<Class<?>> chain = new ArrayList<>();
            for (Class<?> c = block.getClass(); c != null && BlockBehaviour.class.isAssignableFrom(c); c = c.getSuperclass()) {
                chain.add(c);
            }
            w.u8(chain.size());
            for (Class<?> c : chain) {
                w.string(c.getSimpleName());
            }
            for (Behaviour b : BEHAVIOURS) {
                int index = -1;
                for (int i = 0; i < chain.size() && index < 0; i++) {
                    if (declaring(chain.get(i), b) != null) index = i;
                }
                if (index < 0 && !b.optional()) throw new IllegalStateException(block + " has no " + b.name());
                w.u8(index < 0 ? 255 : index);
            }
            w.u32(BuiltInRegistries.ITEM.getId(block.asItem()));
            BlockState plain = block.defaultBlockState();
            w.u8((Integer) igniteOdds.invoke(Blocks.FIRE, plain));
            w.u8((Integer) burnOdds.invoke(Blocks.FIRE, plain));
            List<String[]> fields = new ArrayList<>();
            for (Class<?> c : chain) {
                if (c == Block.class || c == BlockBehaviour.class) continue;
                for (Field f : c.getDeclaredFields()) {
                    if (Modifier.isStatic(f.getModifiers()) || f.isSynthetic()) continue;
                    f.setAccessible(true);
                    fields.add(new String[] {c.getSimpleName() + "." + f.getName(), render(f.get(block), 0)});
                }
            }
            w.u8(fields.size() & 0xFF);
            w.u8(fields.size() >>> 8);
            for (String[] f : fields) {
                w.string(f[0]);
                w.string(f[1]);
            }
        }

        BlockGetter empty = EmptyBlockGetter.INSTANCE;
        w.u32(Block.BLOCK_STATE_REGISTRY.size());
        int count = 0;
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            if (Block.BLOCK_STATE_REGISTRY.getId(state) != count) {
                throw new IllegalStateException("block states out of order at " + count);
            }
            count++;
            int flags = 0;
            if (state.canBeReplaced()) flags |= REPLACEABLE;
            if (state.isSolid()) flags |= SOLID;
            if (state.isSignalSource()) flags |= SIGNAL_SOURCE;
            if (state.isRedstoneConductor(empty, BlockPos.ZERO)) flags |= REDSTONE_CONDUCTOR;
            if (state.isCollisionShapeFullBlock(empty, BlockPos.ZERO)) flags |= COLLISION_FULL_BLOCK;
            if (state.hasBlockEntity()) flags |= BLOCK_ENTITY;
            if (state.liquid()) flags |= LIQUID;
            if (state.isSolidRender()) flags |= SOLID_RENDER;
            if (state.canOcclude()) flags |= CAN_OCCLUDE;
            if (state.ignitedByLava()) flags |= IGNITED_BY_LAVA;
            if (state.isAir()) flags |= AIR;
            if (state.isRandomlyTicking()) flags |= RANDOMLY_TICKING;
            if (state.hasAnalogOutputSignal()) flags |= ANALOG_OUTPUT;
            if (stateRule.get(state) == defaultRule) flags |= SUFFOCATES_BY_TAG;
            flags |= state.getPistonPushReaction().ordinal() << PUSH_REACTION_SHIFT;
            w.u32(flags);
            w.u8(state.instrument().ordinal());
            int support = 0;
            SupportType[] types = {SupportType.FULL, SupportType.CENTER, SupportType.RIGID};
            for (Direction d : Direction.values()) {
                for (int t = 0; t < 3; t++) {
                    if (state.isFaceSturdy(empty, BlockPos.ZERO, d, types[t])) support |= 1 << (d.ordinal() * 3 + t);
                }
            }
            w.u32(support);
            if ((flags & SIGNAL_SOURCE) != 0) {
                for (Direction d : Direction.values()) {
                    w.u8(state.getSignal(empty, BlockPos.ZERO, d));
                }
                for (Direction d : Direction.values()) {
                    w.u8(state.getDirectSignal(empty, BlockPos.ZERO, d));
                }
            }
        }
        w.writeTo(file);
    }

    /** The method if class c itself declares the behaviour. */
    private static Method declaring(Class<?> c, Behaviour b) {
        try {
            Method m = c.getDeclaredMethod(b.method(), b.params());
            return m.isSynthetic() || m.isBridge() ? null : m;
        } catch (NoSuchMethodException e) {
            return null;
        }
    }

    /** A field's value as text: registry entries by key, states by id, records by their plain parts. */
    static String render(Object v, int depth) throws IllegalAccessException {
        if (v == null) return "null";
        if (v instanceof Number || v instanceof Boolean || v instanceof Character || v instanceof String) {
            return String.valueOf(v);
        }
        if (v instanceof Enum<?> e) return e.name();
        if (v instanceof Block b) return BuiltInRegistries.BLOCK.getKey(b).toString();
        if (v instanceof Item i) return BuiltInRegistries.ITEM.getKey(i).toString();
        if (v instanceof BlockState s) return "state:" + Block.BLOCK_STATE_REGISTRY.getId(s);
        if (v instanceof net.minecraft.core.Holder<?> h) {
            return h.unwrapKey().map(k -> k.identifier().toString()).orElse("holder");
        }
        if (v instanceof net.minecraft.resources.ResourceKey<?> k) return k.identifier().toString();
        if (v instanceof net.minecraft.tags.TagKey<?> t) return "#" + t.location();
        if (depth < 2 && v instanceof Object[] array) {
            List<String> parts = new ArrayList<>();
            for (Object o : array) parts.add(render(o, depth + 1));
            return "[" + String.join(",", parts) + "]";
        }
        if (depth < 2 && v instanceof java.util.Map<?, ?> map) {
            List<String> keys = new ArrayList<>();
            for (Object k : map.keySet()) keys.add(render(k, depth + 1));
            java.util.Collections.sort(keys);
            return "{" + String.join(",", keys) + "}";
        }
        if (depth < 2 && v instanceof java.util.Collection<?> list) {
            List<String> parts = new ArrayList<>();
            for (Object o : list) parts.add(render(o, depth + 1));
            return "[" + String.join(",", parts) + "]";
        }
        if (v.getClass().isRecord() && depth < 2) {
            StringBuilder sb = new StringBuilder(v.getClass().getSimpleName()).append('[');
            boolean first = true;
            for (RecordComponent rc : v.getClass().getRecordComponents()) {
                Object part;
                try {
                    part = rc.getAccessor().invoke(v);
                } catch (ReflectiveOperationException e) {
                    continue;
                }
                if (!(part == null || part instanceof Number || part instanceof Boolean || part instanceof String
                        || part instanceof Enum<?> || part instanceof Block || part instanceof Item)) {
                    continue;
                }
                if (!first) sb.append(',');
                first = false;
                sb.append(rc.getName()).append('=').append(render(part, depth + 1));
            }
            return sb.append(']').toString();
        }
        Class<?> c = v.getClass();
        if (depth == 0 && !c.isHidden() && !c.isSynthetic() && c.getName().startsWith("net.minecraft.")
                && !c.isAnonymousClass()) {
            StringBuilder sb = new StringBuilder(c.getSimpleName()).append('{');
            boolean first = true;
            for (Field f : c.getDeclaredFields()) {
                if (Modifier.isStatic(f.getModifiers()) || f.isSynthetic()) continue;
                try {
                    f.setAccessible(true);
                } catch (RuntimeException e) {
                    continue;
                }
                if (!first) sb.append(';');
                first = false;
                sb.append(f.getName()).append('=').append(render(f.get(v), depth + 1));
            }
            String s = sb.append('}').toString();
            return s.length() > 16000 ? s.substring(0, 16000) : s;
        }
        return c.getSimpleName();
    }
}
