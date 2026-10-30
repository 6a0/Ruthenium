package net.ruthenium.gametest;

import com.mojang.blaze3d.pipeline.RenderTarget;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.GpuFormat;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.textures.GpuTexture;
import it.unimi.dsi.fastutil.longs.Long2ObjectOpenHashMap;
import it.unimi.dsi.fastutil.longs.LongOpenHashSet;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.minecraft.client.Minecraft;
import net.ruthenium.interfaces.PackedSectionDraws;
import net.minecraft.client.renderer.DynamicGpuData;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.phys.Vec3;
import net.ruthenium.gametest.mixin.TerrainDrawListGroupAccessor;
import org.joml.Matrix4f;
import org.joml.Matrix4fc;
import org.joml.Vector4f;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.nio.ByteOrder;
import java.nio.FloatBuffer;
import java.util.List;

/**
 * Diagnostic ({@code -Pbench.occlusionProbe=true}, terrain scene): how much of the drawn terrain a GPU occlusion pass
 * like Nvidium's could leave out, at the static view and the screenshot yaws ({@link #survey}, counts only).
 * <ol>
 *     <li>One frame is drawn without the translucent pass (translucent terrain, clouds, weather), and the main
 *     target's depth is read back after the opaque pass: the occluders are everything opaque the frame draws
 *     (terrain and solid features).</li>
 *     <li>The same frame's terrain draw commands give each section's drawn quads per layer (after face groups).</li>
 *     <li>A section is hidden if, at every pixel centre its box could reach, the depth buffer holds something nearer
 *     than the box's nearest corner. Two boxes: the section cube and the bounds of its non-air blocks, both grown by a
 *     block (models may reach past their block). Sections whose box reaches no pixel centre are counted as off screen
 *     (vanilla's cull frustum is wider than the screen).</li>
 *     <li>A proxy for sub-pixel triangle culling: the quads of sections where a whole block face at the box's nearest
 *     point is smaller than 1 or 2 pixels.</li>
 *     <li>The upper bound of the gain: the static view measured as is, then with the hidden sections left out of
 *     the draw commands (only valid because the camera doesn't move), then also without the off screen ones. A
 *     screenshot of the culled view checks that nothing visible was left out.</li>
 * </ol>
 * Depth is 0..1 on both backends (Vulkan natively, OpenGL through {@code glClipControl}; the projection is reversed,
 * near is 1), fitted as {@code a + b / w} from the frame's projection, and the read back rows run bottom to top on
 * both, as vanilla's screenshots assume. Render thread only, except {@link #run} and {@link #survey}.
 */
public final class OcclusionProbe {
    public static final boolean ENABLED = Boolean.parseBoolean(BenchmarkClientGameTest.prop("occlusionProbe", "false"));
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Occlusion Probe");

    private static final ChunkSectionLayer[] LAYERS = ChunkSectionLayer.values();
    /** Corners nearer than this (clip w) make a section untestable, so it counts as visible */
    private static final double NEAR_W = 0.1;
    /** In pixels, around the projected box */
    private static final double BOUNDS_MARGIN = 0.5;
    /** Relative, in 1/w: the depth buffer must be nearer than the box by this much */
    private static final double DEPTH_MARGIN = 1.0e-4;
    /** Blocks around the boxes */
    private static final double BOX_MARGIN = 1.0;

    private enum State { IDLE, ARMED, CAPTURING, READING, DONE, FAILED }

    private static State state = State.IDLE;
    private static final Matrix4f latestProjection = new Matrix4f();
    private static final Matrix4f projection = new Matrix4f();
    private static final Matrix4f modelView = new Matrix4f();
    private static Vec3 eye = Vec3.ZERO;
    /** Section origin (BlockPos.asLong) to the quads drawn per layer in the captured frame */
    private static final Long2ObjectOpenHashMap<int[]> drawnQuads = new Long2ObjectOpenHashMap<>();
    private static float[] depth = new float[0];
    private static int width, height;
    private static String failure = "";

    /** Sections left out of the draw commands while measuring the culled views */
    private static final LongOpenHashSet culled = new LongOpenHashSet();
    private static boolean culling;

    private OcclusionProbe() {
    }

    // Hooks

