package net.thallium.mixin.render.chunk;

import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.chunk.RenderSectionRegion;
import net.minecraft.client.renderer.chunk.SectionCopy;
import net.minecraft.core.BlockPos;
import net.minecraft.core.SectionPos;
import net.minecraft.world.level.ColorResolver;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.thallium.Initializer;
import net.thallium.render.chunk.BiomeTintBlend;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

/**
 * The block states of the region's center section and the blocks around it (18^3) are read from the section copies
 * once and then from an array: culling and smooth lighting look up each block's neighbours many times. The region is
 * an immutable snapshot, so the states are the same. The array is per thread and owned by one region at a time
 * (vanilla uses a region on one thread; another thread takes its own array).
 * <p>
 * Block tints go straight to {@link BiomeTintBlend}, which keeps the section's blended tints itself, instead of
 * through vanilla's per-block tint cache (a read lock and a linear search of the column's heights per lookup).
 */
@Mixin(RenderSectionRegion.class)
public class RenderSectionRegionM {
    @Unique
    private static final int SIZE = 18;
    @Unique
    private static final ThreadLocal<States> STATES = ThreadLocal.withInitial(States::new);

    @Shadow @Final private int minSectionX;
    @Shadow @Final private int minSectionY;
    @Shadow @Final private int minSectionZ;
    @Shadow @Final private ClientLevel level;

    @Shadow
    private SectionCopy getSection(int sectionX, int sectionY, int sectionZ) {
        throw new AssertionError();
    }

    @Unique
    private States thallium$states;

    @Unique
    private static final class States {
        final BlockState[] states = new BlockState[SIZE * SIZE * SIZE];
        final int[] stamps = new int[SIZE * SIZE * SIZE];
        final Thread thread = Thread.currentThread();
        int stamp;
        Object owner;
    }

    /**
     * @author Thallium
     * @reason Cached block states (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public BlockState getBlockState(final BlockPos pos) {
        int x = pos.getX() - (this.minSectionX << 4) - 15;
        int y = pos.getY() - (this.minSectionY << 4) - 15;
        int z = pos.getZ() - (this.minSectionZ << 4) - 15;
        if ((x | y | z) < 0 || x >= SIZE || y >= SIZE || z >= SIZE || !Initializer.CONFIG.sectionStateCache) {
            return this.thallium$lookUp(pos);
        }

        States states = this.thallium$states;
        if (states == null || states.owner != this || states.thread != Thread.currentThread()) {
            states = STATES.get();
            if (++states.stamp == 0) {
                java.util.Arrays.fill(states.stamps, 0);
                states.stamp = 1;
            }
            states.owner = this;
            this.thallium$states = states;
        }
        int index = (y * SIZE + z) * SIZE + x;
        if (states.stamps[index] == states.stamp) {
            return states.states[index];
        }
        BlockState state = this.thallium$lookUp(pos);
        states.states[index] = state;
        states.stamps[index] = states.stamp;
        return state;
    }

    /**
     * @author Thallium
     * @reason Through the cached block states
     */
    @Overwrite
    public FluidState getFluidState(final BlockPos pos) {
        return this.getBlockState(pos).getFluidState();
    }

    /**
     * @author Thallium
     * @reason Tints without vanilla's per-block cache (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public int getBlockTint(final BlockPos pos, final ColorResolver resolver) {
        if (Initializer.CONFIG.biomeBlendCache) {
            int radius = Minecraft.getInstance().options.biomeBlendRadius().get();
            if (BiomeTintBlend.supports(radius)) {
                return BiomeTintBlend.blend(this.level, pos, resolver, radius);
            }
        }
        return this.level.getBlockTint(pos, resolver);
    }

    @Unique
    private BlockState thallium$lookUp(BlockPos pos) {
        return this.getSection(SectionPos.blockToSectionCoord(pos.getX()), SectionPos.blockToSectionCoord(pos.getY()),
                SectionPos.blockToSectionCoord(pos.getZ())).getBlockState(pos);
    }
}
