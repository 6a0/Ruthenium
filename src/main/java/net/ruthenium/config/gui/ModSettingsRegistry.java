package net.ruthenium.config.gui;

import it.unimi.dsi.fastutil.objects.ObjectArraySet;
import net.fabricmc.loader.api.FabricLoader;
import net.fabricmc.loader.api.entrypoint.EntrypointContainer;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.ruthenium.Initializer;
import net.ruthenium.config.api.VkModSettingsEntryBuilder;
import net.ruthenium.config.api.VkModSettingsFactory;
import net.ruthenium.config.gui.util.VGuiConstants;
import net.ruthenium.config.option.Options;

import java.util.Set;

public class ModSettingsRegistry {
    private static final String CONFIG_ENTRY_POINT_KEY = "ruthenium:mod_settings_registry";

    public static final ModSettingsRegistry INSTANCE = new ModSettingsRegistry();

    private final Set<ModSettingsEntry> modEntries = new ObjectArraySet<>();

    ModSettingsRegistry() {
        ModSettingsEntry rutheniumSettings = new ModSettingsEntry(Component.literal("Ruthenium").withColor(VGuiConstants.PRIMARY_RGB),
                                                                  () -> Identifier.fromNamespaceAndPath("ruthenium", "vlogo_transparent.png"),
                                                                  Options::getOptionPages,
                                                                  () -> Initializer.CONFIG.write());
        this.addModEntry(rutheniumSettings);

        // build and add ruthenium settings entrypoints
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
