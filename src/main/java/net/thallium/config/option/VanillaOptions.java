package net.thallium.config.option;

import net.minecraft.client.OptionInstance;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.thallium.mixin.screen.OptionInstanceAccessor;
import net.thallium.mixin.screen.TooltipAccessor;

import java.util.function.Function;
import java.util.function.IntFunction;
import java.util.function.ToIntFunction;

/**
 * Options backed by vanilla {@link OptionInstance}s. Setting the instance runs vanilla's change listener (graphics
 * preset, level reload, sampler reset, ...), and names, value labels and tooltips are vanilla's, so they follow
 * the game's translations.
 */
public final class VanillaOptions {

    private VanillaOptions() {
    }

    public static SwitchOption toggle(OptionInstance<Boolean> instance) {
        var option = new SwitchOption(caption(instance), instance::set, instance::get);
        option.setTooltip(tooltip(instance));
        return option;
    }

    public static <T> CyclingOption<T> cycling(OptionInstance<T> instance, T[] values) {
        var option = new CyclingOption<>(caption(instance), values, instance::set, instance::get);
        option.setTranslator(label(instance));
        option.setTooltip(tooltip(instance));
        return option;
    }

    public static RangeOption range(OptionInstance<Integer> instance, int min, int max, int step) {
        return range(instance, min, max, step, value -> value, value -> value);
    }

    /**
     * A slider over the integers {@code min..max} that map to the instance's values.
     */
    public static <T> RangeOption range(OptionInstance<T> instance, int min, int max, int step, IntFunction<T> toValue,
                                        ToIntFunction<T> fromValue) {
        Function<T, Component> label = label(instance);
        Function<T, Component> tooltip = tooltip(instance);
        var option = new RangeOption(caption(instance), min, max, step,
                value -> label.apply(toValue.apply(value)),
                value -> instance.set(toValue.apply(value)),
                () -> fromValue.applyAsInt(instance.get()));
        option.setTooltip(value -> tooltip.apply(toValue.apply(value)));
        return option;
    }

    @SuppressWarnings("unchecked")
    public static Component caption(OptionInstance<?> instance) {
        return ((OptionInstanceAccessor<Object>) (Object) instance).thallium$getCaption();
    }

    /**
     * Vanilla's value label without the leading caption, which the widget already shows.
     */
    @SuppressWarnings("unchecked")
    public static <T> Function<T, Component> label(OptionInstance<T> instance) {
        Function<T, Component> label = ((OptionInstanceAccessor<T>) (Object) instance).thallium$getValueLabel();
        return value -> stripCaption(label.apply(value));
    }

    @SuppressWarnings("unchecked")
    public static <T> Function<T, Component> tooltip(OptionInstance<T> instance) {
        OptionInstance.TooltipSupplier<T> supplier = ((OptionInstanceAccessor<T>) (Object) instance).thallium$getTooltip();
        return value -> {
            Tooltip tooltip = supplier.apply(value);
            return tooltip != null ? ((TooltipAccessor) tooltip).thallium$getMessage() : null;
        };
    }

    private static Component stripCaption(Component label) {
        if (label.getContents() instanceof TranslatableContents contents && contents.getArgs().length == 2) {
            Object value = contents.getArgs()[1];
            Component valueComponent = value instanceof Component component ? component : Component.literal(String.valueOf(value));

            return switch (contents.getKey()) {
                case "options.generic_value" -> valueComponent;
                case "options.percent_value" -> Component.translatable("thallium.options.percent", valueComponent);
                case "options.pixel_value" -> Component.translatable("thallium.options.pixels", valueComponent);
                default -> label;
            };
        }
        return label;
    }
}
