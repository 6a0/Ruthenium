package net.vulkanmod.interfaces;

/**
 * Packed positions ({@code BlockPos.asLong}) and cullability of a compiled section's renderable block entities,
 * in list order, so block entity culling can test them without dereferencing every block entity.
 */
public interface BlockEntityPositions {

    /** @return the packed positions, built on first use; index i belongs to the list's i-th block entity */
    long[] vulkanmod$getPositions();

    /** @return whether the list's i-th block entity may be culled, see {@code BlockEntityCulling.isCullable} */
    boolean[] vulkanmod$getCullable();
}
