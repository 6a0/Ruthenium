package net.ruthenium.mixin.render.vertex;

import net.minecraft.client.renderer.StagedVertexBuffer;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

/** The draw whose vertex builder is open ({@code StagedVertexBuffer.getVertexBuilder}) */
@Mixin(StagedVertexBuffer.class)
public interface StagedVertexBufferAccessor {
    @Accessor("lastBuildingDraw")
    StagedVertexBuffer.@Nullable Draw ruthenium$lastBuildingDraw();
}
