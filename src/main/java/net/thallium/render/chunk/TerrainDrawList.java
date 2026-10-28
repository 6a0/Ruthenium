package net.thallium.render.chunk;

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
import net.thallium.Initializer;
import net.thallium.interfaces.FaceGroupsHolder;
import net.thallium.interfaces.PackedSectionDraws;
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
 * without its per-frame costs: a compiled mesh's solid and cutout draws are packed once into an int array on the
 * mesh ({@link #pack}) instead of being looked up in the uber buffers' hash maps every frame, commands are collected
 * in reused int arrays instead of a record per draw, and they are written to mapped memory in one copy per group.
 * Draw groups, their order (translucent groups and draws back to front) and the commands are the same as vanilla's,
 * plus the face groups of {@link SectionFaceGroups}.
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
            float eyeX = (float) (eye.x - origin.getX()), eyeY = (float) (eye.y - origin.getY()), eyeZ = (float) (eye.z - origin.getZ());

            int[] packed = null;
            GpuBuffer[] packedBuffers = null;
            if (mesh instanceof PackedSectionDraws holder) {
                packed = holder.thallium$getPackedDraws();
                if (packed == null) {
                    packed = pack(dispatcher, mesh, holder);
                }
                if (packed == UNPACKABLE) {
                    packed = null;
                } else if (packed != null) {
                    packedBuffers = holder.thallium$getPackedBuffers();
                }
            }

            for (ChunkSectionLayer layer : LAYERS) {
                if (packed != null && layer.ordinal() < PACKED_LAYERS) {
                    int o = layer.ordinal() * PACKED_STRIDE;
                    if (packed[o] == 0) {
                        continue;
                    }
                    if (sectionInfo == -1) {
                        sectionInfo = addSectionInfo(section, origin, now, fadeDuration);
                    }
                    int indexCount = packed[o + 1];
                    largestIndexCount = Math.max(largestIndexCount, indexCount);
                    Group group = group(GROUPS[layer.ordinal()], packedBuffers[layer.ordinal()], null, null);
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
                    sectionInfo = addSectionInfo(section, origin, now, fadeDuration);
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

                Group group = null;
                if (layer.translucent() && respectTranslucentOrder) {
                    // Consecutive draws in the same buffers share a group, so the order between sections is kept
                    if (lastTranslucent != null && lastTranslucent.matches(vertexBuffer, indexBuffer, indexType)) {
                        group = lastTranslucent;
                    } else {
                        group = newGroup(GROUPS[layer.ordinal()], vertexBuffer, indexBuffer, indexType);
                    }
                } else {
                    group = group(GROUPS[layer.ordinal()], vertexBuffer, indexBuffer, indexType);
                }
                if (layer.translucent()) {
                    lastTranslucent = group;
                }

                SectionFaceGroups faces = faceCulling && !customIndices && mesh instanceof FaceGroupsHolder holder
                        ? holder.thallium$getFaceGroups(layer) : null;
                if (faces != null && draw.indexCount() == faces.quadCount() * 6) {
                    faces.addVisibleCommands(group, firstIndex, baseVertex, sectionInfo, eyeX, eyeY, eyeZ);
                } else {
                    group.add(draw.indexCount(), firstIndex, baseVertex, sectionInfo);
                }
            }
        }
        return largestIndexCount;
    }

    private static int addSectionInfo(SectionRenderDispatcher.RenderSection section, BlockPos origin, long now, long fadeDuration) {
        int index = SECTION_INFOS.size();
        SECTION_INFOS.add(new DynamicGpuData.ChunkSectionInfo(origin.getX(), origin.getY(), origin.getZ(),
                section.getVisibility(now, fadeDuration)));
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
                holder.thallium$setPackedDraws(UNPACKABLE, buffers);
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
            SectionFaceGroups faces = mesh instanceof FaceGroupsHolder groupsHolder ? groupsHolder.thallium$getFaceGroups(layer) : null;
            if (faces != null && draw.indexCount() == faces.quadCount() * 6) {
                packed[o + 3] = 1;
                faces.packInto(packed, o + 4);
            }
        }
        holder.thallium$setPackedDraws(packed, buffers);
        return packed;
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
            commands = new MappableRingBuffer(() -> "Thallium terrain draw commands",
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
