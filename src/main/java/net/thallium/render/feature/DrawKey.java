package net.thallium.render.feature;

import com.mojang.blaze3d.systems.ScissorState;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import net.minecraft.client.renderer.oit.OitPipelineSet;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import org.jspecify.annotations.Nullable;

import java.util.List;

/**
 * A {@link PreparedRenderType} with its scissor state compared by value ({@code ScissorState} has no {@code equals});
 * the rectangle only counts when the scissor is enabled, as drawing ignores it otherwise.
 */
public record DrawKey(String name, RenderPipeline pipeline, @Nullable OitPipelineSet oitPipelineSet,
                      GpuBufferSlice dynamicTransforms, boolean scissor, int x, int y, int width, int height,
                      List<PreparedRenderType.Texture> textures) {

    public static DrawKey of(PreparedRenderType type) {
        ScissorState scissor = type.scissorState();
        boolean enabled = scissor.enabled();
        return new DrawKey(type.name(), type.pipeline(), type.oitPipelineSet(), type.dynamicTransforms(), enabled,
                enabled ? scissor.x() : 0, enabled ? scissor.y() : 0,
                enabled ? scissor.width() : 0, enabled ? scissor.height() : 0, type.textures());
    }
}
