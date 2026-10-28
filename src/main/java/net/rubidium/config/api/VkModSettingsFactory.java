package net.rubidium.config.api;

import net.rubidium.config.gui.ModSettingsEntry;

public interface VkModSettingsFactory {
    ModSettingsEntry build(VkModSettingsEntryBuilder builder);
}
