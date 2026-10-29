package net.ruthenium.interfaces;

import net.ruthenium.render.text.RetainedLabels;
import org.jspecify.annotations.Nullable;

/** A {@code StagedVertexBuffer.Draw} whose geometry is retained labels ({@link RetainedLabels}) instead of its vertices */
public interface LabelDrawHolder {
    RetainedLabels.@Nullable LabelDraw ruthenium$labelDraw();

    void ruthenium$setLabelDraw(RetainedLabels.@Nullable LabelDraw draw);
}
