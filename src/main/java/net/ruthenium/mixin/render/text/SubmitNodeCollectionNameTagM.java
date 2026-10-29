package net.ruthenium.mixin.render.text;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.gui.Font;
import net.minecraft.client.renderer.SubmitNodeCollection;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FormattedText;
import net.minecraft.util.FormattedCharSequence;
import net.ruthenium.render.text.EntityTextCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/** A name tag's width and visual order kept on its component ({@link EntityTextCache}) */
@Mixin(SubmitNodeCollection.class)
public class SubmitNodeCollectionNameTagM {

    @WrapOperation(method = "submitNameTag", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/gui/Font;width(Lnet/minecraft/network/chat/FormattedText;)I"))
    private int ruthenium$keptWidth(Font font, FormattedText text, Operation<Integer> original) {
        int width = EntityTextCache.keptWidth(font, text);
        if (width < 0) {
            width = original.call(font, text);
            EntityTextCache.keepWidth(font, text, width);
        }
        return width;
    }

    @WrapOperation(method = "submitNameTag", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/network/chat/Component;getVisualOrderText()Lnet/minecraft/util/FormattedCharSequence;"))
    private FormattedCharSequence ruthenium$keptVisualOrder(Component component, Operation<FormattedCharSequence> original) {
        return EntityTextCache.visualOrder(component, original.call(component));
    }
}
