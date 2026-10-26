package net.vulkanmod.mixin.render.text;

import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.gui.font.glyphs.BakedSheetGlyph;
import net.minecraft.util.ARGB;
import net.vulkanmod.interfaces.ExtendedVertexBuilder;
import org.joml.Matrix4fc;
import org.joml.Vector3f;
import org.lwjgl.system.MemoryUtil;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

/**
 * Writes glyph quads directly into world text (POSITION_TEX_LIGHTMAP_COLOR) and GUI text (POSITION_TEX_COLOR)
 * buffers instead of going through the per-element VertexConsumer calls. Output is identical to vanilla.
 */
@Mixin(BakedSheetGlyph.class)
public abstract class BakedSheetGlyphM {
    @Shadow @Final private float u0;
    @Shadow @Final private float u1;
    @Shadow @Final private float v0;
    @Shadow @Final private float v1;
    @Shadow @Final private float left;
    @Shadow @Final private float right;
    @Shadow @Final private float up;
    @Shadow @Final private float down;

    @Shadow protected abstract float shearTop();
    @Shadow protected abstract float shearBottom();

    @Unique private final Vector3f scratch = new Vector3f();

    /**
     * @author VulkanMod
     * @reason Fast path for text buffers; an overwrite rather than an inject so that no CallbackInfo is allocated
     * per glyph
     */
    @Overwrite
    private void render(boolean italic, float x, float y, float z, Matrix4fc pose, VertexConsumer builder,
                        int color, boolean bold, int packedLightCoords) {
        boolean withLight;
        ExtendedVertexBuilder vertexBuilder = ExtendedVertexBuilder.of(builder, DefaultVertexFormat.POSITION_TEX_LIGHTMAP_COLOR);
        if (vertexBuilder != null) {
            withLight = true;
        } else {
            vertexBuilder = ExtendedVertexBuilder.of(builder, DefaultVertexFormat.POSITION_TEX_COLOR);
            withLight = false;
        }

        float x0 = x + this.left;
        float x1 = x + this.right;
        float y0 = y + this.up;
        float y1 = y + this.down;
        float shearY0 = italic ? this.shearTop() : 0.0F;
        float shearY1 = italic ? this.shearBottom() : 0.0F;
        float extraThickness = bold ? 0.1F : 0.0F;

        if (vertexBuilder == null) {
            // Vanilla
            builder.addVertex(pose, x0 + shearY0 - extraThickness, y0 - extraThickness, z).setColor(color).setUv(this.u0, this.v0).setLight(packedLightCoords);
            builder.addVertex(pose, x0 + shearY1 - extraThickness, y1 + extraThickness, z).setColor(color).setUv(this.u0, this.v1).setLight(packedLightCoords);
            builder.addVertex(pose, x1 + shearY1 + extraThickness, y1 + extraThickness, z).setColor(color).setUv(this.u1, this.v1).setLight(packedLightCoords);
            builder.addVertex(pose, x1 + shearY0 + extraThickness, y0 - extraThickness, z).setColor(color).setUv(this.u1, this.v0).setLight(packedLightCoords);
            return;
        }

        int abgr = ARGB.toABGR(color);

        int vertexSize = withLight ? 28 : 24;
        long ptr = vertexBuilder.reserveVertices(4);
        this.putVertex(ptr, pose, x0 + shearY0 - extraThickness, y0 - extraThickness, z, this.u0, this.v0, abgr, packedLightCoords, withLight);
        this.putVertex(ptr + vertexSize, pose, x0 + shearY1 - extraThickness, y1 + extraThickness, z, this.u0, this.v1, abgr, packedLightCoords, withLight);
        this.putVertex(ptr + vertexSize * 2L, pose, x1 + shearY1 + extraThickness, y1 + extraThickness, z, this.u1, this.v1, abgr, packedLightCoords, withLight);
        this.putVertex(ptr + vertexSize * 3L, pose, x1 + shearY0 + extraThickness, y0 - extraThickness, z, this.u1, this.v0, abgr, packedLightCoords, withLight);
    }

    /**
     * POSITION_TEX_LIGHTMAP_COLOR: Position (3 floats), UV0 (2 floats), UV2 (2 shorts), Color (RGBA8)
     * POSITION_TEX_COLOR: Position (3 floats), UV0 (2 floats), Color (RGBA8)
     */
    @Unique
    private void putVertex(long ptr, Matrix4fc pose, float x, float y, float z, float u, float v, int abgr, int light, boolean withLight) {
        Vector3f pos = pose.transformPosition(x, y, z, this.scratch);
        MemoryUtil.memPutFloat(ptr, pos.x());
        MemoryUtil.memPutFloat(ptr + 4, pos.y());
        MemoryUtil.memPutFloat(ptr + 8, pos.z());
        MemoryUtil.memPutFloat(ptr + 12, u);
        MemoryUtil.memPutFloat(ptr + 16, v);
        if (withLight) {
            MemoryUtil.memPutInt(ptr + 20, light);
            MemoryUtil.memPutInt(ptr + 24, abgr);
        } else {
            MemoryUtil.memPutInt(ptr + 20, abgr);
        }
    }
}
