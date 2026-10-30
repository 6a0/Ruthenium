package net.ruthenium.mixin.render.chunk;

import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.chunk.RenderSectionRegion;
import net.minecraft.client.renderer.chunk.SectionCopy;
import net.minecraft.core.BlockPos;
import net.minecraft.core.SectionPos;
import net.minecraft.world.level.ColorResolver;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.SectionScratchHolder;
import net.ruthenium.render.chunk.BiomeTintBlend;
import net.ruthenium.render.chunk.SectionScratch;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

/**
 * The block states of the region's center section and the blocks around it (18^3) are read from the section copies
 * once and then from an array: culling and smooth lighting look up each block's neighbours many times. The region is
 * an immutable snapshot, so the states are the same. The array is the compile's scratch ({@link SectionScratch}),
 * which {@code SectionCompilerM} binds to the region for the compile on the compiling thread; lookups on another
 * thread, outside the compile or after the scratch was bound to another region read the section copies directly.
 * <p>
 * Block tints go straight to {@link BiomeTintBlend}, which keeps the section's blended tints itself in the same
 * scratch, instead of through vanilla's per-block tint cache (a read lock and a linear search of the column's heights
 * per lookup).
 */
@Mixin(RenderSectionRegion.class)
public class RenderSectionRegionM implements SectionScratchHolder {
    @Unique
    private static final int SIZE = SectionScratch.WINDOW;

    @Shadow @Final private int minSectionX;
    @Shadow @Final private int minSectionY;
    @Shadow @Final private int minSectionZ;
    @Shadow @Final private ClientLevel level;

    @Shadow
    private SectionCopy getSection(int sectionX, int sectionY, int sectionZ) {
        throw new AssertionError();
    }

    /** The scratch of the compile this region is in, on {@link #ruthenium$thread}, or null */
    @Unique
    private SectionScratch ruthenium$scratch;
    /** Its block state window, or null while switch {@code sectionStateCache} was off when it was bound */
    @Unique
    private SectionScratch.States ruthenium$states;
    @Unique
    private Thread ruthenium$thread;
    /** The scratch's binding number when this region was bound: it is still bound while that is unchanged */
    @Unique
    private int ruthenium$bind;

    @Override
    public void ruthenium$bindScratch(SectionScratch scratch) {
        this.ruthenium$bind = scratch.bind();
        this.ruthenium$states = scratch.newStates();
        this.ruthenium$thread = Thread.currentThread();
        this.ruthenium$scratch = scratch;
    }

    @Override
    public void ruthenium$unbindScratch() {
        this.ruthenium$scratch = null;
        this.ruthenium$states = null;
        this.ruthenium$thread = null;
    }

    /** The bound scratch if this thread may use it, else null */
    @Unique
    private SectionScratch ruthenium$boundScratch() {
        SectionScratch scratch = this.ruthenium$scratch;
        return scratch != null && this.ruthenium$thread == Thread.currentThread() && scratch.binds() == this.ruthenium$bind
                ? scratch : null;
    }

    /**
     * @author Ruthenium
     * @reason Cached block states (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public BlockState getBlockState(final BlockPos pos) {
        int x = pos.getX() - (this.minSectionX << 4) - 15;
        int y = pos.getY() - (this.minSectionY << 4) - 15;
        int z = pos.getZ() - (this.minSectionZ << 4) - 15;
        SectionScratch.States states = this.ruthenium$states;
        if ((x | y | z) < 0 || x >= SIZE || y >= SIZE || z >= SIZE || states == null || !Initializer.CONFIG.sectionStateCache
                || this.ruthenium$boundScratch() == null) {
            return this.ruthenium$lookUp(pos);
        }

        int index = (y * SIZE + z) * SIZE + x;
        if (states.stamps[index] == states.stamp) {
            return states.states[index];
        }
        BlockState state = this.ruthenium$lookUp(pos);
        states.states[index] = state;
        states.stamps[index] = states.stamp;
        return state;
    }

    /**
     * @author Ruthenium
     * @reason Through the cached block states
     */
    @Overwrite
    public FluidState getFluidState(final BlockPos pos) {
        return this.getBlockState(pos).getFluidState();
    }

    /**
     * @author Ruthenium
     * @reason Tints without vanilla's per-block cache (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public int getBlockTint(final BlockPos pos, final ColorResolver resolver) {
        if (Initializer.CONFIG.biomeBlendCache) {
            int radius = Minecraft.getInstance().options.biomeBlendRadius().get();
            if (BiomeTintBlend.supports(radius)) {
                SectionScratch scratch = this.ruthenium$boundScratch();
                return scratch != null ? BiomeTintBlend.blend(scratch, this.level, pos, resolver, radius)
                        : BiomeTintBlend.blend(this.level, pos, resolver, radius);
            }
        }
        return this.level.getBlockTint(pos, resolver);
    }

    @Unique
    private BlockState ruthenium$lookUp(BlockPos pos) {
        return this.getSection(SectionPos.blockToSectionCoord(pos.getX()), SectionPos.blockToSectionCoord(pos.getY()),
                SectionPos.blockToSectionCoord(pos.getZ())).getBlockState(pos);
    }
}
