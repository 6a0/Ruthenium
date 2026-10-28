package net.vulkanmod.mixin.world;

import net.minecraft.world.level.chunk.LevelChunkSection;
import net.vulkanmod.interfaces.SectionChangeStamp;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(LevelChunkSection.class)
public class LevelChunkSectionM implements SectionChangeStamp {
    @Unique private int changes;

    @Override
    public int vulkanmod$changes() {
        return this.changes;
    }

    @Override
    public void vulkanmod$changed() {
        this.changes++;
    }
}
