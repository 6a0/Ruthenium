/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.nio.file.Path;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockState;

/**
 * The sounds each block state makes when broken, stepped on, placed, hit and fallen on
 * ({@code BlockState.getSoundType}), with the type's volume and pitch.
 *
 * <p>Format {@code block_sounds} v1, after the header: u32 sound type count, then per
 * type: f32 volume, f32 pitch, and the break, step, place, hit and fall sound events as
 * strings; then u32 state count and per state in id order a u32 index into the types.
 */
final class BlockSoundExporter {
    static final String FILE = "block_sounds.bin";

    private BlockSoundExporter() {}

    static void export(Path file) throws Exception {
        Map<SoundType, Integer> ids = new IdentityHashMap<>();
        List<SoundType> types = new ArrayList<>();
        ExportWriter states = new ExportWriter(null, 0);
        int count = 0;
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            if (Block.BLOCK_STATE_REGISTRY.getId(state) != count) {
                throw new IllegalStateException("block states out of order at " + count);
            }
            count++;
            SoundType type = state.getSoundType();
            Integer id = ids.get(type);
            if (id == null) {
                id = types.size();
                ids.put(type, id);
                types.add(type);
            }
            states.u32(id);
        }
        ExportWriter w = new ExportWriter("block_sounds", 1);
        w.u32(types.size());
        for (SoundType t : types) {
            w.f32(t.getVolume());
            w.f32(t.getPitch());
            w.string(t.getBreakSound().location().toString());
            w.string(t.getStepSound().location().toString());
            w.string(t.getPlaceSound().location().toString());
            w.string(t.getHitSound().location().toString());
            w.string(t.getFallSound().location().toString());
        }
        w.u32(count);
        w.bytes(states.payload());
        w.writeTo(file);
    }
}
