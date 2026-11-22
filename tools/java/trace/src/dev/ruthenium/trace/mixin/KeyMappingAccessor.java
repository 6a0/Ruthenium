package dev.ruthenium.trace.mixin;

import com.mojang.blaze3d.platform.InputConstants;
import net.minecraft.client.KeyMapping;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

/** Presses queued for the tick (consumed by the game's click handlers), and the bound key. */
@Mixin(KeyMapping.class)
public interface KeyMappingAccessor {
    @Accessor("clickCount")
    int ruthenium$clickCount();

    @Accessor("key")
    InputConstants.Key ruthenium$key();
}
