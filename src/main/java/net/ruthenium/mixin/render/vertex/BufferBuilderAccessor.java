package net.ruthenium.mixin.render.vertex;

import com.mojang.blaze3d.vertex.BufferBuilder;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

@Mixin(BufferBuilder.class)
public interface BufferBuilderAccessor {

    /** @return the vertices begun so far */
    @Accessor("vertices")
    int ruthenium$getVertices();
}
