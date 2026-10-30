package net.ruthenium.render.entity;

import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.texture.UvMapping;
import org.joml.Matrix4fc;
import org.jspecify.annotations.Nullable;

import java.util.Objects;
import java.util.function.Function;

/**
 * The vertex builder of a model submit whose parts become instances ({@link InstancedParts}): the feature renderer
 * group's builder for the submit's render type is only fetched (a render type switch: {@code RenderType.prepare}, a
 * draw lookup and a new {@code BufferBuilder}) if a part has to be written after all. Reused by its renderer for each
 * submit (render thread).
 */
public final class LazyVertexBuilder implements VertexConsumer {
    private final Function<RenderType, VertexConsumer> builders;
    private @Nullable RenderType renderType;
    private @Nullable UvMapping uvMapping;
    private @Nullable VertexConsumer delegate;

    public LazyVertexBuilder(Function<RenderType, VertexConsumer> builders) {
        this.builders = builders;
    }

    public LazyVertexBuilder reset(RenderType renderType, @Nullable UvMapping uvMapping) {
        this.renderType = renderType;
        this.uvMapping = uvMapping;
        this.delegate = null;
        return this;
    }

    /** The submit's builder (with its UV mapping), fetched on first use */
    public VertexConsumer resolve() {
        VertexConsumer delegate = this.delegate;
        if (delegate == null) {
            delegate = this.builders.apply(Objects.requireNonNull(this.renderType));
            if (this.uvMapping != null) {
                delegate = this.uvMapping.wrap(delegate);
            }
            this.delegate = delegate;
        }
        return delegate;
    }

    @Override
    public VertexConsumer addVertex(float x, float y, float z) {
        return this.resolve().addVertex(x, y, z);
    }

    /** The builder's own (a {@code BufferBuilder} transforms without a new vector), as the default method would end there */
    @Override
    public VertexConsumer addVertex(Matrix4fc pose, float x, float y, float z) {
        return this.resolve().addVertex(pose, x, y, z);
    }

    @Override
    public VertexConsumer setNormal(PoseStack.Pose pose, float x, float y, float z) {
        return this.resolve().setNormal(pose, x, y, z);
    }

    @Override
    public VertexConsumer setColor(int r, int g, int b, int a) {
        return this.resolve().setColor(r, g, b, a);
    }

    @Override
    public VertexConsumer setColor(int color) {
        return this.resolve().setColor(color);
    }

    @Override
    public VertexConsumer setUv(float u, float v) {
        return this.resolve().setUv(u, v);
    }

    @Override
    public VertexConsumer setUv1(int u, int v) {
        return this.resolve().setUv1(u, v);
    }

    @Override
    public VertexConsumer setUv2(int u, int v) {
        return this.resolve().setUv2(u, v);
    }

    @Override
    public VertexConsumer setUv3(float u, float v) {
        return this.resolve().setUv3(u, v);
    }

    @Override
    public VertexConsumer setNormal(float x, float y, float z) {
        return this.resolve().setNormal(x, y, z);
    }

    @Override
    public VertexConsumer setLineWidth(float width) {
        return this.resolve().setLineWidth(width);
    }

    @Override
    public void addVertex(float x, float y, float z, int color, float u, float v, int overlayCoords, int lightCoords, float nx, float ny, float nz) {
        this.resolve().addVertex(x, y, z, color, u, v, overlayCoords, lightCoords, nx, ny, nz);
    }
}
