package net.vulkanmod.render.chunk;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.pipeline.IndexType;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import it.unimi.dsi.fastutil.objects.ObjectArrayList;
import net.minecraft.client.renderer.DynamicGpuData;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.ChunkSectionsToRender;
import net.minecraft.client.renderer.chunk.SectionMesh;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;
import net.minecraft.core.BlockPos;
import net.minecraft.util.Util;
import net.minecraft.world.phys.Vec3;
import net.vulkanmod.Initializer;
import net.vulkanmod.interfaces.CachedSectionSlices;
import net.vulkanmod.interfaces.FaceGroupsHolder;
import org.joml.Matrix4fc;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteOrder;
import java.nio.IntBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.EnumMap;
import java.util.List;

/**
 * Builds the terrain's multi-draw-indirect commands like vanilla's {@code LevelRenderer.prepareChunkRendersIndirect},
 * without its per-frame costs: a solid or cutout layer's buffer slice is cached in the compiled mesh instead of
 * being looked up in the uber buffers' hash maps, commands are collected in reused int arrays instead of a record
 * per draw, and they are written to mapped memory in one copy per group. Draw groups, their order (translucent
 * groups and draws back to front) and the commands are the same as vanilla's, plus the face groups of
 * {@link SectionFaceGroups}.
 * <p>
 * Render thread only.
 */
public final class TerrainDrawList {
    private static final ChunkSectionLayer[] LAYERS = ChunkSectionLayer.values();
    private static final int COMMAND_INTS = 5;
    private static final int COMMAND_BYTES = COMMAND_INTS * Integer.BYTES;

    private static final int[] VERTEX_SIZES = new int[LAYERS.length];
    @SuppressWarnings("unchecked")
    private static final List<Group>[] GROUPS = new List[LAYERS.length];
    private static final List<Group> POOL = new ArrayList<>();
    private static final List<DynamicGpuData.ChunkSectionInfo> SECTION_INFOS = new ArrayList<>();

    private static @Nullable MappableRingBuffer commands;
    private static @Nullable MappableRingBuffer retiredCommands;

    static {
        for (ChunkSectionLayer layer : LAYERS) {
            GROUPS[layer.ordinal()] = new ArrayList<>();
        }
    }

    private TerrainDrawList() {
    }

    /** One multi-draw: sections sharing the same vertex and index buffers */
    static final class Group {
        GpuBuffer vertexBuffer;
        @Nullable GpuBuffer indexBuffer;
        @Nullable IndexType indexType;
        int[] commands = new int[COMMAND_INTS * 64];
        int count;

        boolean matches(GpuBuffer vertexBuffer, @Nullable GpuBuffer indexBuffer, @Nullable IndexType indexType) {
            return this.vertexBuffer == vertexBuffer && this.indexBuffer == indexBuffer && this.indexType == indexType;
        }

        void add(int indexCount, int firstIndex, int baseVertex, int baseInstance) {
            int i = this.count * COMMAND_INTS;
            if (i + COMMAND_INTS > this.commands.length) {
                this.commands = Arrays.copyOf(this.commands, this.commands.length * 2);
            }
            int[] c = this.commands;
            c[i] = indexCount;
            c[i + 1] = 1;
            c[i + 2] = firstIndex;
            c[i + 3] = baseVertex;
            c[i + 4] = baseInstance;
            this.count++;
        }
    }

    public static ChunkSectionsToRender prepare(@Nullable SectionRenderDispatcher dispatcher,
                                                ObjectArrayList<SectionRenderDispatcher.RenderSection> visibleSections,
                                                Vec3 eye, double fadeInSeconds, Matrix4fc modelView,
                                                int atlasWidth, int atlasHeight, boolean respectTranslucentOrder) {
        for (List<Group> groups : GROUPS) {
            for (Group group : groups) {
                group.count = 0;
                POOL.add(group);
            }
            groups.clear();
        }
        SECTION_INFOS.clear();
        // The layers' vertex format can change when the section dispatcher is recreated (CompressedTerrain)
        for (ChunkSectionLayer layer : LAYERS) {
            VERTEX_SIZES[layer.ordinal()] = layer.pipeline(false).getVertexFormatBinding(0).getVertexSize();
        }

        int largestIndexCount = 0;
        if (dispatcher != null) {
            dispatcher.lock();
            try {
                largestIndexCount = collect(dispatcher, visibleSections, eye, fadeInSeconds, respectTranslucentOrder);
            } finally {
                dispatcher.unlock();
            }
        }

        EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> indirectDraws = writeCommands(respectTranslucentOrder);

        GpuBufferSlice terrainTransform = RenderSystem.getDynamicUniforms().writeTerrainTransform(modelView, atlasWidth, atlasHeight);
        GpuBufferSlice sectionInfos = RenderSystem.getDynamicUniforms().writeChunkSectionsInstanced(SECTION_INFOS);
        RenderSystem.AutoStorageIndexBuffer autoIndices = RenderSystem.getSequentialBuffer(PrimitiveTopology.QUADS);
        if (largestIndexCount != 0) {
            autoIndices.requestIndexCount(largestIndexCount);
        }

        return new ChunkSectionsToRender.DrawIndirect(terrainTransform, indirectDraws, largestIndexCount, sectionInfos);
    }

