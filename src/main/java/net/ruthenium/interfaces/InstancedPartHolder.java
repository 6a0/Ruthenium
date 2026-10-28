package net.ruthenium.interfaces;

import net.ruthenium.render.entity.InstancedParts;
import org.jspecify.annotations.Nullable;

/** The meshes of a model part drawn as instances ({@link InstancedParts}), one per render type and UV mapping */
public interface InstancedPartHolder {
    InstancedParts.@Nullable PartBatches ruthenium$getBatches();

    void ruthenium$setBatches(InstancedParts.PartBatches batches);
}
