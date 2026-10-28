package net.vulkanmod.world;

import net.minecraft.core.BlockPos;
import net.minecraft.core.SectionPos;
import net.minecraft.util.Mth;
import net.minecraft.util.profiling.Profiler;
import net.minecraft.util.profiling.ProfilerFiller;
import net.minecraft.world.entity.EntitySelector;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.EnchantingTableBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import net.vulkanmod.interfaces.ParkableTicker;

import java.util.Arrays;
import java.util.List;

/**
 * The client's enchanting table tickers, ticked in one loop instead of through the level's ticker list.
 * <p>
 * Every loaded enchanting table animates its book every tick (the closed book keeps turning), and on a client with
 * thousands of them most of the time each took was the way to the animation, not the animation: the ticker
 * wrapper, the bound ticker, the block entity and its position (scattered objects, each a likely cache miss with
 * thousands of tables), the guards in front of the tick, the profiler and a nearest player search through the
 * level's player list with an entity predicate per table. The tickers are parked ({@link ParkableTicker}) into
 * this batch, which keeps each table's position and runs the same guards and, unless a player that isn't a
 * spectator is within the 3 blocks {@link EnchantingTableBlockEntity#bookAnimationTick} searches, the same float
 * arithmetic as its branch without a player (so the same results bit for bit; tables with a player near call
 * vanilla's tick). The players are read once per tick: block entity ticks on the client don't move them.
 * <p>
 * Tables keep the order they joined in, which is their order in the ticker list. A ticker leaves when its wrapper is
 * rebound or unparked ({@link #remove}, from the wrapper) or its block entity is removed; a rebound table rejoins at
 * the end, where vanilla keeps its place in the list. Their order only matters for the tables' shared random source,
 * which is only used with a player near and is unseeded.
 * <p>
 * Client levels only, render thread only.
 */
public final class EnchantingTableBatch {
    private static final double RANGE_SQR = 3.0 * 3.0;

    private ParkableTicker[] wrappers = new ParkableTicker[256];
    /** Null where a ticker left */
    private EnchantingTableBlockEntity[] tables = new EnchantingTableBlockEntity[256];
    private long[] positions = new long[256];
    private int size;
    private double[] players = new double[3 * 4];

    public int size() {
        return this.size;
    }

    /** Adds a table whose wrapper was just parked into this batch; returns its slot */
    public int add(ParkableTicker wrapper, EnchantingTableBlockEntity table) {
        if (this.size == this.wrappers.length) {
            int capacity = this.size * 2;
            this.wrappers = Arrays.copyOf(this.wrappers, capacity);
            this.tables = Arrays.copyOf(this.tables, capacity);
            this.positions = Arrays.copyOf(this.positions, capacity);
        }
        this.wrappers[this.size] = wrapper;
        this.tables[this.size] = table;
        this.positions[this.size] = table.getBlockPos().asLong();
        return this.size++;
    }

    /** Called by a batched wrapper when it leaves (rebound, unparked) */
    public void remove(int slot) {
        this.tables[slot] = null;
    }

