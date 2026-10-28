package net.rubidium.mixin.world;

import net.minecraft.world.level.chunk.LevelChunkSection;
import net.rubidium.interfaces.SectionChangeStamp;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(LevelChunkSection.class)
public class LevelChunkSectionM implements SectionChangeStamp {
    @Unique private int changes;

    @Override
    public int rubidium$changes() {
        return this.changes;
    }

    @Override
    public void rubidium$changed() {
        this.changes++;
    }
}
