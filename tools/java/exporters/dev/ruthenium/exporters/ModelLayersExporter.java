/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.io.IOException;
import java.lang.reflect.Field;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

import net.minecraft.client.model.geom.LayerDefinitions;
import net.minecraft.client.model.geom.ModelLayerLocation;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.geom.PartPose;
import net.minecraft.client.model.geom.builders.LayerDefinition;

/**
 * The geometry of every model layer (entities and block entities), as the game bakes it
 * from code.
 *
 * <p>Format {@code model_layers} v1, after the header: u32 layer count, then per layer,
 * ordered by name: string {@code namespace:model#layer} and its root part. A part is
 * nine f32, its initial pose (position in sixteenths of a block, rotation in radians
 * about x, y and z, scale); u32 cube count, then per cube u8 face count and per face
 * three f32 normal and four corners of five f32 (position in sixteenths, then u and v
 * from 0 to 1), in the game's order; u32 child count, then per child, ordered by name,
 * its string name and the part.
 *
 * <p>Children are ordered by name because the game keeps them in hash maps. The order
 * parts are drawn in shows only where translucent parts overlap.
 */
final class ModelLayersExporter {
    static final String FILE = "model_layers.bin";

    private ModelLayersExporter() {}

    static void export(Path file) throws IOException, ReflectiveOperationException {
        Field cubes = ModelPart.class.getDeclaredField("cubes");
        Field children = ModelPart.class.getDeclaredField("children");
        cubes.setAccessible(true);
        children.setAccessible(true);

        List<Map.Entry<ModelLayerLocation, LayerDefinition>> layers =
                new ArrayList<>(LayerDefinitions.createRoots().entrySet());
        layers.sort(Comparator.comparing(e -> e.getKey().toString()));
        ExportWriter w = new ExportWriter("model_layers", 1);
        w.u32(layers.size());
        for (Map.Entry<ModelLayerLocation, LayerDefinition> e : layers) {
            w.string(e.getKey().toString());
            part(w, e.getValue().bakeRoot(), cubes, children);
        }
        w.writeTo(file);
    }

    @SuppressWarnings("unchecked")
    private static void part(ExportWriter w, ModelPart part, Field cubes, Field children)
            throws ReflectiveOperationException {
        PartPose p = part.getInitialPose();
        for (float f : new float[] {
                p.x(), p.y(), p.z(), p.xRot(), p.yRot(), p.zRot(), p.xScale(), p.yScale(), p.zScale()}) {
            w.f32(f);
        }
        List<ModelPart.Cube> boxes = (List<ModelPart.Cube>) cubes.get(part);
        w.u32(boxes.size());
        for (ModelPart.Cube cube : boxes) {
            w.u8(cube.polygons.length);
            for (ModelPart.Polygon face : cube.polygons) {
                w.f32(face.normal().x());
                w.f32(face.normal().y());
                w.f32(face.normal().z());
                if (face.vertices().length != 4) {
                    throw new IllegalStateException("a face of " + face.vertices().length + " corners");
                }
                for (ModelPart.Vertex v : face.vertices()) {
                    w.f32(v.x());
                    w.f32(v.y());
                    w.f32(v.z());
                    w.f32(v.u());
                    w.f32(v.v());
                }
            }
        }
        Map<String, ModelPart> named = new TreeMap<>((Map<String, ModelPart>) children.get(part));
        w.u32(named.size());
        for (Map.Entry<String, ModelPart> e : named.entrySet()) {
            w.string(e.getKey());
            part(w, e.getValue(), cubes, children);
        }
    }
}
