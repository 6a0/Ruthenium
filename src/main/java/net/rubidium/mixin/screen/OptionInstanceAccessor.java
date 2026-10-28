package net.rubidium.mixin.screen;

import net.minecraft.client.OptionInstance;
import net.minecraft.network.chat.Component;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import java.util.function.Function;

@Mixin(OptionInstance.class)
public interface OptionInstanceAccessor<T> {

    @Accessor("caption")
    Component rubidium$getCaption();

    /** The value label, usually "caption: value" */
    @Accessor("toString")
    Function<T, Component> rubidium$getValueLabel();

    @Accessor("tooltip")
    OptionInstance.TooltipSupplier<T> rubidium$getTooltip();
}
