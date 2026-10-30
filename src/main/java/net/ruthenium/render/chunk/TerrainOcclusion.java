package net.ruthenium.render.chunk;

import com.mojang.blaze3d.buffers.Std140Builder;
import com.mojang.blaze3d.pipeline.RenderTarget;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.GpuFormat;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.commands.CommandEncoder;
import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.device.GpuDevice;
import com.mojang.renderpearl.api.pipeline.BindGroupLayout;
import com.mojang.renderpearl.api.pipeline.BlendFunction;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.renderpearl.api.pipeline.UniformType;
import com.mojang.renderpearl.api.textures.FilterMode;
import com.mojang.renderpearl.api.textures.GpuSampler;
import com.mojang.renderpearl.api.textures.GpuTexture;
import com.mojang.renderpearl.api.textures.GpuTextureView;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.minecraft.client.renderer.BindGroupLayouts;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.minecraft.client.renderer.chunk.ChunkSectionLayerGroup;
import net.minecraft.client.renderer.chunk.ChunkSectionsToRender;
import net.minecraft.resources.Identifier;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.PackedSectionDraws;
import org.joml.Vector4f;
import org.joml.Vector4fc;
import org.jspecify.annotations.Nullable;

import java.nio.ByteBuffer;
import java.util.ArrayDeque;
import java.util.Arrays;
import java.util.List;
import java.util.Optional;

/**
 * Two-phase occlusion culling of the terrain on the GPU, through renderpearl's render passes and copies only (no
 * compute): {@link TerrainDrawList} draws the solid and cutout layers of the sections that were visible a few
 * frames ago from the CPU as before (the occluders), and its other commands (the other sections' solid and cutout
 * layers, and every translucent layer) get their instance count from the GPU:
 * <ol>
 * <li>the occluders are drawn in a pass of their own before vanilla's main pass ({@link #execute});</li>
 * <li>their depth is reduced into a pyramid of farthest depths ({@code occlusion_hiz});</li>
 * <li>16 points per section info test parts of the section's box's screen rectangle against the depth (small
 * rectangles) or the pyramid, and write 0 or 1 to the section's texel of the visibility target, blending with MAX
 * ({@code occlusion_visibility});</li>
 * <li>five points per command write its ints, with the instance count replaced by its section's visibility, to
 * an R32_UINT target ({@code occlusion_commands}), which is copied into the buffer the indirect draws read.</li>
 * </ol>
 * Vanilla's main pass then draws those commands: the sections that became visible, and the translucent ones in
 * front of the occluders. A hidden section is drawn with 0 instances, so nothing is drawn that the depth test of
 * the occluders' depth wouldn't have rejected. The visibility is read back to choose the next frames' occluders
 * ({@link PackedSectionDraws#ruthenium$setOccluder}); a stale choice only costs time.
 * <p>
 * Render thread only.
 */
public final class TerrainOcclusion {
    /** Texels per row of the visibility target: one per section info */
    private static final int VISIBILITY_WIDTH = 256;
    /** Texels per row of the command target: one per command int */
    private static final int COMMAND_WIDTH = 4096;
    public static final int COMMAND_INTS = 5;

    private static final BindGroupLayout OCCLUSION_INFO = BindGroupLayout.builder()
            .withUniform("OcclusionInfo", UniformType.UNIFORM_BUFFER)
            .build();
    /** Points per section of the visibility pass, each reading a part of the depth under the section */
    private static final int VISIBILITY_POINTS = 16;
    private static final VertexFormat SECTION_FORMAT = VertexFormat.builder(1)
            .addAttribute("ChunkPosition", GpuFormat.RGB32_SINT)
            .addAttribute("ChunkVisibility", GpuFormat.R32_FLOAT)
            .build();
    private static final VertexFormat COMMAND_FORMAT = VertexFormat.builder(1)
            .addAttribute("Command", GpuFormat.RGBA32_UINT)
            .addAttribute("CommandSection", GpuFormat.R32_UINT)
            .build();

