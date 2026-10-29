package net.ruthenium.interfaces;

import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.renderer.StagedVertexBuffer;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.ruthenium.render.entity.PlayerPoseDraws;
import org.jspecify.annotations.Nullable;

/** A {@code RenderTypeFeatureRenderer$Group}'s draws, for {@link PlayerPoseDraws} and {@code RetainedLabels} */
public interface PosedDrawGroup {
    /** Whether the group may reorder its draws (it is not strictly ordered) */
    boolean ruthenium$canReorder();

    /** The group's last draw if geometry of this render type would join it ({@code getVertexBuilder}), else null */
    StagedVertexBuffer.@Nullable Draw ruthenium$joiningDraw(RenderType renderType);

    /** Adds the draw {@code getVertexBuilder} would add for this render type, without fetching a vertex builder */
    StagedVertexBuffer.Draw ruthenium$addDraw(RenderType renderType);

    /** The prepared render type of the group's last draw */
    PreparedRenderType ruthenium$lastPreparedType();

    /** The prepared render type of one of the group's draws */
    PreparedRenderType ruthenium$preparedTypeOf(StagedVertexBuffer.Draw draw);

    /** Whether vertices were written into this draw (or its vertex builder is open) */
    boolean ruthenium$hasVertices(StagedVertexBuffer.Draw draw);

    /** {@code getVertexBuilder}: the vertex builder geometry of this render type is written into */
    VertexConsumer ruthenium$vertexBuilder(RenderType renderType);
}