    /**
     * Ticks the tables (if {@code tick}, i.e. the tick rate manager runs normally) and drops the ones that left,
     * with the guards of vanilla's {@code Level.tickBlockEntities} and {@code BoundTickingBlockEntity.tick}
     */
    public void tick(Level level, boolean tick) {
        int playerCount = tick ? this.collectPlayers(level) : 0;
        double[] players = this.players;
        WorldBorder border = level.getWorldBorder();
        ParkableTicker[] wrappers = this.wrappers;
        EnchantingTableBlockEntity[] tables = this.tables;
        long[] positions = this.positions;
        int size = this.size;
        int kept = 0;

        ProfilerFiller profiler = Profiler.get();
        profiler.push("vulkanmod:enchantingTables");
        int i = 0;
        try {
            for (; i < size; i++) {
                EnchantingTableBlockEntity table = tables[i];
                if (table == null) {
                    // Left: back in the level's ticker list, or its block entity is gone
                    continue;
                }
                if (table.isRemoved()) {
                    // The level's loop drops tickers of removed block entities
                    wrappers[i].vulkanmod$leaveBatch();
                    continue;
                }
                long pos = positions[i];
                if (kept != i) {
                    ParkableTicker wrapper = wrappers[i];
                    wrappers[kept] = wrapper;
                    tables[kept] = table;
                    positions[kept] = pos;
                    wrapper.vulkanmod$setBatchSlot(kept);
                }
                kept++;

                if (!tick) {
                    continue;
                }
                int bx = BlockPos.getX(pos), by = BlockPos.getY(pos), bz = BlockPos.getZ(pos);
                // Level.shouldTickBlocksAt(BlockPos), BoundTickingBlockEntity.tick's level and LevelChunk.isTicking's border
                if (!level.shouldTickBlocksAt(ChunkPos.pack(SectionPos.blockToSectionCoord(bx), SectionPos.blockToSectionCoord(bz)))
                        || !table.hasLevel() || !border.isWithinBounds(bx, bz)) {
                    continue;
                }

                double x = bx + 0.5, y = by + 0.5, z = bz + 0.5;
                boolean near = false;
                for (int p = 0; p < playerCount * 3; p += 3) {
                    // Entity.distanceToSqr(x, y, z)
                    double xd = players[p] - x, yd = players[p + 1] - y, zd = players[p + 2] - z;
                    if (xd * xd + yd * yd + zd * zd < RANGE_SQR) {
                        near = true;
                        break;
                    }
                }
                if (near) {
                    BlockState state = wrappers[kept - 1].vulkanmod$tickedState();
                    if (state != null) {
                        EnchantingTableBlockEntity.bookAnimationTick(level, table.getBlockPos(), state, table);
                    }
                } else {
                    tickWithoutPlayer(table);
                }
            }
        } finally {
            // Only after a tick threw (the throwing table is kept already): keep the tables not visited yet
            for (int j = i + 1; j < size; j++) {
                if (tables[j] != null) {
                    ParkableTicker wrapper = wrappers[j];
                    wrappers[kept] = wrapper;
                    tables[kept] = tables[j];
                    positions[kept] = positions[j];
                    wrapper.vulkanmod$setBatchSlot(kept);
                    kept++;
                }
            }
            if (kept < size) {
                Arrays.fill(wrappers, kept, size, null);
                Arrays.fill(tables, kept, size, null);
            }
            this.size = kept;
            profiler.pop();
        }
    }

    /** The positions of the level's players that {@code EntitySelector.NO_SPECTATORS} accepts */
    private int collectPlayers(Level level) {
        List<? extends Player> list = level.players();
        int count = 0;
        for (int i = 0; i < list.size(); i++) {
            Player player = list.get(i);
            if (EntitySelector.NO_SPECTATORS.test(player)) {
                if (count * 3 == this.players.length) {
                    this.players = Arrays.copyOf(this.players, this.players.length * 2);
                }
                this.players[count * 3] = player.getX();
                this.players[count * 3 + 1] = player.getY();
                this.players[count * 3 + 2] = player.getZ();
                count++;
            }
        }
        return count;
    }

    /** {@link EnchantingTableBlockEntity#bookAnimationTick} without a player within 3 blocks, statement for statement */
    private static void tickWithoutPlayer(EnchantingTableBlockEntity entity) {
        entity.oOpen = entity.open;
        entity.oRot = entity.rot;
        entity.tRot += 0.02F;
        entity.open -= 0.1F;

        while (entity.rot >= (float) Math.PI) {
            entity.rot -= (float) (Math.PI * 2);
        }

        while (entity.rot < (float) -Math.PI) {
            entity.rot += (float) (Math.PI * 2);
        }

        while (entity.tRot >= (float) Math.PI) {
            entity.tRot -= (float) (Math.PI * 2);
        }

        while (entity.tRot < (float) -Math.PI) {
            entity.tRot += (float) (Math.PI * 2);
        }

        float rotDir = entity.tRot - entity.rot;

        while (rotDir >= (float) Math.PI) {
            rotDir -= (float) (Math.PI * 2);
        }

        while (rotDir < (float) -Math.PI) {
            rotDir += (float) (Math.PI * 2);
        }

        entity.rot += rotDir * 0.4F;
        entity.open = Mth.clamp(entity.open, 0.0F, 1.0F);
        entity.time++;
        entity.oFlip = entity.flip;
        float diff = (entity.flipT - entity.flip) * 0.4F;
        diff = Mth.clamp(diff, -0.2F, 0.2F);
        entity.flipA = entity.flipA + (diff - entity.flipA) * 0.9F;
        entity.flip = entity.flip + entity.flipA;
    }

    /** Puts every table back into the level's ticker list */
    public void unparkAll() {
        for (int i = 0; i < this.size; i++) {
            if (this.tables[i] != null) {
                this.wrappers[i].vulkanmod$unpark();
            }
        }
        Arrays.fill(this.wrappers, 0, this.size, null);
        Arrays.fill(this.tables, 0, this.size, null);
        this.size = 0;
    }
}
