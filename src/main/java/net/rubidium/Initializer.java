package net.rubidium;

import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.loader.api.FabricLoader;
import net.rubidium.config.Config;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

public class Initializer implements ClientModInitializer {
	public static final Logger LOGGER = LogManager.getLogger("Rubidium");

	private static String VERSION;
	public static Config CONFIG;

	@Override
	public void onInitializeClient() {
		VERSION = FabricLoader.getInstance()
				.getModContainer("rubidium")
				.get()
				.getMetadata()
				.getVersion().getFriendlyString();

		LOGGER.info("== Rubidium ==");

		var configPath = FabricLoader.getInstance()
				.getConfigDir()
				.resolve("rubidium_settings.json");

		CONFIG = Config.load(configPath);
	}

	public static String getVersion() {
		return VERSION;
	}
}
