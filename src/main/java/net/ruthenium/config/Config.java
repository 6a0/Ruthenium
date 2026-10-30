package net.ruthenium.config;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonParseException;
import net.ruthenium.Initializer;

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
    /** Retained block entity batches whose section's blocks or light changed are rebuilt only if their block entities' states or light did */
    public boolean retainedChangeChecks = true;
    /** Signs with glowing text are retained too (with retainedChests); those vanilla outlines near the camera take its path while it does */
    public boolean retainedGlowingText = true;
    /** Closed enchanting table books and armor stands drawn as GPU instances */
    public boolean instancedModels = true;
    /** An armor stand's base plate, which vanilla turns by the stand's yaw, is an instance of its own (with instancedModels) */
    public boolean armorStandPlates = true;
    /** CPU skip of model faces and sign text the GPU would cull as back faces */
    public boolean modelBackFaceCulling = true;
    /** CPU skip of model faces whose texels all draw nothing (alpha cutout, or alpha 0 with translucent blending) */
    public boolean transparentFaceCulling = true;
    /** Particles dropped at spawn when their group can't take them */
    public boolean particleSpawnLimit = true;
    /** Skips block entity ticks that would do nothing */
    public boolean skipIdleTicks = true;
    /** Idle block entity tickers leave the client's ticker list until something wakes them (with skipIdleTicks) */
    public boolean parkIdleTickers = true;
    /** A ticking block entity checks its block state once per state instead of every tick (client) */
    public boolean cacheTickerStates = true;
    /** The client's enchanting tables are ticked in one loop with the player search done once per tick (with parking and cacheTickerStates) */
    public boolean batchTableTicks = true;
    /** Entity frustum culling without allocations or renderer lookups for renderers that keep vanilla's culling */
    public boolean fastEntityCulling = true;
    /** Entities keep the block and sky light at their light probe position until it moves or light data changes */
    public boolean entityLightCache = true;
    /** Entity collision queries skip entity sections without entities that can be collided with (boats, shulkers) */
    public boolean fastEntityCollisions = true;
    /** The block animation tick steps its random seed without atomics and skips biome lookups where no biome has ambient particles */
    public boolean fastAnimateTick = true;
    /** Client boat ticks: full cube ground friction tests from boxes, push searches for the local player only */
    public boolean fastBoatTicks = true;
    /** Resting entities keep their supporting block while no block of the client level changes */
    public boolean fastSupportingBlocks = true;
    /** Resting entities keep the block collision answer of their move while no block of the client level near them changes */
    public boolean fastRestingCollisions = true;
    /** Name tags equal to the ones an entity showed last keep their width, visual order and prepared glyphs */
    public boolean nameTagTextCache = true;
    /** Text display lines keep their prepared glyphs across frames */
    public boolean textDisplayTextCache = true;
    /** Text displays without a culling size are frustum culled by the extent of their text */
    public boolean textDisplayCulling = true;
    /** Sorted translucent draws start their quad sort from their order in the previous frame */
    public boolean reuseSortOrder = true;
    /** Player bodies are drawn from a part-space mesh and per-frame part poses instead of writing their vertices */
    public boolean playerPoseDraws = true;
    /** Clouds read their faces from a texture instead of vanilla's texel buffer (which costs every submit on some drivers) */
    public boolean cloudFaceTexture = true;
    /** Full size screenshots (F2, a new world's icon) copy the read back frame a row at a time instead of a pixel at a time */
    public boolean fastScreenshots = true;
    /** Name tags and text display lines are drawn from a label-space mesh of their glyphs and a per-frame pose */
    public boolean retainedLabels = true;
    /** Armor stands with empty equipment skip their (vanilla) layers and the model pose for them */
    public boolean emptyArmorStandLayers = true;
    /** Two-argument render type lookups (entityCutout, ...) check a small cache of vanilla's answers before vanilla's memo */
    public boolean renderTypeCache = true;
    /** Humanoids that aren't using an item skip the enchantment lookup of the crossbow charge time (vanilla's answer for an empty hand) */
    public boolean emptyCrossbowCharge = true;
    /** Armor stands extracted into render states reused across frames */
    public boolean reuseEntityStates = true;
    /** Block models of entities (item frames, block displays, ...) keep their quads across frames */
    public boolean blockModelMeshCache = true;
    /** Render states of cached block models get no mesh of their own (the Fabric Renderer API's, which stays empty) */
    public boolean noScratchMesh = true;
    /** Model parts render their children from an array instead of an iterator of their children map per render */
    public boolean partChildArrays = true;
    /** Submits take their pose copies from a pool reused from the next frame on instead of new ones per submit */
    public boolean pooledSubmitPoses = true;
    /** Entities with the default light probe (their eye) find their light without a new vector and block position per frame */
    public boolean noAllocLightProbe = true;
    /** Name tags of entities whose display name is built from the same objects as last frame are not built again */
    public boolean keepDisplayNames = true;
    /** Finds the draw of a render type with a hash lookup instead of a linear search */
    public boolean hashedDrawLookup = true;
    /** Geometry of equal render types joins one draw in groups that may be reordered */
    public boolean mergeDraws = true;
    /** The parts of posed world models (mobs, ...) are drawn as GPU instances instead of writing their vertices */
    public boolean instancedParts = true;
    public boolean instancedArmor = true;
    public boolean instancedItems = true;
    public boolean instancedBlockModels = true;
    /** Layers that submit a model again for the same render state reuse the part poses of its first submit */
    public boolean reuseLayerPoses = true;
    /** Baked quads (items, block models) are written into vertex buffers a whole quad at a time */
    public boolean directBakedQuads = true;
    /** Skips the terrain faces of a section that all face away from the camera */
    public boolean terrainFaceCulling = true;
    /** Solid and cutout terrain vertices in 20 instead of 28 bytes */
    public boolean compressedTerrain = true;
    /** Biome tint blending computes each column's sample once per section instead of once per block */
    public boolean biomeBlendCache = true;
    /** Smooth lighting keeps the light of the section being compiled in arrays */
    public boolean sectionLightCache = true;
    /** Section compiles read the block states around each block from an array */
    public boolean sectionStateCache = true;

    /** The vignette is skipped while its color is black (its blend then leaves every pixel as it is) */
    public boolean skipBlackVignette = true;
    /** Instanced models of closed opaque boxes (armor stands) are drawn with back faces culled while the eye is outside them */
    public boolean closedModelCulling = true;
    /** Retained block entity models are drawn per 8x8x8 block cell of their section, and cells outside the view are skipped */
    public boolean retainedCellCulling = true;

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
     * {@code -Druthenium.<field>=true|false} overrides a switch for this launch (used by the benchmark tasks).
     */
    private void applyOverrides() {
        for (Field field : Config.class.getDeclaredFields()) {
            int modifiers = field.getModifiers();
            if (field.getType() != boolean.class || Modifier.isStatic(modifiers) || Modifier.isPrivate(modifiers)) {
                continue;
            }

            String value = System.getProperty("ruthenium." + field.getName());
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
