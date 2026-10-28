package net.thallium.mixin.world;

import net.minecraft.world.level.chunk.LevelChunkSection;
import net.thallium.interfaces.SectionChangeStamp;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(LevelChunkSection.class)
public class LevelChunkSectionM implements SectionChangeStamp {
    @Unique private int changes;

    @Override
    public int thallium$changes() {
        return this.changes;
    }

    @Override
    public void thallium$changed() {
        this.changes++;
    }
}