    /** 4x4 texels of the level below per texel: the depth into mip 0, then each even mip into the one two below it */
    private static final RenderPipeline HI_Z = RenderPipeline.builder()
            .withLocation(Identifier.fromNamespaceAndPath("ruthenium", "pipeline/occlusion_hiz"))
            .withVertexShader(Identifier.withDefaultNamespace("core/screenquad"))
            .withFragmentShader(Identifier.fromNamespaceAndPath("ruthenium", "core/occlusion_hiz"))
            .withBindGroupLayout(BindGroupLayouts.IN_SAMPLER)
            .withColorTargetState(new ColorTargetState(Optional.empty(), GpuFormat.R32_FLOAT, ColorTargetState.WRITE_ALL))
            .withPrimitiveTopology(PrimitiveTopology.TRIANGLES)
            .build();
    private static final RenderPipeline VISIBILITY = RenderPipeline.builder()
            .withLocation(Identifier.fromNamespaceAndPath("ruthenium", "pipeline/occlusion_visibility"))
            .withVertexShader(Identifier.fromNamespaceAndPath("ruthenium", "core/occlusion_visibility"))
            .withFragmentShader(Identifier.fromNamespaceAndPath("ruthenium", "core/occlusion_visibility"))
            .withBindGroupLayout(BindGroupLayouts.GLOBALS)
            .withBindGroupLayout(BindGroupLayouts.PROJECTION)
            .withBindGroupLayout(BindGroupLayouts.TERRAIN_INFO)
            .withBindGroupLayout(OCCLUSION_INFO)
            .withBindGroupLayout(BindGroupLayout.builder()
                    .withUniform("DepthSampler", UniformType.COMBINED_IMAGE_SAMPLER)
                    .withUniform("HiZSampler", UniformType.COMBINED_IMAGE_SAMPLER)
                    .build())
            .withVertexBinding(0, SECTION_FORMAT)
            .withColorTargetState(new ColorTargetState(Optional.of(BlendFunction.MAX), GpuFormat.R8_UNORM, ColorTargetState.WRITE_ALL))
            .withPrimitiveTopology(PrimitiveTopology.POINTS)
            .build();
    private static final RenderPipeline COMMANDS = RenderPipeline.builder()
            .withLocation(Identifier.fromNamespaceAndPath("ruthenium", "pipeline/occlusion_commands"))
            .withVertexShader(Identifier.fromNamespaceAndPath("ruthenium", "core/occlusion_commands"))
            .withFragmentShader(Identifier.fromNamespaceAndPath("ruthenium", "core/occlusion_commands"))
            .withBindGroupLayout(OCCLUSION_INFO)
            .withBindGroupLayout(BindGroupLayout.builder().withUniform("VisibilitySampler", UniformType.COMBINED_IMAGE_SAMPLER).build())
            .withVertexBinding(0, COMMAND_FORMAT)
            .withColorTargetState(new ColorTargetState(Optional.empty(), GpuFormat.R32_UINT, ColorTargetState.WRITE_ALL))
            .withPrimitiveTopology(PrimitiveTopology.POINTS)
            .build();
    public static final List<RenderPipeline> PIPELINES = List.of(HI_Z, VISIBILITY, COMMANDS);

    /**
     * The culling costs about 0.1 ms of GPU time at 1920x1080 (mostly reading the depth for the pyramid) and a few
     * render passes: it stays on while at least {@link #KEEP_HIDDEN} of the solid and cutout quads it tested are
     * hidden, and starts when a check finds {@link #START_HIDDEN}. Without it, one frame every {@link #PROBE_NANOS}
     * is culled to check (time-based, so fast frames don't check more often).
     */
    private static final double START_HIDDEN = 0.15;
    private static final double KEEP_HIDDEN = 0.10;
    private static final long PROBE_NANOS = 250_000_000L;
    /** The highest pyramid mip written: its texels cover 64x64 pixels, so it holds boxes up to 1024 pixels wide; wider ones are drawn */
    private static final int MAX_LEVEL = 4;

    private static final Runnable NOTHING = () -> {
    };
    private static final Vector4fc CLEAR = new Vector4f(0.0F);

    /** Whether the last visibility read back hid enough to keep culling */
    private static boolean paying;
    private static long lastProbe;

    // This frame's work, from TerrainDrawList.prepare
    private static boolean pending;
    private static @Nullable ChunkSectionsToRender occluders;
    private static int sectionCount;
    private static int commandCount;
    private static @Nullable GpuBufferSlice sectionInfos;
    private static @Nullable GpuBufferSlice terrainTransform;
    private static @Nullable GpuBufferSlice templates;
    private static PackedSectionDraws @Nullable [] meshes = new PackedSectionDraws[0];

    // The pyramid: mip 0 has a quarter of the depth's size and only even mips are written; views of those to render
    // them, and of all mips to read them
    private static @Nullable GpuTexture hiZ;
    private static GpuTextureView @Nullable [] hiZLevels;
    private static @Nullable GpuTextureView hiZView;
    private static int depthWidth;
    private static int depthHeight;

