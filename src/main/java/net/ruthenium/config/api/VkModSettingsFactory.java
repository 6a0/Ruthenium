package net.ruthenium.config.api;

import net.ruthenium.config.gui.ModSettingsEntry;

public interface VkModSettingsFactory {
    ModSettingsEntry build(VkModSettingsEntryBuilder builder);
}
