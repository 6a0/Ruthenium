package net.ruthenium;

import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.loader.api.FabricLoader;
import net.ruthenium.config.Config;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

public class Initializer implements ClientModInitializer {
	public static final Logger LOGGER = LogManager.getLogger("Ruthenium");

	private static String VERSION;
	public static Config CONFIG;

	@Override
	public void onInitializeClient() {
		VERSION = FabricLoader.getInstance()
				.getModContainer("ruthenium")
				.get()
				.getMetadata()
				.getVersion().getFriendlyString();

		LOGGER.info("== Ruthenium ==");

		var configPath = FabricLoader.getInstance()
				.getConfigDir()
				.resolve("ruthenium_settings.json");

		CONFIG = Config.load(configPath);

		LoaderManifests.trim();
	}

	public static String getVersion() {
		return VERSION;
	}
}