    private static @Nullable GpuTexture visibility;
    private static @Nullable GpuTextureView visibilityView;
    private static @Nullable GpuTexture commandTexture;
    private static @Nullable GpuTextureView commandView;
    /** The commands the indirect draws read: the command target's rows, copied */
    private static @Nullable GpuBuffer commands;
    private static @Nullable GpuBuffer retiredCommands;
    private static @Nullable MappableRingBuffer info;

    /** Visibility read back for the occluder choice: free buffers of the visibility target's size, and in flight */
    private static final ArrayDeque<Readback> FREE_READBACKS = new ArrayDeque<>();

    private TerrainOcclusion() {
    }

    private static final class Readback {
        final GpuBuffer buffer;
        PackedSectionDraws[] meshes = new PackedSectionDraws[0];
        int count;

        Readback(GpuBuffer buffer) {
            this.buffer = buffer;
        }
    }

    /**
     * Whether this frame's terrain is culled (once per frame): the switch is on, the pipelines compiled, and the
     * culling pays, or a check is due ({@link #PROBE_NANOS})
     */
    static boolean cullThisFrame() {
        if (!Initializer.CONFIG.terrainOcclusionCulling
                || RenderSystem.getCompiledPipelineNullable(HI_Z) == null
                || RenderSystem.getCompiledPipelineNullable(VISIBILITY) == null
                || RenderSystem.getCompiledPipelineNullable(COMMANDS) == null) {
            return false;
        }
        if (paying) {
            return true;
        }
        long now = System.nanoTime();
        if (now - lastProbe < PROBE_NANOS) {
            return false;
        }
        lastProbe = now;
        return true;
    }

    /**
     * The buffer the culled commands' indirect draws read, holding at least this many commands (with the command
     * target to match). Its content is written by {@link #execute} before the main pass draws.
     */
    static GpuBuffer commandBuffer(int count) {
        int rows = Math.max(1, (count * COMMAND_INTS + COMMAND_WIDTH - 1) / COMMAND_WIDTH);
        if (commandTexture == null || commandTexture.getHeight(0) < rows) {
            GpuDevice device = RenderSystem.getDevice();
            int capacity = Math.max(rows * 3 / 2, 4);
            closeCommandTarget();
            commandTexture = device.createTexture(() -> "Ruthenium terrain commands",
                    GpuTexture.USAGE_RENDER_ATTACHMENT | GpuTexture.USAGE_COPY_SRC, GpuFormat.R32_UINT, COMMAND_WIDTH, capacity, 1, 1);
            commandView = device.createTextureView(commandTexture);
            if (retiredCommands != null) {
                retiredCommands.close();
            }
            // Frames in flight may still draw from the old buffer
            retiredCommands = commands;
            commands = device.createBuffer(() -> "Ruthenium culled terrain commands",
                    GpuBuffer.USAGE_INDIRECT_PARAMETERS | GpuBuffer.USAGE_COPY_DST, (long) COMMAND_WIDTH * capacity * Integer.BYTES);
        }
        return commands;
    }

    /**
     * This frame's work: the occluders' draws (solid and cutout only), the section infos the commands' base instances
     * index (with the mesh of each, null if it can't keep a history) and the culled commands' templates
     */
    static void prepare(@Nullable ChunkSectionsToRender occluderDraws, GpuBufferSlice infos, List<PackedSectionDraws> infoMeshes,
                        GpuBufferSlice transform, @Nullable GpuBufferSlice culledTemplates, int culledCommands) {
        occluders = occluderDraws;
        sectionInfos = infos;
        sectionCount = infoMeshes.size();
        terrainTransform = transform;
        templates = culledTemplates;
        commandCount = culledCommands;
        if (meshes.length < sectionCount) {
            meshes = new PackedSectionDraws[Math.max(sectionCount, meshes.length * 2)];
        }
        for (int i = 0; i < sectionCount; i++) {
            meshes[i] = infoMeshes.get(i);
        }
        pending = sectionCount > 0;
    }

