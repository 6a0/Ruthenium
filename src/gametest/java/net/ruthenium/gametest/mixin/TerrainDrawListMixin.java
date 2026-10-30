package net.ruthenium.gametest.mixin;

import it.unimi.dsi.fastutil.objects.ObjectArrayList;
import net.minecraft.client.renderer.DynamicGpuData;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;
import net.minecraft.world.phys.Vec3;
import net.ruthenium.gametest.GpuProbe;
import net.ruthenium.gametest.OcclusionProbe;
import net.ruthenium.render.chunk.TerrainDrawList;
import org.joml.Matrix4fc;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import java.util.List;

@Mixin(value = TerrainDrawList.class, remap = false)
public class TerrainDrawListMixin {
    @Shadow @Final private static List[] GROUPS;
    @Shadow @Final private static List<DynamicGpuData.ChunkSectionInfo> SECTION_INFOS;

    @Inject(method = "prepare", at = @At("HEAD"))
    private static void beginOcclusionProbe(SectionRenderDispatcher dispatcher, ObjectArrayList<SectionRenderDispatcher.RenderSection> visibleSections,
                                            Vec3 eye, double fadeInSeconds, Matrix4fc modelView, int atlasWidth, int atlasHeight,
                                            boolean respectTranslucentOrder, CallbackInfoReturnable<?> cir) {
        if (OcclusionProbe.ENABLED) {
            OcclusionProbe.beginFrame(eye, modelView);
        }
    }

    /** Before the draw count below, so it counts what is drawn */
    @Inject(method = "writeCommands", at = @At("HEAD"))
    private static void occlusionProbe(boolean respectTranslucentOrder, CallbackInfoReturnable<?> cir) {
        if (OcclusionProbe.ENABLED) {
            for (ChunkSectionLayer layer : ChunkSectionLayer.values()) {
                OcclusionProbe.onCommands(layer, GROUPS[layer.ordinal()], SECTION_INFOS);
            }
        }
    }

    @Inject(method = "writeCommands", at = @At("HEAD"))
    private static void countDraws(boolean respectTranslucentOrder, CallbackInfoReturnable<?> cir) {
        if (!GpuProbe.ENABLED) {
            return;
        }
        GpuProbe.countFrame();
        for (ChunkSectionLayer layer : ChunkSectionLayer.values()) {
            for (Object group : GROUPS[layer.ordinal()]) {
                TerrainDrawListGroupAccessor accessor = (TerrainDrawListGroupAccessor) group;
                GpuProbe.countDraws(layer, accessor.getCommands(), accessor.getCount());
            }
        }
    }
}
