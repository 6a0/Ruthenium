package net.rubidium.gametest.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

@Mixin(targets = "net.rubidium.render.chunk.TerrainDrawList$Group", remap = false)
public interface TerrainDrawListGroupAccessor {
    @Accessor("commands")
    int[] getCommands();

    @Accessor("count")
    int getCount();
}
