package net.thallium.mixin.render.text;

import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.level.block.entity.SignText;
import net.thallium.render.text.SignTextCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import java.util.function.Function;

/**
 * Wraps each render line once, when SignText computes it, so that it carries its own {@link SignTextCache} entry.
 */
@Mixin(SignText.class)
public class SignTextM {

    @Redirect(method = "getRenderMessages", at = @At(value = "INVOKE",
            target = "Ljava/util/function/Function;apply(Ljava/lang/Object;)Ljava/lang/Object;"))
    private Object wrapLine(Function<Object, Object> function, Object component) {
        return SignTextCache.wrap((FormattedCharSequence) function.apply(component));
    }
}
