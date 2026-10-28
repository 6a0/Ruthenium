package net.thallium.config.gui;

import it.unimi.dsi.fastutil.objects.ObjectArraySet;
import net.fabricmc.loader.api.FabricLoader;
import net.fabricmc.loader.api.entrypoint.EntrypointContainer;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.thallium.Initializer;
import net.thallium.config.api.VkModSettingsEntryBuilder;
import net.thallium.config.api.VkModSettingsFactory;
import net.thallium.config.gui.util.VGuiConstants;
import net.thallium.config.option.Options;

import java.util.Set;

public class ModSettingsRegistry {
    private static final String CONFIG_ENTRY_POINT_KEY = "thallium:mod_settings_registry";

    public static final ModSettingsRegistry INSTANCE = new ModSettingsRegistry();

    private final Set<ModSettingsEntry> modEntries = new ObjectArraySet<>();

    ModSettingsRegistry() {
        ModSettingsEntry thalliumSettings = new ModSettingsEntry(Component.literal("Thallium").withColor(VGuiConstants.PRIMARY_RGB),
                                                                  () -> Identifier.fromNamespaceAndPath("thallium", "vlogo_transparent.png"),
                                                                  Options::getOptionPages,
                                                                  () -> Initializer.CONFIG.write());
        this.addModEntry(thalliumSettings);

        // build and add thallium settings entrypoints
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
