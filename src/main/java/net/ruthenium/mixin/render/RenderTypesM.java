package net.ruthenium.mixin.render;

import com.llamalad7.mixinextras.injector.ModifyExpressionValue;
import net.minecraft.client.renderer.rendertype.RenderTypes;
import net.ruthenium.render.RenderTypeFrontCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

import java.util.function.BiFunction;

/** The two-argument render type memos get a front cache ({@link RenderTypeFrontCache}) */
@Mixin(RenderTypes.class)
public class RenderTypesM {
    @ModifyExpressionValue(method = "<clinit>", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/util/Util;memoize(Ljava/util/function/BiFunction;)Ljava/util/function/BiFunction;"))
    private static BiFunction<Object, Object, Object> ruthenium$frontCache(BiFunction<Object, Object, Object> memo) {
        return new RenderTypeFrontCache<>(memo);
    }
}
