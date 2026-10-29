package net.ruthenium.mixin.render.vertex;

import com.mojang.blaze3d.vertex.VertexSorting;
import net.minecraft.client.renderer.StagedVertexBuffer;
import net.ruthenium.interfaces.LabelDrawHolder;
import net.ruthenium.interfaces.PosedDrawHolder;
import net.ruthenium.render.entity.PlayerPoseDraws;
import net.ruthenium.render.text.RetainedLabels;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

/** The posed draw ({@link PlayerPoseDraws}) or label draw ({@link RetainedLabels}) drawn in a staged draw's place */
@Mixin(StagedVertexBuffer.Draw.class)
public class StagedVertexBufferDrawM implements PosedDrawHolder, LabelDrawHolder {
    @Shadow @Final private @Nullable VertexSorting quadSorting;

    @Unique private PlayerPoseDraws.@Nullable PoseDraw poseDraw;
    @Unique private RetainedLabels.@Nullable LabelDraw labelDraw;

    @Override
    public RetainedLabels.@Nullable LabelDraw ruthenium$labelDraw() {
        return this.labelDraw;
    }

    @Override
    public void ruthenium$setLabelDraw(RetainedLabels.@Nullable LabelDraw draw) {
        this.labelDraw = draw;
    }

    @Override
    public PlayerPoseDraws.@Nullable PoseDraw ruthenium$poseDraw() {
        return this.poseDraw;
    }

    @Override
    public void ruthenium$setPoseDraw(PlayerPoseDraws.@Nullable PoseDraw draw) {
        this.poseDraw = draw;
    }

    @Override
    public @Nullable VertexSorting ruthenium$quadSorting() {
        return this.quadSorting;
    }
}
