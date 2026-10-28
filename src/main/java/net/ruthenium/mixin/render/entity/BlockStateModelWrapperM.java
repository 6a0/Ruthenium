package net.ruthenium.mixin.render.entity;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.llamalad7.mixinextras.sugar.Local;
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
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

import java.util.function.Predicate;

/**
 * Block model quads of entities from {@link BlockModelMeshCache}, submitted by the render state as they are
 * ({@code BlockModelRenderStateM}). The call is in the Fabric Renderer API's overwrite of {@code update}, so this mixin
 * is applied after it (priority); without that overwrite nothing matches and the method stays as it is.
 */
@Mixin(value = BlockStateModelWrapper.class, priority = 1100)
public class BlockStateModelWrapperM {

    @WrapOperation(method = "update", require = 0, at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/block/dispatch/BlockStateModel;emitQuads(Lnet/fabricmc/fabric/api/client/renderer/v1/mesh/QuadEmitter;Lnet/minecraft/client/renderer/block/BlockAndTintGetter;Lnet/minecraft/core/BlockPos;Lnet/minecraft/world/level/block/state/BlockState;Lnet/minecraft/util/RandomSource;Ljava/util/function/Predicate;)V"))
    private void cachedQuads(BlockStateModel model, QuadEmitter emitter, BlockAndTintGetter level, BlockPos pos, BlockState state, RandomSource random,
                             Predicate<@Nullable Direction> cullTest, Operation<Void> original, @Local(argsOnly = true) long seed,
                             @Local(argsOnly = true) BlockModelRenderState output) {
        Object key = BlockModelMeshCache.key(model, level, pos, state, seed);
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
