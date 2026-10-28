package net.ruthenium.mixin.render.entity;

import net.minecraft.world.entity.Display;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

/** The split text a text display keeps until its text or line width changes ({@code TextDisplay.cacheDisplay}) */
@Mixin(Display.TextDisplay.class)
public interface TextDisplayAccessor {
    @Accessor("clientDisplayCache")
    Display.TextDisplay.@Nullable CachedInfo ruthenium$cachedInfo();
}
