package net.vulkanmod.render.entity;

import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.renderpearl.api.pipeline.BlendFunction;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.DepthStencilState;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.renderpearl.api.pipeline.ShaderType;
import com.mojang.renderpearl.api.textures.GpuTexture;
import net.minecraft.client.Minecraft;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.renderer.ShaderDefines;
import net.minecraft.client.renderer.rendertype.RenderSetup;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.rendertype.TextureTransform;
import net.minecraft.client.renderer.texture.AbstractTexture;
import net.minecraft.client.renderer.texture.SimpleTexture;
import net.minecraft.client.renderer.texture.TextureContents;
import net.minecraft.resources.Identifier;
import net.vulkanmod.Initializer;
import org.jspecify.annotations.Nullable;

import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * Model cube faces that draw nothing: every texel their UVs can sample is discarded by the pipeline's alpha cutout
 * or, in a blending pipeline without depth writes, has alpha 0 and blends to exactly what is already there. Spider
 * and enderman eyes draw the whole model with a texture that is empty except for the eyes; horse markings leave
 * faces empty too. Those faces are not written ({@code ModelPartCubeM}).
 * <p>
 * Only for render types of vanilla's {@code core/entity} shaders without a texture matrix, glint, dissolve or improved
 * transparency (OIT), whose {@code Sampler0} is a plain resource texture ({@link SimpleTexture}): its alpha is read
 * from the resource once. The UV rectangle of a face is widened by one texel on each side, and both repeated and
 * clamped addressing are tested, so nearest and linear filtering are covered. Render thread only; cleared on
 * resource reload.
 */
public final class TransparentFaces {
    private static final Identifier ENTITY_SHADER = Identifier.withDefaultNamespace("core/entity");
    private static final Object NONE = new Object();

    /** {@link TextureAlpha} or {@link #NONE} per render type */
    private static final Map<RenderType, Object> BY_RENDER_TYPE = new IdentityHashMap<>();
    private static @Nullable RenderType lastRenderType;
    private static @Nullable TextureAlpha lastAlpha;

    private TransparentFaces() {
    }

    /** A texture's alpha and the 8 bit alpha below which a texel draws nothing in a pipeline */
    public static final class TextureAlpha {
        final byte[] alpha;
        final int width;
        final int height;
        final int threshold;
        /** The texture it was read for; a reload replaces the texture's GPU texture */
        final Identifier location;
        final AbstractTexture texture;
        final GpuTexture gpuTexture;

        TextureAlpha(byte[] alpha, int width, int height, int threshold, Identifier location, AbstractTexture texture, GpuTexture gpuTexture) {
            this.alpha = alpha;
            this.width = width;
            this.height = height;
            this.threshold = threshold;
            this.location = location;
            this.texture = texture;
            this.gpuTexture = gpuTexture;
        }

        boolean isCurrent() {
            return this.texture.getTexture() == this.gpuTexture && Minecraft.getInstance().getTextureManager().getTexture(this.location) == this.texture;
        }

        /** Bit {@code p} is set if polygon {@code p} of the cube draws nothing */
        public int transparentPolygons(ModelPart.Polygon[] polygons) {
            int mask = 0;
            for (int p = 0; p < polygons.length; p++) {
                ModelPart.Vertex[] vertices = polygons[p].vertices();
                float u0 = Float.POSITIVE_INFINITY, u1 = Float.NEGATIVE_INFINITY, v0 = Float.POSITIVE_INFINITY, v1 = Float.NEGATIVE_INFINITY;
                for (ModelPart.Vertex vertex : vertices) {
                    u0 = Math.min(u0, vertex.u());
                    u1 = Math.max(u1, vertex.u());
                    v0 = Math.min(v0, vertex.v());
                    v1 = Math.max(v1, vertex.v());
                }
                if (this.isTransparent(u0, u1, v0, v1)) {
                    mask |= 1 << p;
                }
            }
            return mask;
        }

        private boolean isTransparent(float u0, float u1, float v0, float v1) {
            if (!Float.isFinite(u0) || !Float.isFinite(u1) || !Float.isFinite(v0) || !Float.isFinite(v1)) {
                return false;
            }
            double x0 = Math.floor(u0 * (double) this.width) - 1, x1 = Math.floor(u1 * (double) this.width) + 1;
            double y0 = Math.floor(v0 * (double) this.height) - 1, y1 = Math.floor(v1 * (double) this.height) + 1;
            // A face spanning the texture more than once (repeating UVs) is not worth testing
            if (x1 - x0 > this.width + 2 || y1 - y0 > this.height + 2) {
                return false;
            }
            for (int y = (int) y0; y <= (int) y1; y++) {
                for (int x = (int) x0; x <= (int) x1; x++) {
                    if (!this.isTransparent(Math.floorMod(x, this.width), Math.floorMod(y, this.height))
                            || !this.isTransparent(Math.clamp(x, 0, this.width - 1), Math.clamp(y, 0, this.height - 1))) {
                        return false;
                    }
                }
            }
            return true;
        }

