package net.ruthenium.config.option;

import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;

public enum PerformanceImpact {
    LOW(Component.translatable("ruthenium.options.performanceImpact.low").withStyle(ChatFormatting.DARK_GREEN)),
    MEDIUM(Component.translatable("ruthenium.options.performanceImpact.medium").withStyle(ChatFormatting.YELLOW)),
    HIGH(Component.translatable("ruthenium.options.performanceImpact.high").withStyle(ChatFormatting.RED));

    private final Component component;

    PerformanceImpact(Component component) {
        this.component = component;
    }

    public Component component() {
        return this.component;
    }
}
