package net.vulkanmod.interfaces;

import net.vulkanmod.render.entity.InstancedParts;
import org.jspecify.annotations.Nullable;

/** The meshes of a model part drawn as instances ({@link InstancedParts}), one per render type and UV mapping */
public interface InstancedPartHolder {
    InstancedParts.@Nullable PartBatch vulkanmod$getBatch();

    void vulkanmod$setBatch(InstancedParts.PartBatch batch);
}