        private boolean isTransparent(int x, int y) {
            return (this.alpha[y * this.width + x] & 0xFF) < this.threshold;
        }
    }

    /** Drops everything (resource reload, switch toggled) */
    public static void clear() {
        BY_RENDER_TYPE.clear();
        lastRenderType = null;
        lastAlpha = null;
    }

    /** The texture alpha of a render type whose transparent faces may be left out, or null */
    public static @Nullable TextureAlpha of(RenderType renderType) {
        if (renderType == lastRenderType) {
            return lastAlpha;
        }
        TextureAlpha alpha = lookUp(renderType);
        lastRenderType = renderType;
        lastAlpha = alpha;
        return alpha;
    }

    private static @Nullable TextureAlpha lookUp(RenderType renderType) {
        Object value = BY_RENDER_TYPE.get(renderType);
        if (value instanceof TextureAlpha alpha && !alpha.isCurrent()) {
            value = null;
        }
        if (value == null) {
            TextureAlpha alpha = null;
            try {
                alpha = create(renderType);
            } catch (RuntimeException e) {
                Initializer.LOGGER.warn("Can't read the texture alpha of {}", renderType, e);
            }
            value = alpha != null ? alpha : NONE;
            BY_RENDER_TYPE.put(renderType, value);
        }
        return value == NONE ? null : (TextureAlpha) value;
    }

    private static @Nullable TextureAlpha create(RenderType renderType) {
        RenderPipeline pipeline = renderType.pipeline();
        Map<ShaderType, Identifier> shaders = pipeline.getShaders();
        if (!ENTITY_SHADER.equals(shaders.get(ShaderType.VERTEX)) || !ENTITY_SHADER.equals(shaders.get(ShaderType.FRAGMENT))) {
            return null;
        }
        ShaderDefines defines = pipeline.getShaderDefines();
        if (defines.flags().contains("APPLY_TEXTURE_MATRIX") || defines.flags().contains("GLINT") || defines.flags().contains("DISSOLVE")
                || defines.values().containsKey("APPLY_TEXTURE_MATRIX") || defines.values().containsKey("GLINT")
                || defines.values().containsKey("DISSOLVE")) {
            return null;
        }
        RenderSetup setup = renderType.state;
        if (setup.textureTransform != TextureTransform.DEFAULT_TEXTURING) {
            return null;
        }

        int threshold;
        String cutout = defines.values().get("ALPHA_CUTOUT");
        if (cutout != null) {
            // The shader discards samples with alpha < cutout (before any color is applied); 8 bit alpha a samples as
            // a / 255, kept clear of the boundary
            float value;
            try {
                value = Float.parseFloat(cutout);
            } catch (NumberFormatException e) {
                return null;
            }
            threshold = (int) Math.ceil(value * 255.0 - 0.01);
            if (threshold <= 0) {
                return null;
            }
            threshold = Math.min(threshold, 256);
        } else {
            // Alpha 0 samples give alpha 0 fragments; with translucent blending and no depth writes they change nothing
            DepthStencilState depth = pipeline.getDepthStencilState();
            if (depth == null || depth.writeDepth()) {
                return null;
            }
            List<@Nullable ColorTargetState> targets = pipeline.getColorTargetStates();
            if (targets.isEmpty()) {
                return null;
            }
            for (ColorTargetState target : targets) {
                if (target == null || !target.blendFunction().equals(Optional.of(BlendFunction.TRANSLUCENT))) {
                    return null;
                }
            }
            threshold = 1;
        }

        RenderSetup.TextureBinding binding = setup.textures.get("Sampler0");
        if (binding == null || setup.textures.size() != 1) {
            return null;
        }
        return load(binding.location(), threshold);
    }

    private static @Nullable TextureAlpha load(Identifier location, int threshold) {
        Minecraft minecraft = Minecraft.getInstance();
        AbstractTexture texture = minecraft.getTextureManager().getTexture(location);
        if (texture.getClass() != SimpleTexture.class) {
            return null;
        }
        try (TextureContents contents = TextureContents.load(minecraft.getResourceManager(), location)) {
            NativeImage image = contents.image();
            int width = image.getWidth(), height = image.getHeight();
            if (width != texture.getTexture().getWidth(0) || height != texture.getTexture().getHeight(0)
                    || texture.getTexture().getMipLevels() != 1) {
                return null;
            }
            byte[] alpha = new byte[width * height];
            for (int y = 0; y < height; y++) {
                for (int x = 0; x < width; x++) {
                    alpha[y * width + x] = (byte) (image.getPixel(x, y) >>> 24);
                }
            }
            return new TextureAlpha(alpha, width, height, threshold, location, texture, texture.getTexture());
        } catch (Exception e) {
            return null;
        }
    }
}
