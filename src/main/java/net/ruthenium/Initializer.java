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

		config();

		LoaderManifests.trim();
	}

	/**
	 * The config, loaded on first use: startup mixins that run before the mod initializer (in the game's main method
	 * and the bootstrap) read their switches through this.
	 */
	public static synchronized Config config() {
		if (CONFIG == null) {
			CONFIG = Config.load(FabricLoader.getInstance().getConfigDir().resolve("ruthenium_settings.json"));
		}
		return CONFIG;
	}

	public static String getVersion() {
		return VERSION;
	}
}