    private static int collect(SectionRenderDispatcher dispatcher, ObjectArrayList<SectionRenderDispatcher.RenderSection> visibleSections,
                               Vec3 eye, double fadeInSeconds, boolean respectTranslucentOrder) {
        long fadeDuration = Util.toMillis(fadeInSeconds);
        long now = Util.getMillis();
        boolean faceCulling = Initializer.CONFIG.terrainFaceCulling;
        int largestIndexCount = 0;
        @Nullable Group lastTranslucent = null;

        Object[] sections = visibleSections.elements();
        for (int s = 0, size = visibleSections.size(); s < size; s++) {
            SectionRenderDispatcher.RenderSection section = (SectionRenderDispatcher.RenderSection) sections[s];
            SectionMesh mesh = section.getSectionMesh();
            BlockPos origin = section.getRenderOrigin();
            int sectionInfo = -1;

            for (ChunkSectionLayer layer : LAYERS) {
                SectionMesh.SectionDraw draw = mesh.getSectionDraw(layer);
                if (draw == null) {
                    continue;
                }

                boolean customIndices = draw.hasCustomIndexBuffer();
                SectionRenderDispatcher.RenderSectionBufferSlice slice = slice(dispatcher, mesh, layer, customIndices);
                if (slice == null || customIndices && slice.indexBuffer() == null) {
                    continue;
                }

                if (sectionInfo == -1) {
                    sectionInfo = SECTION_INFOS.size();
                    SECTION_INFOS.add(new DynamicGpuData.ChunkSectionInfo(origin.getX(), origin.getY(), origin.getZ(),
                            section.getVisibility(now, fadeDuration)));
                }

                GpuBuffer vertexBuffer = slice.vertexBuffer();
                GpuBuffer indexBuffer = null;
                IndexType indexType = null;
                int firstIndex = 0;
                if (customIndices) {
                    indexBuffer = slice.indexBuffer();
                    indexType = draw.indexType();
                    firstIndex = (int) (slice.indexBufferOffset() / indexType.bytes);
                } else if (draw.indexCount() > largestIndexCount) {
                    largestIndexCount = draw.indexCount();
                }
                int baseVertex = (int) (slice.vertexBufferOffset() / VERTEX_SIZES[layer.ordinal()]);

                List<Group> layerGroups = GROUPS[layer.ordinal()];
                Group group = null;
                if (layer.translucent() && respectTranslucentOrder) {
                    // Consecutive draws in the same buffers share a group, so the order between sections is kept
                    if (lastTranslucent != null && lastTranslucent.matches(vertexBuffer, indexBuffer, indexType)) {
                        group = lastTranslucent;
                    }
                } else {
                    for (int g = 0, n = layerGroups.size(); g < n; g++) {
                        Group candidate = layerGroups.get(g);
                        if (candidate.matches(vertexBuffer, indexBuffer, indexType)) {
                            group = candidate;
                            break;
                        }
                    }
                }
                if (group == null) {
                    group = POOL.isEmpty() ? new Group() : POOL.removeLast();
                    group.vertexBuffer = vertexBuffer;
                    group.indexBuffer = indexBuffer;
                    group.indexType = indexType;
                    layerGroups.add(group);
                }
                if (layer.translucent()) {
                    lastTranslucent = group;
                }

                SectionFaceGroups faces = faceCulling && !customIndices && mesh instanceof FaceGroupsHolder holder
                        ? holder.vulkanmod$getFaceGroups(layer) : null;
                if (faces != null && draw.indexCount() == faces.quadCount() * 6) {
                    faces.addVisibleCommands(group, firstIndex, baseVertex, sectionInfo,
                            (float) (eye.x - origin.getX()), (float) (eye.y - origin.getY()), (float) (eye.z - origin.getZ()));
                } else {
                    group.add(draw.indexCount(), firstIndex, baseVertex, sectionInfo);
                }
            }
        }
        return largestIndexCount;
    }

