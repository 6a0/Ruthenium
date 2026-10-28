package net.rubidium.interfaces;

import net.rubidium.render.entity.InstancedParts;
import org.jspecify.annotations.Nullable;

/** The meshes of a model part drawn as instances ({@link InstancedParts}), one per render type and UV mapping */
public interface InstancedPartHolder {
    InstancedParts.@Nullable PartBatches rubidium$getBatches();

    void rubidium$setBatches(InstancedParts.PartBatches batches);
}
