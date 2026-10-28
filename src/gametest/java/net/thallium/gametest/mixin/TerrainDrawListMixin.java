package net.thallium.gametest.mixin;

import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.thallium.gametest.GpuProbe;
import net.thallium.render.chunk.TerrainDrawList;
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
