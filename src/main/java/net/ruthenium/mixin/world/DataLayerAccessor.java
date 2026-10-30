package net.ruthenium.mixin.world;

import net.minecraft.world.level.chunk.DataLayer;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

@Mixin(DataLayer.class)
public interface DataLayerAccessor {
    /** The nibble array, null while the layer holds one value everywhere (without creating it, unlike {@code getData()}) */
    @Accessor("data")
    byte @Nullable [] ruthenium$data();
}
