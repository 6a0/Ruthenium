package net.thallium.config.api;

import net.thallium.config.gui.ModSettingsEntry;

public interface VkModSettingsFactory {
    ModSettingsEntry build(VkModSettingsEntryBuilder builder);
}