    /**
     * Before vanilla's main pass (classic or OIT): draws the occluders into the main target, then writes the culled
     * commands
     */
    public static void execute(RenderTarget target, @Nullable GpuSampler terrainSampler, GpuTextureView atlas, boolean wireframe) {
        if (!pending) {
            return;
        }
        pending = false;
        GpuTextureView depthView = target.getDepthTextureView();
        if (depthView == null || terrainSampler == null || sectionInfos == null || terrainTransform == null) {
            return;
        }
        GpuDevice device = RenderSystem.getDevice();
        CommandEncoder encoder = device.createCommandEncoder();
        GpuSampler nearest = RenderSystem.getSamplerCache().getClampToEdge(FilterMode.NEAREST);

        boolean hasOccluders = occluders != null;
        if (occluders != null) {
            try (RenderPass renderPass = encoder.createRenderPass(() -> "Ruthenium terrain occluders", target.getColorTextureView(),
                    Optional.empty(), depthView, java.util.OptionalDouble.empty())) {
                RenderSystem.bindDefaultUniforms(renderPass);
                occluders.renderGroup(ChunkSectionLayerGroup.OPAQUE, renderPass, terrainSampler, atlas, wireframe);
            }
        }
        occluders = null;

        ensureHiZ(device, target.width, target.height);
        if (hasOccluders) {
            GpuTextureView source = depthView;
            for (int level = 0; level < hiZLevels.length; level += 2) {
                try (RenderPass renderPass = encoder.createRenderPass(() -> "Ruthenium terrain Hi-Z", hiZLevels[level], Optional.empty())) {
                    renderPass.setPipeline(RenderSystem.getCompiledPipeline(HI_Z));
                    renderPass.setUniform("InSampler", source, nearest);
                    renderPass.draw(3, 1, 0, 0);
                }
                source = hiZLevels[level];
            }
        }

        int visibilityRows = (sectionCount + VISIBILITY_WIDTH - 1) / VISIBILITY_WIDTH;
        ensureVisibility(device, visibilityRows);
        if (info == null) {
            info = new MappableRingBuffer(() -> "Ruthenium terrain occlusion info", GpuBuffer.USAGE_UNIFORM | GpuBuffer.USAGE_MAP_WRITE, 32);
        }
        info.rotate();
        GpuBuffer infoBuffer = info.currentBuffer();
        try (GpuBufferSlice.MappedView view = infoBuffer.map(false, true)) {
            Std140Builder.intoBuffer(view.data())
                    .putIVec2(depthWidth, depthHeight)
                    .putIVec2(VISIBILITY_WIDTH, visibility.getHeight(0))
                    .putIVec2(COMMAND_WIDTH, commandTexture != null ? commandTexture.getHeight(0) : 1)
                    .putInt(hasOccluders ? hiZLevels.length : 0);
        }

        try (RenderPass renderPass = encoder.createRenderPass(() -> "Ruthenium section visibility", visibilityView, Optional.of(CLEAR))) {
            RenderSystem.bindDefaultUniforms(renderPass);
            renderPass.setPipeline(RenderSystem.getCompiledPipeline(VISIBILITY));
            renderPass.setUniform("TerrainUniform", terrainTransform);
            renderPass.setUniform("OcclusionInfo", infoBuffer);
            renderPass.setUniform("DepthSampler", depthView, nearest);
            renderPass.setUniform("HiZSampler", hiZView, nearest);
            renderPass.setVertexBuffer(0, sectionInfos);
            renderPass.draw(VISIBILITY_POINTS, sectionCount, 0, 0);
        }

        if (commandCount > 0 && templates != null && commandTexture != null && commands != null) {
            try (RenderPass renderPass = encoder.createRenderPass(() -> "Ruthenium culled terrain commands", commandView, Optional.empty())) {
                renderPass.setPipeline(RenderSystem.getCompiledPipeline(COMMANDS));
                renderPass.setUniform("OcclusionInfo", infoBuffer);
                renderPass.setUniform("VisibilitySampler", visibilityView, nearest);
                renderPass.setVertexBuffer(0, templates);
                renderPass.draw(COMMAND_INTS, commandCount, 0, 0);
            }
            int rows = (commandCount * COMMAND_INTS + COMMAND_WIDTH - 1) / COMMAND_WIDTH;
            encoder.copyTextureToBuffer(commandTexture, commands, 0L, NOTHING, 0, 0, 0, COMMAND_WIDTH, rows);
        }
        templates = null;

        readBack(device, encoder, visibilityRows);
    }

