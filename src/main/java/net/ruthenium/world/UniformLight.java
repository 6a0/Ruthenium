package net.ruthenium.world;

import java.util.Arrays;
import net.minecraft.world.level.chunk.DataLayer;
import net.ruthenium.mixin.world.DataLayerAccessor;
import org.jspecify.annotations.Nullable;

/**
 * Switch {@code compactSkyLight}: light data that arrives as a 2048-byte array with every nibble at 15 (a section in
 * open sky) is stored as {@code new DataLayer(15)}, which keeps no array until something writes to it. Vanilla creates
 * such layers itself for sections it lights from scratch, so every reader already handles them: {@code get} returns
 * the default, {@code getData()} creates the filled array on demand (writes, network, saving), {@code copy()} keeps
 * the form. Only full sky light is converted. An all-zero array would become {@link DataLayer#isEmpty()}, which the
 * sky light engine fills with 15 when a chunk's light is enabled and the server neither saves nor sends; such arrays
 * are also rare (a handful at render distance 12).
 */
public final class UniformLight {
    private static final byte[] FULL = new byte[DataLayer.SIZE];

    static {
        Arrays.fill(FULL, (byte) 0xFF);
    }

    private UniformLight() {
    }

    /** {@code layer}, or a layer without an array when its array holds 15 everywhere */
    public static @Nullable DataLayer compact(@Nullable DataLayer layer) {
        if (layer == null) {
            return null;
        }
        byte[] data = ((DataLayerAccessor) layer).ruthenium$data();
        // Arrays.mismatch is vectorized; a mixed array usually differs in its first bytes and returns at once
        if (data != null && data.length == DataLayer.SIZE && Arrays.mismatch(data, FULL) < 0) {
            return new DataLayer(15);
        }
        return layer;
    }
}
