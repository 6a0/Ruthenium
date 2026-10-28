package net.thallium.interfaces;

import net.thallium.render.entity.InstancedParts;
import org.jspecify.annotations.Nullable;

/** The meshes of a model part drawn as instances ({@link InstancedParts}), one per render type and UV mapping */
public interface InstancedPartHolder {
    InstancedParts.@Nullable PartBatches thallium$getBatches();

    void thallium$setBatches(InstancedParts.PartBatches batches);
}