    /** Copies the visibility for the occluder choice of a later frame */
    private static void readBack(GpuDevice device, CommandEncoder encoder, int rows) {
        long size = (long) VISIBILITY_WIDTH * visibility.getHeight(0);
        Readback readback = FREE_READBACKS.poll();
        if (readback == null) {
            readback = new Readback(device.createBuffer(() -> "Ruthenium section visibility readback",
                    GpuBuffer.USAGE_MAP_READ | GpuBuffer.USAGE_COPY_DST, size));
        }
        if (readback.meshes.length < sectionCount) {
            readback.meshes = new PackedSectionDraws[meshes.length];
        }
        System.arraycopy(meshes, 0, readback.meshes, 0, sectionCount);
        Arrays.fill(meshes, 0, sectionCount, null);
        readback.count = sectionCount;
        Readback finished = readback;
        encoder.copyTextureToBuffer(visibility, readback.buffer, 0L, () -> apply(finished, size), 0, 0, 0, VISIBILITY_WIDTH, rows);
    }

    private static void apply(Readback readback, long size) {
        try (GpuBufferSlice.MappedView view = readback.buffer.map(true, false)) {
            ByteBuffer data = view.data();
            PackedSectionDraws[] infoMeshes = readback.meshes;
            long quads = 0, hidden = 0;
            for (int i = 0; i < readback.count; i++) {
                PackedSectionDraws mesh = infoMeshes[i];
                if (mesh != null) {
                    boolean visible = data.get(i) != 0;
                    mesh.ruthenium$setOccluder(visible);
                    int meshQuads = TerrainDrawList.packedQuads(mesh.ruthenium$getPackedDraws());
                    quads += meshQuads;
                    if (!visible) {
                        hidden += meshQuads;
                    }
                    infoMeshes[i] = null;
                }
            }
            paying = hidden >= (paying ? KEEP_HIDDEN : START_HIDDEN) * quads && hidden > 0;
        }
        // A buffer of an older visibility target's size is dropped
        if (visibility != null && size == (long) VISIBILITY_WIDTH * visibility.getHeight(0)) {
            FREE_READBACKS.add(readback);
        } else {
            readback.buffer.close();
        }
    }

    private static void ensureHiZ(GpuDevice device, int width, int height) {
        if (hiZ != null && depthWidth == width && depthHeight == height) {
            return;
        }
        closeHiZ();
        depthWidth = width;
        depthHeight = height;
        int levelWidth = Math.max(1, width / 4), levelHeight = Math.max(1, height / 4);
        int levels = 1;
        for (int w = levelWidth, h = levelHeight; (w > 1 || h > 1) && levels <= MAX_LEVEL; levels++) {
            w = Math.max(1, w >> 1);
            h = Math.max(1, h >> 1);
        }
        hiZ = device.createTexture(() -> "Ruthenium terrain Hi-Z", GpuTexture.USAGE_RENDER_ATTACHMENT | GpuTexture.USAGE_TEXTURE_BINDING,
                GpuFormat.R32_FLOAT, levelWidth, levelHeight, 1, levels);
        hiZLevels = new GpuTextureView[levels];
        for (int level = 0; level < levels; level += 2) {
            hiZLevels[level] = device.createTextureView(hiZ, level, 1);
        }
        hiZView = device.createTextureView(hiZ);
    }

    private static void ensureVisibility(GpuDevice device, int rows) {
        if (visibility != null && visibility.getHeight(0) >= rows) {
            return;
        }
        if (visibilityView != null) {
            visibilityView.close();
        }
        if (visibility != null) {
            visibility.close();
        }
        visibility = device.createTexture(() -> "Ruthenium section visibility",
                GpuTexture.USAGE_RENDER_ATTACHMENT | GpuTexture.USAGE_TEXTURE_BINDING | GpuTexture.USAGE_COPY_SRC, GpuFormat.R8_UNORM,
                VISIBILITY_WIDTH, Math.max(rows * 3 / 2, 4), 1, 1);
        visibilityView = device.createTextureView(visibility);
        Readback readback;
        while ((readback = FREE_READBACKS.poll()) != null) {
            readback.buffer.close();
        }
    }

    private static void closeHiZ() {
        if (hiZLevels != null) {
            for (GpuTextureView level : hiZLevels) {
                if (level != null) {
                    level.close();
                }
            }
            hiZLevels = null;
        }
        if (hiZView != null) {
            hiZView.close();
            hiZView = null;
        }
        if (hiZ != null) {
            hiZ.close();
            hiZ = null;
        }
    }

    private static void closeCommandTarget() {
        if (commandView != null) {
            commandView.close();
            commandView = null;
        }
        if (commandTexture != null) {
            commandTexture.close();
            commandTexture = null;
        }
    }
}
