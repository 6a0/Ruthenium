package net.ruthenium.gametest.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

@Mixin(targets = "net.ruthenium.render.chunk.TerrainDrawList$Group", remap = false)
public interface TerrainDrawListGroupAccessor {
    @Accessor("commands")
    int[] getCommands();

    @Accessor("count")
    int getCount();

    @Accessor("count")
    void setCount(int count);
}
