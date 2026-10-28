package net.thallium.gametest.mixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import it.unimi.dsi.fastutil.objects.ObjectArrayList;
import net.minecraft.client.renderer.DynamicGpuData;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;
import net.minecraft.util.profiling.Profiler;
import net.minecraft.util.profiling.ProfilerFiller;
import net.minecraft.world.phys.Vec3;
import net.thallium.render.chunk.TerrainDrawList;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

import java.util.EnumMap;
import java.util.List;

/** Profiler sections inside {@link TerrainDrawList#prepare} for the light profiler */
@Mixin(value = TerrainDrawList.class, remap = false)
public class TerrainDrawListProfileMixin {
    @WrapOperation(method = "prepare", at = @At(value = "INVOKE",
            target = "Lnet/thallium/render/chunk/TerrainDrawList;collect(Lnet/minecraft/client/renderer/chunk/SectionRenderDispatcher;Lit/unimi/dsi/fastutil/objects/ObjectArrayList;Lnet/minecraft/world/phys/Vec3;DZ)I"))
    private static int profileCollect(SectionRenderDispatcher dispatcher, ObjectArrayList<?> sections, Vec3 eye, double fade,
                                      boolean order, Operation<Integer> original) {
        ProfilerFiller profiler = Profiler.get();
        profiler.push("collect");
        profiler.incrementCounter("visibleSections", sections.size());
        int result = original.call(dispatcher, sections, eye, fade, order);
        profiler.pop();
        return result;
    }

    @WrapOperation(method = "prepare", at = @At(value = "INVOKE",
            target = "Lnet/thallium/render/chunk/TerrainDrawList;writeCommands(Z)Ljava/util/EnumMap;"))
    private static EnumMap<?, ?> profileWrite(boolean order, Operation<EnumMap<?, ?>> original) {
        ProfilerFiller profiler = Profiler.get();
        profiler.push("writeCommands");
        EnumMap<?, ?> result = original.call(order);
        profiler.pop();
        return result;
    }

    @WrapOperation(method = "prepare", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/DynamicGpuData;writeChunkSectionsInstanced(Ljava/util/List;)Lcom/mojang/renderpearl/api/buffers/GpuBufferSlice;"))
    private static GpuBufferSlice profileSectionInfos(DynamicGpuData data, List<?> infos, Operation<GpuBufferSlice> original) {
        ProfilerFiller profiler = Profiler.get();
        profiler.push("writeChunkSectionsInstanced");
        profiler.incrementCounter("sections", infos.size());
        GpuBufferSlice result = original.call(data, infos);
        profiler.pop();
        return result;
    }
}
