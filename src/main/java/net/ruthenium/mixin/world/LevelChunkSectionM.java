package net.ruthenium.mixin.world;

import net.minecraft.world.level.chunk.LevelChunkSection;
import net.ruthenium.interfaces.SectionChangeStamp;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(LevelChunkSection.class)
public class LevelChunkSectionM implements SectionChangeStamp {
    @Unique private int changes;

    @Override
    public int ruthenium$changes() {
        return this.changes;
    }

    @Override
    public void ruthenium$changed() {
        this.changes++;
    }
}