    /** The level projection of the frame, as {@code GameRenderer.renderLevel} sets it */
    public static void setProjection(Matrix4fc matrix) {
        latestProjection.set(matrix);
    }

    /** {@code TerrainDrawList.prepare}: starts capturing the armed frame */
    public static void beginFrame(Vec3 cameraEye, Matrix4fc terrainModelView) {
        if (state == State.ARMED) {
            state = State.CAPTURING;
            eye = cameraEye;
            modelView.set(terrainModelView);
            projection.set(latestProjection);
            drawnQuads.clear();
        }
    }

    /** True while the captured frame is drawn: its translucent pass is left out */
    public static boolean isCapturing() {
        return state == State.CAPTURING;
    }

    /** {@code TerrainDrawList.writeCommands}: records the captured frame's commands, and leaves out culled sections */
    public static void onCommands(ChunkSectionLayer layer, List<?> groups, List<DynamicGpuData.ChunkSectionInfo> sectionInfos) {
        boolean capturing = state == State.CAPTURING;
        if (!capturing && !culling) {
            return;
        }
        for (Object group : groups) {
            TerrainDrawListGroupAccessor accessor = (TerrainDrawListGroupAccessor) group;
            int[] c = accessor.getCommands();
            int n = accessor.getCount();
            int kept = 0;
            for (int i = 0; i < n; i++) {
                int o = i * 5;
                DynamicGpuData.ChunkSectionInfo info = sectionInfos.get(c[o + 4]);
                long key = BlockPos.asLong(info.x(), info.y(), info.z());
                if (capturing) {
                    drawnQuads.computeIfAbsent(key, _ -> new int[LAYERS.length])[layer.ordinal()] += c[o] / 6;
                }
                if (culling && culled.contains(key)) {
                    continue;
                }
                if (kept != i) {
                    System.arraycopy(c, o, c, kept * 5, 5);
                }
                kept++;
            }
            if (kept != n) {
                accessor.setCount(kept);
            }
        }
    }

    /** {@code LevelRenderer.executeOutline}: the opaque pass has ended, so its depth is complete */
    public static void afterOpaquePass() {
        if (state != State.CAPTURING) {
            return;
        }
        RenderTarget target = Minecraft.getInstance().gameRenderer.mainRenderTarget();
        GpuTexture texture = target.getDepthTexture();
        if (texture == null || texture.getFormat() != GpuFormat.D32_FLOAT) {
            failure = "depth format " + (texture == null ? "none" : texture.getFormat());
            state = State.FAILED;
            return;
        }
        int w = target.width, h = target.height;
        GpuBuffer buffer = RenderSystem.getDevice().createBuffer(() -> "Occlusion probe depth",
                GpuBuffer.USAGE_MAP_READ | GpuBuffer.USAGE_COPY_DST, (long) w * h * 4);
        state = State.READING;
        RenderSystem.getDevice().createCommandEncoder().copyTextureToBuffer(texture, buffer, 0L, () -> {
            try (GpuBufferSlice.MappedView read = buffer.map(true, false)) {
                FloatBuffer floats = read.data().order(ByteOrder.nativeOrder()).asFloatBuffer();
                depth = new float[w * h];
                floats.get(0, depth);
                width = w;
                height = h;
                state = State.DONE;
            } catch (RuntimeException e) {
                failure = e.toString();
                state = State.FAILED;
            }
            buffer.close();
        }, 0);
    }

    // The probe

    /** Runs at the static view, after it was measured; adds the culled views' phases to the results */
    static void run(ClientGameTestContext context, String label, String runId, List<PhaseResult> results) {
        LongOpenHashSet hidden = new LongOpenHashSet();
        LongOpenHashSet offScreen = new LongOpenHashSet();
        if (!capture(context, "static", hidden, offScreen)) {
            return;
        }
        measureCulled(context, results, "static-ref", null);
        measureCulled(context, results, "static-occluded", hidden);
        LongOpenHashSet all = new LongOpenHashSet(hidden);
        all.addAll(offScreen);
        measureCulled(context, results, "static-culled", all);
        context.takeScreenshot(label + "-terrain-culled-" + runId);
        context.runOnClient(_ -> culling = false);
    }