    /**
     * The layer's place in the uber buffers. A solid or cutout layer's vertex allocation stays where it is until its
     * mesh is released (only translucent index buffers are re-uploaded under the same mesh, when resorted), and a mesh
     * is only drawn once all its buffers are uploaded, so the slice is cached in the mesh on first use.
     */
    private static SectionRenderDispatcher.@Nullable RenderSectionBufferSlice slice(SectionRenderDispatcher dispatcher, SectionMesh mesh,
                                                                                     ChunkSectionLayer layer, boolean customIndices) {
        if (customIndices || layer.translucent() || !(mesh instanceof CachedSectionSlices cache)) {
            return dispatcher.getRenderSectionSlice(mesh, layer);
        }
        SectionRenderDispatcher.RenderSectionBufferSlice slice = cache.vulkanmod$getSlice(layer);
        if (slice == null) {
            slice = dispatcher.getRenderSectionSlice(mesh, layer);
            if (slice != null) {
                cache.vulkanmod$setSlice(layer, slice);
            }
        }
        return slice;
    }

    private static EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> writeCommands(boolean respectTranslucentOrder) {
        EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> indirectDraws = new EnumMap<>(ChunkSectionLayer.class);
        int total = 0;
        for (ChunkSectionLayer layer : LAYERS) {
            indirectDraws.put(layer, new ArrayList<>());
            for (Group group : GROUPS[layer.ordinal()]) {
                total += group.count;
            }
        }

        if (retiredCommands != null) {
            retiredCommands.close();
            retiredCommands = null;
        }
        if (commands != null) {
            // Fences the previous frame's use of the current buffer
            commands.rotate();
        }
        if (total == 0) {
            return indirectDraws;
        }

        int bytes = total * COMMAND_BYTES;
        if (commands == null || commands.size() < bytes) {
            // The old buffers may still be read by frames in flight
            retiredCommands = commands;
            commands = new MappableRingBuffer(() -> "VulkanMod terrain draw commands",
                    GpuBuffer.USAGE_INDIRECT_PARAMETERS | GpuBuffer.USAGE_MAP_WRITE, Math.max(bytes * 3 / 2, 64 * 1024));
        }

        GpuBuffer buffer = commands.currentBuffer();
        try (GpuBufferSlice.MappedView view = buffer.slice(0, bytes).map(false, true)) {
            long address = MemoryUtil.memAddress(view.data());
            IntBuffer ints = view.data().order(ByteOrder.nativeOrder()).asIntBuffer();
            long offset = 0;
            for (ChunkSectionLayer layer : LAYERS) {
                List<Group> groups = GROUPS[layer.ordinal()];
                boolean backToFront = layer.translucent() && respectTranslucentOrder;
                List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect> layerDraws = indirectDraws.get(layer);
                for (int g = 0, n = groups.size(); g < n; g++) {
                    Group group = groups.get(backToFront ? n - 1 - g : g);
                    long start = offset;
                    if (backToFront) {
                        for (int c = group.count - 1; c >= 0; c--) {
                            putCommand(address + offset, group.commands, c);
                            offset += COMMAND_BYTES;
                        }
                    } else {
                        ints.put((int) (offset / Integer.BYTES), group.commands, 0, group.count * COMMAND_INTS);
                        offset += (long) group.count * COMMAND_BYTES;
                    }
                    layerDraws.add(new ChunkSectionsToRender.GpuMultiDrawIndexedIndirect(group.vertexBuffer.slice(),
                            group.indexBuffer != null ? group.indexBuffer.slice() : null, group.indexType,
                            buffer.slice(start, offset - start), group.count));
                }
            }
        }
        return indirectDraws;
    }

    private static void putCommand(long address, int[] commands, int command) {
        int i = command * COMMAND_INTS;
        MemoryUtil.memPutInt(address, commands[i]);
        MemoryUtil.memPutInt(address + 4, commands[i + 1]);
        MemoryUtil.memPutInt(address + 8, commands[i + 2]);
        MemoryUtil.memPutInt(address + 12, commands[i + 3]);
        MemoryUtil.memPutInt(address + 16, commands[i + 4]);
    }
}
