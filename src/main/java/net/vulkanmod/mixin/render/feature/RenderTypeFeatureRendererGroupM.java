package net.vulkanmod.mixin.render.feature;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import net.vulkanmod.Initializer;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Finds a group's existing draw for a prepared render type with a hash lookup instead of vanilla's linear
 * {@code indexOf} over the group's draws, whose record {@code equals} is deep. The map holds the first index of each
 * prepared type, so it returns the draw {@code indexOf} would.
 */
@Mixin(targets = "net.minecraft.client.renderer.feature.RenderTypeFeatureRenderer$Group")
public class RenderTypeFeatureRendererGroupM {
    @Shadow @Final private List<PreparedRenderType> drawRenderTypes;

    /** Read once per group, so the map always covers the whole list when it is used */
    @Unique private final boolean hashedLookup = Initializer.CONFIG.hashedDrawLookup;
    @Unique private final Map<PreparedRenderType, Integer> drawIndices = new HashMap<>();

    @WrapOperation(method = "getOrAddDraw", at = @At(value = "INVOKE", target = "Ljava/util/List;indexOf(Ljava/lang/Object;)I"))
    private int lookUpDraw(List<?> list, Object renderType, Operation<Integer> original) {
        if (!this.hashedLookup) {
            return original.call(list, renderType);
        }
        Integer index = this.drawIndices.get(renderType);
        return index != null ? index : -1;
    }

    @WrapOperation(method = "getOrAddDraw", at = @At(value = "INVOKE", target = "Ljava/util/List;add(Ljava/lang/Object;)Z"))
    private boolean addDraw(List<?> list, Object element, Operation<Boolean> original) {
        if (this.hashedLookup && list == this.drawRenderTypes) {
            this.drawIndices.putIfAbsent((PreparedRenderType) element, list.size());
        }
        return original.call(list, element);
    }
}
