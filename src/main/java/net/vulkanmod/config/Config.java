package net.vulkanmod.config;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonParseException;
import net.vulkanmod.Initializer;

import java.io.IOException;
import java.io.Reader;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.nio.file.Files;
import java.nio.file.Path;

public class Config {
    /** Per block entity frustum culling, on top of vanilla's per section culling */
    public boolean blockEntityCulling = true;
    /** Closed chest geometry kept on the GPU across frames */
    public boolean retainedChests = true;
    /** Closed enchanting table books and armor stands drawn as GPU instances */
    public boolean instancedModels = true;
    /** CPU skip of model faces and sign text the GPU would cull as back faces */
    public boolean modelBackFaceCulling = true;
    /** Particles dropped at spawn when their group can't take them */
    public boolean particleSpawnLimit = true;
    /** Skips block entity ticks that would do nothing */
    public boolean skipIdleTicks = true;
    /** Skips the terrain faces of a section that all face away from the camera */
    public boolean terrainFaceCulling = true;
    /** Solid and cutout terrain vertices in 20 instead of 28 bytes */
    public boolean compressedTerrain = true;

    private static Path CONFIG_PATH;

    private static final Gson GSON = new GsonBuilder()
            .setPrettyPrinting()
            .excludeFieldsWithModifiers(Modifier.PRIVATE, Modifier.STATIC, Modifier.TRANSIENT)
            .create();

    public void write() {
        try {
            Files.createDirectories(CONFIG_PATH.getParent());
            Files.writeString(CONFIG_PATH, GSON.toJson(this));
        } catch (IOException e) {
            Initializer.LOGGER.error("Failed to write {}", CONFIG_PATH, e);
        }
    }

    public static Config load(Path path) {
        Config.CONFIG_PATH = path;

        Config config = null;
        if (Files.exists(path)) {
            try (Reader reader = Files.newBufferedReader(path)) {
                config = GSON.fromJson(reader, Config.class);
            } catch (IOException | JsonParseException e) {
                Initializer.LOGGER.error("Failed to read {}, using defaults", path, e);
            }
        }

        if (config == null) {
            config = new Config();
            config.write();
        }

        config.applyOverrides();
        return config;
    }

    /**
     * {@code -Dvulkanmod.<field>=true|false} overrides a switch for this launch (used by the benchmark tasks).
     */
    private void applyOverrides() {
        for (Field field : Config.class.getDeclaredFields()) {
            int modifiers = field.getModifiers();
            if (field.getType() != boolean.class || Modifier.isStatic(modifiers) || Modifier.isPrivate(modifiers)) {
                continue;
            }

            String value = System.getProperty("vulkanmod." + field.getName());
            if (value == null) {
                continue;
            }

            try {
                field.setBoolean(this, Boolean.parseBoolean(value));
                Initializer.LOGGER.info("Config override: {} = {}", field.getName(), field.getBoolean(this));
            } catch (IllegalAccessException e) {
                throw new IllegalStateException(e);
            }
        }
    }
}
