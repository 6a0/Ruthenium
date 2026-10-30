package net.ruthenium.mixin.render.entity;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.llamalad7.mixinextras.sugar.Local;
import com.mojang.blaze3d.systems.RenderSystem;
import net.fabricmc.fabric.api.client.renderer.v1.Renderer;
import net.fabricmc.fabric.api.client.renderer.v1.mesh.Mesh;
import net.fabricmc.fabric.api.client.renderer.v1.mesh.MutableMesh;
import net.fabricmc.fabric.api.client.renderer.v1.mesh.QuadEmitter;
import net.minecraft.client.renderer.block.BlockAndTintGetter;
import net.minecraft.client.renderer.block.BlockModelRenderState;
import net.minecraft.client.renderer.block.dispatch.BlockStateModel;
import net.minecraft.client.renderer.block.model.BlockStateModelWrapper;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.block.state.BlockState;
import net.ruthenium.interfaces.CachedMeshRenderState;
import net.ruthenium.render.entity.BlockModelMeshCache;
import org.joml.Matrix4fc;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;

import java.util.function.Predicate;

/**
 * Block model quads of entities from {@link BlockModelMeshCache}, submitted by the render state as they are
 * ({@code BlockModelRenderStateM}). The call is in the Fabric Renderer API's overwrite of {@code update}, so this mixin
 * is applied after it (priority); without that overwrite nothing matches and the method stays as it is.
 * <p>
 * With {@code noScratchMesh} the render state of a cached model is set up without a mesh of its own (which the
 * overwrite's {@code setupMesh} allocates for every new render state, i.e. every entity and frame, to stay empty), and
 * the model gets a shared random instead of a new one per render state. The key computed at {@code setupMesh} is kept
 * for the {@code emitQuads} call that follows it.
 */
@Mixin(value = BlockStateModelWrapper.class, priority = 1100)
public class BlockStateModelWrapperM {
    @Shadow @Final private BlockStateModel model;

    /** The state whose key {@link #setupCached} computed, the key and whether the state was set up without a mesh */
    @Unique private static @Nullable BlockModelRenderState keyedOutput;
    @Unique private static @Nullable Object keyedKey;
    @Unique private static boolean keyedSkipped;

    @WrapOperation(method = "update", require = 0, at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/block/BlockModelRenderState;scratchRandomSource(J)Lnet/minecraft/util/RandomSource;"))
    private RandomSource sharedRandom(BlockModelRenderState output, long seed, Operation<RandomSource> original) {
        if (BlockModelMeshCache.skipScratchMesh() && RenderSystem.isOnRenderThread()) {
            return BlockModelMeshCache.scratchRandom(seed);
        }
        return original.call(output, seed);
    }

    @WrapOperation(method = "update", require = 0, at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/block/BlockModelRenderState;setupMesh(Lorg/joml/Matrix4fc;Z)Lnet/fabricmc/fabric/api/client/renderer/v1/mesh/QuadEmitter;"))
    private QuadEmitter setupCached(BlockModelRenderState output, Matrix4fc transformation, boolean hasTranslucency, Operation<QuadEmitter> original,
                                    @Local(argsOnly = true) BlockState state, @Local(argsOnly = true) long seed) {
        if (!BlockModelMeshCache.skipScratchMesh() || !RenderSystem.isOnRenderThread()) {
            return original.call(output, transformation, hasTranslucency);
        }
        // The overwrite's emitQuads gets the empty level and the zero position
        Object key = BlockModelMeshCache.key(this.model, BlockAndTintGetter.EMPTY, BlockPos.ZERO, state, seed);
        Mesh mesh = key != null ? BlockModelMeshCache.get(key) : null;
        keyedOutput = output;
        keyedKey = key;
        keyedSkipped = mesh != null && ((CachedMeshRenderState) output).ruthenium$setupCachedMesh(transformation, hasTranslucency, mesh);
        if (keyedSkipped) {
            // Unused: emitQuads isn't called for a cached mesh
            return BlockModelMeshCache.scratchEmitter();
        }
        return original.call(output, transformation, hasTranslucency);
    }

    @WrapOperation(method = "update", require = 0, at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/block/dispatch/BlockStateModel;emitQuads(Lnet/fabricmc/fabric/api/client/renderer/v1/mesh/QuadEmitter;Lnet/minecraft/client/renderer/block/BlockAndTintGetter;Lnet/minecraft/core/BlockPos;Lnet/minecraft/world/level/block/state/BlockState;Lnet/minecraft/util/RandomSource;Ljava/util/function/Predicate;)V"))
    private void cachedQuads(BlockStateModel model, QuadEmitter emitter, BlockAndTintGetter level, BlockPos pos, BlockState state, RandomSource random,
                             Predicate<@Nullable Direction> cullTest, Operation<Void> original, @Local(argsOnly = true) long seed,
                             @Local(argsOnly = true) BlockModelRenderState output) {
        Object key;
        if (keyedOutput == output && level == BlockAndTintGetter.EMPTY && pos == BlockPos.ZERO) {
            keyedOutput = null;
            if (keyedSkipped) {
                // Set up with the cached mesh already
                return;
            }
            key = keyedKey;
        } else {
            keyedOutput = null;
            key = BlockModelMeshCache.key(model, level, pos, state, seed);
        }
        if (key == null) {
            ((CachedMeshRenderState) output).ruthenium$setCachedMesh(null);
            original.call(model, emitter, level, pos, state, random, cullTest);
            return;
        }

        Mesh mesh = BlockModelMeshCache.get(key);
        if (mesh == null) {
            MutableMesh mutableMesh = Renderer.get().mutableMesh();
            original.call(model, mutableMesh.emitter(), level, pos, state, random, cullTest);
            mesh = mutableMesh.immutableCopy();
            BlockModelMeshCache.put(key, mesh);
        }
        // The render state submits the cached mesh instead of a copy of its own (empty) mesh
        ((CachedMeshRenderState) output).ruthenium$setCachedMesh(mesh);
    }
}