    /** Logs what a view would hide, without measuring it */
    static void survey(ClientGameTestContext context, String view) {
        capture(context, view, new LongOpenHashSet(), new LongOpenHashSet());
    }

    /** Captures the next frame and sorts its drawn sections; false if the depth couldn't be read */
    private static boolean capture(ClientGameTestContext context, String view, LongOpenHashSet hidden, LongOpenHashSet offScreen) {
        context.runOnClient(_ -> {
            failure = "";
            state = State.ARMED;
        });
        context.waitFor(_ -> state == State.DONE || state == State.FAILED, 200);
        if (context.computeOnClient(_ -> state) == State.FAILED) {
            LOGGER.error("Occlusion probe failed: {}", failure);
            context.runOnClient(_ -> state = State.IDLE);
            return false;
        }
        context.runOnClient(mc -> analyze(mc, view, hidden, offScreen));
        context.runOnClient(_ -> state = State.IDLE);
        return true;
    }

    /** Measures the static view with these sections left out (none: all drawn); the culling stays as set */
    private static void measureCulled(ClientGameTestContext context, List<PhaseResult> results, String phase,
                                      @Nullable LongOpenHashSet sections) {
        context.runOnClient(_ -> {
            culled.clear();
            if (sections != null) {
                culled.addAll(sections);
            }
            culling = sections != null;
        });
        context.waitTicks(5);
        context.runOnClient(_ -> {
            FrameRecorder.startPhase();
            GpuProbe.reset();
        });
        BenchmarkClientGameTest.waitSeconds(context, BenchmarkClientGameTest.STATIC_SECONDS);
        results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase(phase, -1)));
        if (GpuProbe.ENABLED) {
            context.runOnClient(_ -> GpuProbe.report(phase));
        }
        LOGGER.info(results.getLast().summaryLine());
    }

    private static void analyze(Minecraft mc, String view, LongOpenHashSet hidden, LongOpenHashSet offScreen) {
        Matrix4f clip = new Matrix4f(projection).mul(modelView);
        // Depth is affine in 1/w (d = a + b / w): fitted from the projection, checked at a third distance
        double[] fit = fitDepth(projection);
        double a = fit[0], b = fit[1];
        double focal = 0.5 * height * Math.abs(projection.m11());

        // Beyond twice the render distance: the far plane (sky)
        double farInv = 1.0 / (2.0 * 16 * mc.options.renderDistance().get());
        int sky = 0;
        for (float d : depth) {
            if ((d - a) / b < farInv) {
                sky++;
            }
        }
        LOGGER.info("Occlusion probe {}: depth {}x{}, {}% sky, depth fit d = {} + {} / w (error {}), focal {} px",
                view, width, height, "%.1f".formatted(100.0 * sky / depth.length), a, b, "%.2e".formatted(fit[2]),
                "%.1f".formatted(focal));

        long[] all = new long[LAYERS.length];
        long[] cubeHidden = new long[LAYERS.length];
        long[] blocksHidden = new long[LAYERS.length];
        long[] offQuads = new long[LAYERS.length];
        long[] under1 = new long[LAYERS.length];
        long[] under2 = new long[LAYERS.length];
        int cubeSections = 0, blockSections = 0, offSections = 0;
        double[] box = new double[6];
        // TerrainOcclusion's last visibility of each section (its occluder flag), checked against this depth: a
        // section it hid must be hidden behind the final depth too
        Long2ObjectOpenHashMap<PackedSectionDraws> meshes = new Long2ObjectOpenHashMap<>();
        for (var section : mc.levelRenderer.visibleSections()) {
            if (section.getSectionMesh() instanceof PackedSectionDraws mesh) {
                meshes.put(section.getRenderOrigin().asLong(), mesh);
            }
        }
        int gpuHidden = 0, violations = 0;

        for (var entry : drawnQuads.long2ObjectEntrySet()) {
            long key = entry.getLongKey();
            int[] quads = entry.getValue();
            int ox = BlockPos.getX(key), oy = BlockPos.getY(key), oz = BlockPos.getZ(key);
            add(all, quads);

            cube(box, ox, oy, oz);
            int cube = test(box, clip, a, b);
            PackedSectionDraws mesh = meshes.get(key);
            if (mesh != null && !mesh.ruthenium$isOccluder()) {
                gpuHidden++;
                if (cube == VISIBLE) {
                    violations++;
                    if (violations <= 20) {
                        LOGGER.warn("Occlusion probe {}: GPU hid visible section {} {} {} at {} blocks, quads {}", view, ox, oy, oz,
                                "%.0f".formatted(distance(box)), java.util.Arrays.toString(quads));
                    }
                }
            }
            int blocks = blockBounds(mc, box, ox, oy, oz) ? test(box, clip, a, b) : cube;
            if (blocks == OFF_SCREEN) {
                offSections++;
                add(offQuads, quads);
                offScreen.add(key);
                continue;
            }
            if (cube == HIDDEN) {
                cubeSections++;
                add(cubeHidden, quads);
            }
            if (blocks == HIDDEN) {
                blockSections++;
                add(blocksHidden, quads);
                hidden.add(key);
                continue;
            }
            double blockPixels = focal / Math.max(1.0e-3, distance(box));
            if (blockPixels < 1.0) {
                add(under1, quads);
            }
            if (blockPixels < 2.0) {
                add(under2, quads);
            }
        }

        LOGGER.info("Occlusion probe {}: {} sections drawn, quads {}", view, drawnQuads.size(), describe(all, null));
        LOGGER.info("Occlusion probe {}: TerrainOcclusion hid {} sections, {} of them visible in the final depth", view, gpuHidden, violations);
        LOGGER.info("Occlusion probe {}: off screen {} sections, quads {}", view, offSections, describe(offQuads, all));
        LOGGER.info("Occlusion probe {}: hidden (section cube) {} sections, quads {}", view, cubeSections, describe(cubeHidden, all));
        LOGGER.info("Occlusion probe {}: hidden (block bounds) {} sections, quads {}", view, blockSections, describe(blocksHidden, all));
        LOGGER.info("Occlusion probe {}: visible, block face under 1 px: quads {}", view, describe(under1, all));
        LOGGER.info("Occlusion probe {}: visible, block face under 2 px: quads {}", view, describe(under2, all));
    }

    private static final int VISIBLE = 0, HIDDEN = 1, OFF_SCREEN = 2;

    /** Whether the box is hidden behind the depth buffer, off screen, or visible */
    private static int test(double[] box, Matrix4f clip, double a, double b) {
        double minSx = Double.POSITIVE_INFINITY, minSy = Double.POSITIVE_INFINITY;
        double maxSx = Double.NEGATIVE_INFINITY, maxSy = Double.NEGATIVE_INFINITY;
        double nearestInv = 0.0;
        Vector4f v = new Vector4f();
        for (int i = 0; i < 8; i++) {
            float x = (float) ((i & 1) == 0 ? box[0] : box[3]);
            float y = (float) ((i & 2) == 0 ? box[1] : box[4]);
            float z = (float) ((i & 4) == 0 ? box[2] : box[5]);
            clip.transform(v.set(x, y, z, 1.0f));
            if (v.w < NEAR_W) {
                return VISIBLE;
            }
            double inv = 1.0 / v.w;
            double sx = (v.x * inv * 0.5 + 0.5) * width;
            double sy = (v.y * inv * 0.5 + 0.5) * height;
            minSx = Math.min(minSx, sx);
            maxSx = Math.max(maxSx, sx);
            minSy = Math.min(minSy, sy);
            maxSy = Math.max(maxSy, sy);
            nearestInv = Math.max(nearestInv, inv);
        }
        // Pixel centres i + 0.5 within the projected box (rows bottom to top, as read back)
        int i0 = Math.max(0, (int) Math.ceil(minSx - BOUNDS_MARGIN - 0.5));
        int i1 = Math.min(width - 1, (int) Math.floor(maxSx + BOUNDS_MARGIN - 0.5));
        int j0 = Math.max(0, (int) Math.ceil(minSy - BOUNDS_MARGIN - 0.5));
        int j1 = Math.min(height - 1, (int) Math.floor(maxSy + BOUNDS_MARGIN - 0.5));
        if (i0 > i1 || j0 > j1) {
            return OFF_SCREEN;
        }
        double threshold = nearestInv * (1.0 + DEPTH_MARGIN);
        for (int j = j0; j <= j1; j++) {
            int row = j * width;
            for (int i = i0; i <= i1; i++) {
                // 1/w of what the pixel holds
                double inv = (depth[row + i] - a) / b;
                if (inv <= threshold) {
                    return VISIBLE;
                }
            }
        }
        return HIDDEN;
    }

    /** The section cube, camera-relative, grown by the margin */
    private static void cube(double[] box, int ox, int oy, int oz) {
        box[0] = ox - eye.x - BOX_MARGIN;
        box[1] = oy - eye.y - BOX_MARGIN;
        box[2] = oz - eye.z - BOX_MARGIN;
        box[3] = ox + 16 - eye.x + BOX_MARGIN;
        box[4] = oy + 16 - eye.y + BOX_MARGIN;
        box[5] = oz + 16 - eye.z + BOX_MARGIN;
    }

    /** The bounds of the section's non-air blocks, camera-relative, grown by the margin; false if unknown */
    private static boolean blockBounds(Minecraft mc, double[] box, int ox, int oy, int oz) {
        var chunk = mc.level.getChunk(ox >> 4, oz >> 4);
        int index = mc.level.getSectionIndex(oy);
        if (index < 0 || index >= chunk.getSections().length) {
            return false;
        }
        LevelChunkSection section = chunk.getSection(index);
        if (section.hasOnlyAir()) {
            return false;
        }
        int minX = 16, minY = 16, minZ = 16, maxX = -1, maxY = -1, maxZ = -1;
        for (int y = 0; y < 16; y++) {
            for (int z = 0; z < 16; z++) {
                for (int x = 0; x < 16; x++) {
                    if (!section.getBlockState(x, y, z).isAir()) {
                        minX = Math.min(minX, x);
                        minY = Math.min(minY, y);
                        minZ = Math.min(minZ, z);
                        maxX = Math.max(maxX, x);
                        maxY = Math.max(maxY, y);
                        maxZ = Math.max(maxZ, z);
                    }
                }
            }
        }
        box[0] = ox + minX - eye.x - BOX_MARGIN;
        box[1] = oy + minY - eye.y - BOX_MARGIN;
        box[2] = oz + minZ - eye.z - BOX_MARGIN;
        box[3] = ox + maxX + 1 - eye.x + BOX_MARGIN;
        box[4] = oy + maxY + 1 - eye.y + BOX_MARGIN;
        box[5] = oz + maxZ + 1 - eye.z + BOX_MARGIN;
        return true;
    }

    /** Distance from the camera to the nearest point of the box */
    private static double distance(double[] box) {
        double dx = Math.max(0.0, Math.max(box[0], -box[3]));
        double dy = Math.max(0.0, Math.max(box[1], -box[4]));
        double dz = Math.max(0.0, Math.max(box[2], -box[5]));
        return Math.sqrt(dx * dx + dy * dy + dz * dz);
    }

    /** {a, b, error}: depth = a + b / w along the view axis, fitted at 1 and 100 blocks, checked at 10 */
    private static double[] fitDepth(Matrix4f projection) {
        Vector4f near = projection.transform(new Vector4f(0.0f, 0.0f, -1.0f, 1.0f));
        Vector4f far = projection.transform(new Vector4f(0.0f, 0.0f, -100.0f, 1.0f));
        Vector4f mid = projection.transform(new Vector4f(0.0f, 0.0f, -10.0f, 1.0f));
        double b = (near.z / near.w - far.z / far.w) / (1.0 / near.w - 1.0 / far.w);
        double a = near.z / near.w - b / near.w;
        double error = Math.abs(a + b / mid.w - mid.z / mid.w);
        return new double[]{a, b, error};
    }

    private static void add(long[] total, int[] quads) {
        for (int l = 0; l < quads.length; l++) {
            total[l] += quads[l];
        }
    }

    private static String describe(long[] quads, long[] of) {
        StringBuilder line = new StringBuilder();
        for (ChunkSectionLayer layer : LAYERS) {
            int l = layer.ordinal();
            if (!line.isEmpty()) {
                line.append(", ");
            }
            line.append(layer.label()).append(' ').append(quads[l]);
            if (of != null && of[l] > 0) {
                line.append(" (").append("%.1f".formatted(100.0 * quads[l] / of[l])).append("%)");
            }
        }
        return line.toString();
    }
}
