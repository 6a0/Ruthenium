package net.ruthenium.render.chunk;

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
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.FaceGroupsHolder;
import net.ruthenium.interfaces.PackedSectionDraws;
import org.joml.Matrix4fc;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.IntBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.EnumMap;
import java.util.List;

/**
 * Builds the terrain's multi-draw-indirect commands like vanilla's {@code LevelRenderer.prepareChunkRendersIndirect},
 * without its per-frame costs: a compiled mesh's solid and cutout draws are packed once into an int array on the
 * mesh ({@link #pack}) instead of being looked up in the uber buffers' hash maps every frame, commands are collected
 * in reused int arrays instead of a record per draw, and they are written to mapped memory in one copy per group.
 * Draw groups, their order (translucent groups and draws back to front) and the commands are the same as vanilla's,
 * plus the face groups of {@link SectionFaceGroups}.
 * <p>
 * With {@link TerrainOcclusion} on, only the solid and cutout commands of the sections that were last seen visible
 * (the occluders) are written here; the others' and every translucent command are written as templates whose
 * instance count {@link TerrainOcclusion} writes on the GPU, into the buffer the returned draws read.
 * <p>
 * Render thread only.
 */
public final class TerrainDrawList {
    private static final ChunkSectionLayer[] LAYERS = ChunkSectionLayer.values();
    private static final int COMMAND_INTS = 5;
    private static final int COMMAND_BYTES = COMMAND_INTS * Integer.BYTES;

    /** Layers packed per mesh: solid and cutout (translucent index buffers move when they are resorted) */
    private static final int PACKED_LAYERS = 2;
    /** Per packed layer: drawn (0 or 1), index count, base vertex, face groups (0 or 1), then the groups */
    private static final int PACKED_STRIDE = 4 + SectionFaceGroups.PACKED_INTS;
    /** Meshes with a custom index buffer in a packed layer use the lookups every frame */
    private static final int[] UNPACKABLE = new int[0];

    private static final int[] VERTEX_SIZES = new int[LAYERS.length];
    @SuppressWarnings("unchecked")
    private static final List<Group>[] GROUPS = new List[LAYERS.length];
    /** The commands {@link TerrainOcclusion} culls */
    @SuppressWarnings("unchecked")
    private static final List<Group>[] CULLED_GROUPS = new List[LAYERS.length];
    private static final List<Group> POOL = new ArrayList<>();
    private static final List<DynamicGpuData.ChunkSectionInfo> SECTION_INFOS = new ArrayList<>();
    /** The mesh of each section info, for its occluder history (null if it has none) */
    private static final List<@Nullable PackedSectionDraws> SECTION_MESHES = new ArrayList<>();

    private static @Nullable MappableRingBuffer commands;
    private static @Nullable MappableRingBuffer retiredCommands;
    private static @Nullable MappableRingBuffer templates;
    private static @Nullable MappableRingBuffer retiredTemplates;

