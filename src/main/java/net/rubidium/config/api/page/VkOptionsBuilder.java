package net.rubidium.config.api.page;

import net.rubidium.config.option.Option;

import java.util.ArrayList;
import java.util.List;

public class VkOptionsBuilder {
    private final VkPageOptionsBuilder parent;
    private final List<Option<?>> options = new ArrayList<>();

    public VkOptionsBuilder(VkPageOptionsBuilder parent) {
        this.parent = parent;
    }

    public <T> VkOptionsBuilder addOption(Option<T> option) {
        options.add(option);
        return this;
    }

    public VkPageOptionsBuilder finish() {
         return parent;
    }

    public Option<?>[] build() {
        Option<?>[] optionsArray = new Option<?>[options.size()];
        options.toArray(optionsArray);
        return optionsArray;
    }
}
