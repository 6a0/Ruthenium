package net.rubidium.render.entity;

import com.mojang.blaze3d.systems.RenderSystem;
import net.fabricmc.fabric.api.client.renderer.v1.mesh.Mesh;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.block.BlockAndTintGetter;
import net.minecraft.client.renderer.block.ModelBlockRenderer;
import net.minecraft.client.renderer.block.dispatch.BlockStateModel;
import net.minecraft.core.BlockPos;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.block.state.BlockState;
import net.rubidium.Initializer;
import org.jspecify.annotations.Nullable;

import java.util.HashMap;
import java.util.Map;

/**
 * The quads of block models that entities render (item frames, block displays, minecart blocks, endermen's blocks),
 * kept across frames. With the Fabric Renderer API, {@code BlockStateModelWrapper.update} emits the model's quads into
 * a new mesh every time it is called, i.e. for every such entity and frame, decoding and encoding each baked quad.
 * Here a model's output is emitted once into a mesh of its own, which the render state then submits itself
 * ({@code BlockModelRenderStateM}) instead of copying it into its own mesh and that into a new one per submit.
 * <p>
 * The cache is keyed by the model's geometry key ({@code FabricBlockStateModel.createGeometryKey}), which by its
 * contract determines what {@code emitQuads} outputs for the same level, position, state and random and a cull test
 * that culls nothing, as {@code update} passes. Models without a key (custom models may not have one) are emitted as
 * usual. Fabric's default {@code emitQuads} also moves the quads of leaves to the solid layer when cutout leaves are
 * off, which the key doesn't cover, so that is part of the cache key too. Only used on the render thread.
 */
public final class BlockModelMeshCache {
    /** Bounds the cache if keys keep changing (they are model instances or records of them) */
    private static final int MAX_ENTRIES = 4096;

    private static final Map<Object, Mesh> MESHES = new HashMap<>();
    private static @Nullable RandomSource keyRandom;

    private BlockModelMeshCache() {
    }

    public static void clear() {
        MESHES.clear();
    }

    /**
     * @param seed the seed of the random {@code emitQuads} gets
     * @return the cache key of what {@code model.emitQuads} outputs for these arguments, or null if it isn't cached
     */
    public static @Nullable Object key(BlockStateModel model, BlockAndTintGetter level, BlockPos pos, BlockState state, long seed) {
        if (!Initializer.CONFIG.blockModelMeshCache || !RenderSystem.isOnRenderThread()) {
            return null;
        }

        // The key consumes randomness like emitQuads does, so it gets a random of its own with the same seed
        RandomSource random = keyRandom;
        if (random == null) {
            random = keyRandom = RandomSource.create(seed);
        } else {
            random.setSeed(seed);
        }
        Object key = model.createGeometryKey(level, pos, state, random);
        if (key != null && ModelBlockRenderer.forceOpaque(Minecraft.getInstance().options.cutoutLeaves().get(), state)) {
            return new OpaqueKey(key);
        }
        return key;
    }

    public static @Nullable Mesh get(Object key) {
        return MESHES.get(key);
    }

    public static void put(Object key, Mesh mesh) {
        if (MESHES.size() < MAX_ENTRIES) {
            MESHES.put(key, mesh);
        }
    }

    /** Quads of a model whose quads were forced to the solid layer */
    private record OpaqueKey(Object key) {
    }
}
