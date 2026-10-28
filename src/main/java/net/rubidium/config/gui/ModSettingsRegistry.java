package net.rubidium.config.gui;

import it.unimi.dsi.fastutil.objects.ObjectArraySet;
import net.fabricmc.loader.api.FabricLoader;
import net.fabricmc.loader.api.entrypoint.EntrypointContainer;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.rubidium.Initializer;
import net.rubidium.config.api.VkModSettingsEntryBuilder;
import net.rubidium.config.api.VkModSettingsFactory;
import net.rubidium.config.gui.util.VGuiConstants;
import net.rubidium.config.option.Options;

import java.util.Set;

public class ModSettingsRegistry {
    private static final String CONFIG_ENTRY_POINT_KEY = "rubidium:mod_settings_registry";

    public static final ModSettingsRegistry INSTANCE = new ModSettingsRegistry();

    private final Set<ModSettingsEntry> modEntries = new ObjectArraySet<>();

    ModSettingsRegistry() {
        ModSettingsEntry rubidiumSettings = new ModSettingsEntry(Component.literal("Rubidium").withColor(VGuiConstants.PRIMARY_RGB),
                                                                  () -> Identifier.fromNamespaceAndPath("rubidium", "vlogo_transparent.png"),
                                                                  Options::getOptionPages,
                                                                  () -> Initializer.CONFIG.write());
        this.addModEntry(rubidiumSettings);

        // build and add rubidium settings entrypoints
        var entryPointContainers = FabricLoader.getInstance().getEntrypointContainers(CONFIG_ENTRY_POINT_KEY, VkModSettingsFactory.class);
        for (EntrypointContainer<VkModSettingsFactory> entryPointContainer : entryPointContainers) {
            VkModSettingsFactory modSettingsFactory = entryPointContainer.getEntrypoint();
            VkModSettingsEntryBuilder builder = new VkModSettingsEntryBuilder();

            this.addModEntry(modSettingsFactory.build(builder));
        }
    }

    public void addModEntry(ModSettingsEntry entry) {
        this.modEntries.add(entry);
    }

    public Set<ModSettingsEntry> getModEntries() {
        return modEntries;
    }
}