    static {
        for (ChunkSectionLayer layer : LAYERS) {
            GROUPS[layer.ordinal()] = new ArrayList<>();
            CULLED_GROUPS[layer.ordinal()] = new ArrayList<>();
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
                                                int atlasWidth, int atlasHeight, boolean respectTranslucentOrder,
                                                boolean wireframe) {
        release(GROUPS);
        release(CULLED_GROUPS);
        SECTION_INFOS.clear();
        SECTION_MESHES.clear();
        // Wireframe occluders would hide what is seen between their lines
        boolean occlusion = !wireframe && TerrainOcclusion.cullThisFrame();
        // The layers' vertex format can change when the section dispatcher is recreated (CompressedTerrain)
        for (ChunkSectionLayer layer : LAYERS) {
            VERTEX_SIZES[layer.ordinal()] = layer.pipeline(false).getVertexFormatBinding(0).getVertexSize();
        }

        int largestIndexCount = 0;
        if (dispatcher != null) {
            dispatcher.lock();
            try {
                largestIndexCount = collect(dispatcher, visibleSections, eye, fadeInSeconds, respectTranslucentOrder, occlusion);
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

        if (!occlusion) {
            return new ChunkSectionsToRender.DrawIndirect(terrainTransform, indirectDraws, largestIndexCount, sectionInfos);
        }
        int culled = count(CULLED_GROUPS);
        EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> culledDraws = writeTemplates(culled, respectTranslucentOrder);
        ChunkSectionsToRender occluders = count(GROUPS) == 0 ? null
                : new ChunkSectionsToRender.DrawIndirect(terrainTransform, indirectDraws, largestIndexCount, sectionInfos);
        TerrainOcclusion.prepare(occluders, sectionInfos, SECTION_MESHES, terrainTransform,
                culled == 0 ? null : templates.currentBuffer().slice(0, (long) culled * COMMAND_BYTES), culled);
        return new ChunkSectionsToRender.DrawIndirect(terrainTransform, culledDraws, largestIndexCount, sectionInfos);
    }

    /** The solid and cutout quads of a mesh's packed draws (0 if it has none) */
    static int packedQuads(int @Nullable [] packed) {
        if (packed == null || packed == UNPACKABLE) {
            return 0;
        }
        int indices = 0;
        for (int l = 0; l < PACKED_LAYERS; l++) {
            int o = l * PACKED_STRIDE;
            if (packed[o] != 0) {
                indices += packed[o + 1];
            }
        }
        return indices / 6;
    }

    private static void release(List<Group>[] layerGroups) {
        for (List<Group> groups : layerGroups) {
            for (Group group : groups) {
                group.count = 0;
                POOL.add(group);
            }
            groups.clear();
        }
    }

    private static int count(List<Group>[] layerGroups) {
        int total = 0;
        for (List<Group> groups : layerGroups) {
            for (Group group : groups) {
                total += group.count;
            }
        }
        return total;
    }

    private static int collect(SectionRenderDispatcher dispatcher, ObjectArrayList<SectionRenderDispatcher.RenderSection> visibleSections,
                               Vec3 eye, double fadeInSeconds, boolean respectTranslucentOrder, boolean occlusion) {
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
            float eyeX = (float) (eye.x - origin.getX()), eyeY = (float) (eye.y - origin.getY()), eyeZ = (float) (eye.z - origin.getZ());
            List<Group>[] opaqueGroups = !occlusion || mesh instanceof PackedSectionDraws history && history.ruthenium$isOccluder()
                    ? GROUPS : CULLED_GROUPS;
            List<Group>[] translucentGroups = occlusion ? CULLED_GROUPS : GROUPS;

            int[] packed = null;
            GpuBuffer[] packedBuffers = null;
            if (mesh instanceof PackedSectionDraws holder) {
                packed = holder.ruthenium$getPackedDraws();
                if (packed == null) {
                    packed = pack(dispatcher, mesh, holder);
                }
                if (packed == UNPACKABLE) {
                    packed = null;
                } else if (packed != null) {
                    packedBuffers = holder.ruthenium$getPackedBuffers();
                }
            }

            for (ChunkSectionLayer layer : LAYERS) {
                if (packed != null && layer.ordinal() < PACKED_LAYERS) {
                    int o = layer.ordinal() * PACKED_STRIDE;
                    if (packed[o] == 0) {
                        continue;
                    }
                    if (sectionInfo == -1) {
                        sectionInfo = addSectionInfo(section, mesh, origin, now, fadeDuration);
                    }
                    int indexCount = packed[o + 1];
                    largestIndexCount = Math.max(largestIndexCount, indexCount);
                    Group group = group(opaqueGroups[layer.ordinal()], packedBuffers[layer.ordinal()], null, null);
                    if (faceCulling && packed[o + 3] != 0) {
                        SectionFaceGroups.addVisibleCommands(packed, o + 4, group, 0, packed[o + 2], sectionInfo, eyeX, eyeY, eyeZ);
                    } else {
                        group.add(indexCount, 0, packed[o + 2], sectionInfo);
                    }
                    continue;
                }

                SectionMesh.SectionDraw draw = mesh.getSectionDraw(layer);
                if (draw == null) {
                    continue;
                }

                boolean customIndices = draw.hasCustomIndexBuffer();
                SectionRenderDispatcher.RenderSectionBufferSlice slice = dispatcher.getRenderSectionSlice(mesh, layer);
                if (slice == null || customIndices && slice.indexBuffer() == null) {
                    continue;
                }

                if (sectionInfo == -1) {
                    sectionInfo = addSectionInfo(section, mesh, origin, now, fadeDuration);
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

                List<Group> layerGroups = (layer.translucent() ? translucentGroups : opaqueGroups)[layer.ordinal()];
                Group group = null;
                if (layer.translucent() && respectTranslucentOrder) {
                    // Consecutive draws in the same buffers share a group, so the order between sections is kept
                    if (lastTranslucent != null && lastTranslucent.matches(vertexBuffer, indexBuffer, indexType)) {
                        group = lastTranslucent;
                    } else {
                        group = newGroup(layerGroups, vertexBuffer, indexBuffer, indexType);
                    }
                } else {
                    group = group(layerGroups, vertexBuffer, indexBuffer, indexType);
                }
                if (layer.translucent()) {
                    lastTranslucent = group;
                }

                SectionFaceGroups faces = faceCulling && !customIndices && mesh instanceof FaceGroupsHolder holder
                        ? holder.ruthenium$getFaceGroups(layer) : null;
                if (faces != null && draw.indexCount() == faces.quadCount() * 6) {
                    faces.addVisibleCommands(group, firstIndex, baseVertex, sectionInfo, eyeX, eyeY, eyeZ);
                } else {
                    group.add(draw.indexCount(), firstIndex, baseVertex, sectionInfo);
                }
            }
        }
        return largestIndexCount;
    }

    private static int addSectionInfo(SectionRenderDispatcher.RenderSection section, SectionMesh mesh, BlockPos origin, long now,
                                      long fadeDuration) {
        int index = SECTION_INFOS.size();
        int x = origin.getX(), y = origin.getY(), z = origin.getZ();
        float visibility = section.getVisibility(now, fadeDuration);
        DynamicGpuData.ChunkSectionInfo info;
        if (mesh instanceof PackedSectionDraws holder) {
            // The mesh's last info while its values are the same (a record: the same writes)
            info = holder.ruthenium$getSectionInfo();
            if (info == null || info.x() != x || info.y() != y || info.z() != z
                    || Float.floatToRawIntBits(info.visibility()) != Float.floatToRawIntBits(visibility)) {
                info = new DynamicGpuData.ChunkSectionInfo(x, y, z, visibility);
                holder.ruthenium$setSectionInfo(info);
            }
        } else {
            info = new DynamicGpuData.ChunkSectionInfo(x, y, z, visibility);
        }
        SECTION_INFOS.add(info);
        SECTION_MESHES.add(mesh instanceof PackedSectionDraws holder ? holder : null);
        return index;
    }

    /** The layer's group drawing from these buffers */
    private static Group group(List<Group> layerGroups, GpuBuffer vertexBuffer, @Nullable GpuBuffer indexBuffer, @Nullable IndexType indexType) {
        for (int g = 0, n = layerGroups.size(); g < n; g++) {
            Group candidate = layerGroups.get(g);
            if (candidate.matches(vertexBuffer, indexBuffer, indexType)) {
                return candidate;
            }
        }
        return newGroup(layerGroups, vertexBuffer, indexBuffer, indexType);
    }

    private static Group newGroup(List<Group> layerGroups, GpuBuffer vertexBuffer, @Nullable GpuBuffer indexBuffer, @Nullable IndexType indexType) {
        Group group = POOL.isEmpty() ? new Group() : POOL.removeLast();
        group.vertexBuffer = vertexBuffer;
        group.indexBuffer = indexBuffer;
        group.indexType = indexType;
        layerGroups.add(group);
        return group;
    }

    /**
     * Packs the mesh's solid and cutout draws: their place in the uber buffers stays where it is until the mesh is
     * released (only translucent index buffers are re-uploaded under the same mesh, when resorted), and a mesh is only
     * drawn once all its buffers are uploaded.
     *
     * @return the packed draws, {@link #UNPACKABLE}, or null if a buffer isn't uploaded yet
     */
    private static int @Nullable [] pack(SectionRenderDispatcher dispatcher, SectionMesh mesh, PackedSectionDraws holder) {
        int[] packed = new int[PACKED_LAYERS * PACKED_STRIDE];
        GpuBuffer[] buffers = new GpuBuffer[PACKED_LAYERS];
        for (int l = 0; l < PACKED_LAYERS; l++) {
            ChunkSectionLayer layer = LAYERS[l];
            SectionMesh.SectionDraw draw = mesh.getSectionDraw(layer);
            if (draw == null) {
                continue;
            }
            if (draw.hasCustomIndexBuffer()) {
                holder.ruthenium$setPackedDraws(UNPACKABLE, buffers);
                return UNPACKABLE;
            }
            SectionRenderDispatcher.RenderSectionBufferSlice slice = dispatcher.getRenderSectionSlice(mesh, layer);
            if (slice == null) {
                return null;
            }
            int o = l * PACKED_STRIDE;
            packed[o] = 1;
            packed[o + 1] = draw.indexCount();
            packed[o + 2] = (int) (slice.vertexBufferOffset() / VERTEX_SIZES[l]);
            buffers[l] = slice.vertexBuffer();
            SectionFaceGroups faces = mesh instanceof FaceGroupsHolder groupsHolder ? groupsHolder.ruthenium$getFaceGroups(layer) : null;
            if (faces != null && draw.indexCount() == faces.quadCount() * 6) {
                packed[o + 3] = 1;
                faces.packInto(packed, o + 4);
            }
        }
        holder.ruthenium$setPackedDraws(packed, buffers);
        return packed;
    }

    private static EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> writeCommands(boolean respectTranslucentOrder) {
        EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> indirectDraws = emptyDraws();
        int total = count(GROUPS);

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
            commands = new MappableRingBuffer(() -> "Ruthenium terrain draw commands",
                    GpuBuffer.USAGE_INDIRECT_PARAMETERS | GpuBuffer.USAGE_MAP_WRITE, Math.max(bytes * 3 / 2, 64 * 1024));
        }

        GpuBuffer buffer = commands.currentBuffer();
        try (GpuBufferSlice.MappedView view = buffer.slice(0, bytes).map(false, true)) {
            put(GROUPS, view.data(), buffer, indirectDraws, respectTranslucentOrder);
        }
        return indirectDraws;
    }

    /**
     * The culled commands, as the vertices of {@link TerrainOcclusion}'s command pass; their draws read the buffer it
     * writes them to, at the same offsets
     */
    private static EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> writeTemplates(
            int total, boolean respectTranslucentOrder) {
        EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> indirectDraws = emptyDraws();
        if (retiredTemplates != null) {
            retiredTemplates.close();
            retiredTemplates = null;
        }
        if (templates != null) {
            templates.rotate();
        }
        if (total == 0) {
            return indirectDraws;
        }

        int bytes = total * COMMAND_BYTES;
        if (templates == null || templates.size() < bytes) {
            retiredTemplates = templates;
            templates = new MappableRingBuffer(() -> "Ruthenium culled terrain command templates",
                    GpuBuffer.USAGE_VERTEX | GpuBuffer.USAGE_MAP_WRITE, Math.max(bytes * 3 / 2, 64 * 1024));
        }

        GpuBuffer target = TerrainOcclusion.commandBuffer(total);
        try (GpuBufferSlice.MappedView view = templates.currentBuffer().slice(0, bytes).map(false, true)) {
            put(CULLED_GROUPS, view.data(), target, indirectDraws, respectTranslucentOrder);
        }
        return indirectDraws;
    }

    private static EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> emptyDraws() {
        EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> indirectDraws = new EnumMap<>(ChunkSectionLayer.class);
        for (ChunkSectionLayer layer : LAYERS) {
            indirectDraws.put(layer, new ArrayList<>());
        }
        return indirectDraws;
    }

    /** Writes the groups' commands to mapped memory, and their draws reading them from this buffer at the same offsets */
    private static void put(List<Group>[] layerGroups, ByteBuffer data, GpuBuffer drawBuffer,
                            EnumMap<ChunkSectionLayer, List<ChunkSectionsToRender.GpuMultiDrawIndexedIndirect>> indirectDraws,
                            boolean respectTranslucentOrder) {
        long address = MemoryUtil.memAddress(data);
        IntBuffer ints = data.order(ByteOrder.nativeOrder()).asIntBuffer();
        long offset = 0;
        for (ChunkSectionLayer layer : LAYERS) {
            List<Group> groups = layerGroups[layer.ordinal()];
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
                        drawBuffer.slice(start, offset - start), group.count));
            }
        }
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
