package net.ruthenium.interfaces;

import net.ruthenium.render.entity.TransparentFaces;

/** A model cube's polygons that draw nothing in a texture ({@link TransparentFaces}), as {@code ModelPartCubeM} skips them */
public interface TransparentCube {
    int ruthenium$transparentPolygons(TransparentFaces.TextureAlpha textureAlpha);
}
