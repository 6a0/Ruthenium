package net.ruthenium.config.option;

import com.mojang.blaze3d.platform.Monitor;
import com.mojang.blaze3d.platform.VideoMode;
import com.mojang.blaze3d.platform.Window;
import net.minecraft.client.*;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ParticleStatus;
import net.ruthenium.Initializer;
import net.ruthenium.config.Config;
import net.ruthenium.config.gui.OptionBlock;
import net.ruthenium.interfaces.SectionDispatcherReset;
import net.ruthenium.interfaces.TickerParkingLevel;
import net.ruthenium.render.blockentity.InstancedModels;
import net.ruthenium.render.entity.EntityLight;
import net.ruthenium.render.entity.InstancedParts;
import net.ruthenium.render.entity.PlayerPoseDraws;
import net.ruthenium.render.text.RetainedLabels;
import net.ruthenium.render.entity.TransparentFaces;
import net.ruthenium.render.blockentity.RetainedBlockEntities;
import net.ruthenium.render.entity.BlockModelMeshCache;

import java.util.ArrayList;
import java.util.List;
import java.util.Optional;

public abstract class Options {

    private static final Config config = Initializer.CONFIG;
    private static final Minecraft minecraft = Minecraft.getInstance();
    private static final net.minecraft.client.Options mcOptions = minecraft.options;

    public static List<OptionPage> getOptionPages() {
        List<OptionPage> optionPages = new ArrayList<>();

        OptionPage page = new OptionPage(
                Component.translatable("ruthenium.options.pages.video").getString(),
                Options.getVideoOpts()
        );
        optionPages.add(page);

        page = new OptionPage(
                Component.translatable("ruthenium.options.pages.graphics").getString(),
                Options.getGraphicsOpts()
        );
        optionPages.add(page);

        page = new OptionPage(
                Component.translatable("ruthenium.options.pages.optimizations").getString(),
                Options.getOptimizationOpts()
        );
        optionPages.add(page);

        return optionPages;
    }

    public static OptionBlock[] getVideoOpts() {
        Window window = minecraft.getWindow();

        var windowModeOption = new CyclingOption<>(Component.translatable("ruthenium.options.windowMode"),
                                                   WindowMode.values(),
                                                   value -> {
                                                       if (value == WindowMode.WINDOWED) {
                                                           // Keeps the exclusive preference for F11
                                                           mcOptions.fullscreen().set(false);
                                                       } else {
                                                           // Exclusive first, so entering fullscreen picks the right mode
                                                           mcOptions.exclusiveFullscreen().set(value == WindowMode.EXCLUSIVE_FULLSCREEN);
                                                           mcOptions.fullscreen().set(true);
                                                       }
                                                   },
                                                   WindowMode::current)
                .setTranslator(value -> Component.translatable(value.translationKey))
                .setTooltip(value -> switch (value) {
                    case WINDOWED -> null;
                    case BORDERLESS_FULLSCREEN -> Component.translatable("options.exclusiveFullscreen.off.tooltip");
                    case EXCLUSIVE_FULLSCREEN -> Component.translatable("options.exclusiveFullscreen.on.tooltip");
                });

        // The fullscreen video mode of the monitor the window is on, -1 for the monitor's current mode
        Monitor monitor = window.findBestMonitor();
        int modeCount = monitor != null ? monitor.modeCount() : 0;
        var videoModeOption = new RangeOption(Component.translatable("options.fullscreen.exclusive.mode"),
                                              -1, Math.max(modeCount - 1, 0), 1,
                                              value -> {
                                                  if (monitor == null) {
                                                      return Component.translatable("options.fullscreen.unavailable");
                                                  }
                                                  if (value < 0 || value >= modeCount) {
                                                      return Component.translatable("options.fullscreen.current");
                                                  }
                                                  VideoMode mode = monitor.mode(value);
                                                  return Component.literal("%dx%d@%s".formatted(mode.getWidth(), mode.getHeight(), mode.refreshRateLabel()));
                                              },
                                              value -> {
                                                  if (monitor != null) {
                                                      window.setPreferredFullscreenVideoMode(value < 0 || value >= modeCount
                                                              ? Optional.empty()
                                                              : Optional.of(monitor.mode(value)));
                                                      window.changeFullscreenVideoMode();
                                                  }
                                              },
                                              () -> monitor == null ? -1 : window.getPreferredFullscreenVideoMode()
                                                                                 .map(monitor::indexOfMode)
                                                                                 .orElse(-1))
                .setTooltip(value -> Component.translatable("options.fullscreen.exclusive.mode.tooltip"));
        videoModeOption.setActivationFn(() -> monitor != null && windowModeOption.getNewValue() == WindowMode.EXCLUSIVE_FULLSCREEN);
        windowModeOption.setOnChange(videoModeOption::updateActiveState);

        return new OptionBlock[]{
                new OptionBlock("", new Option<?>[]{
                        windowModeOption,
                        videoModeOption,
                        VanillaOptions.range(mcOptions.framerateLimit(), 10, 260, 10),
                        VanillaOptions.toggle(mcOptions.enableVsync()),
                        VanillaOptions.cycling(mcOptions.inactivityFpsLimit(), InactivityFpsLimit.values())
                }),
                new OptionBlock("", new Option<?>[]{
                        VanillaOptions.range(mcOptions.guiScale(),
                                0, Math.max(window.calculateScale(0, minecraft.isEnforceUnicode()), mcOptions.guiScale().get()), 1),
                        VanillaOptions.range(mcOptions.gamma(), 0, 100, 1, value -> value * 0.01, value -> (int) Math.round(value * 100.0)),
                        VanillaOptions.cycling(mcOptions.preferredGraphicsBackend(), PreferredGraphicsApi.values())
                }),
                new OptionBlock("", new Option<?>[]{
                        VanillaOptions.cycling(mcOptions.attackIndicator(), AttackIndicatorStatus.values()),
                        VanillaOptions.toggle(mcOptions.showAutosaveIndicator())
                })
        };
    }

    public static OptionBlock[] getGraphicsOpts() {
        var texFilteringOption = VanillaOptions.cycling(mcOptions.textureFiltering(), TextureFilteringMethod.values());
        texFilteringOption.setImpact(PerformanceImpact.MEDIUM);

        var maxAnisotropyOption = VanillaOptions.range(mcOptions.maxAnisotropyBit(), 1, 3, 1);
        maxAnisotropyOption.setActivationFn(() -> texFilteringOption.getNewValue() == TextureFilteringMethod.ANISOTROPIC);
        texFilteringOption.setOnChange(maxAnisotropyOption::updateActiveState);

        return new OptionBlock[]{
                new OptionBlock("", new Option<?>[]{
                        // First, so the options below it apply after it when both changed
                        VanillaOptions.cycling(mcOptions.graphicsPreset(), GraphicsPreset.values()),
                        VanillaOptions.range(mcOptions.renderDistance(), 2, rangeMax(mcOptions.renderDistance(), 32), 1)
                                .setImpact(PerformanceImpact.HIGH),
                        VanillaOptions.range(mcOptions.simulationDistance(), 5, rangeMax(mcOptions.simulationDistance(), 32), 1),
                        VanillaOptions.cycling(mcOptions.prioritizeChunkUpdates(), PrioritizeChunkUpdates.values())
                }),
                new OptionBlock("", new Option<?>[]{
                        texFilteringOption,
                        maxAnisotropyOption,
                        VanillaOptions.cycling(mcOptions.particles(),
                                        new ParticleStatus[]{ParticleStatus.MINIMAL, ParticleStatus.DECREASED, ParticleStatus.ALL})
                                .setImpact(PerformanceImpact.MEDIUM),
                        VanillaOptions.cycling(mcOptions.cloudStatus(), CloudStatus.values()),
                        VanillaOptions.range(mcOptions.cloudRange(), 2, 128, 1),
                        VanillaOptions.toggle(mcOptions.cutoutLeaves()),
                        VanillaOptions.toggle(mcOptions.improvedTransparency())
                                .setImpact(PerformanceImpact.MEDIUM),
                        VanillaOptions.range(mcOptions.chunkSectionFadeInTime(), 0, 40, 1,
                                value -> value / 20.0, value -> (int) Math.round(value * 20.0)),
                        VanillaOptions.toggle(mcOptions.ambientOcclusion())
                                .setImpact(PerformanceImpact.LOW),
                        VanillaOptions.range(mcOptions.biomeBlendRadius(), 0, 7, 1)
                }),
                new OptionBlock("", new Option<?>[]{
                        VanillaOptions.toggle(mcOptions.entityShadows())
                                .setImpact(PerformanceImpact.LOW),
                        VanillaOptions.range(mcOptions.entityDistanceScaling(), 2, 20, 1,
                                        value -> value / 4.0, value -> (int) Math.round(value * 4.0))
                                .setImpact(PerformanceImpact.HIGH),
                        VanillaOptions.range(mcOptions.mipmapLevels(), 0, 4, 1)
                                .setImpact(PerformanceImpact.LOW),
                        VanillaOptions.range(mcOptions.weatherRadius(), 3, 10, 1),
                        VanillaOptions.range(mcOptions.menuBackgroundBlurriness(), 0, 10, 1),
                        VanillaOptions.toggle(mcOptions.vignette())
                })
        };
    }

    public static OptionBlock[] getOptimizationOpts() {
        return new OptionBlock[]{
                new OptionBlock("", new Option<?>[]{
                        new SwitchOption(Component.translatable("ruthenium.options.terrainFaceCulling"),
                                value -> {
                                    config.terrainFaceCulling = value;
                                    // Sections are grouped by facing when they compile
                                    Minecraft.getInstance().levelExtractor.allChanged();
                                },
                                () -> config.terrainFaceCulling)
                                .setTooltip(value -> Component.translatable("ruthenium.options.terrainFaceCulling.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("ruthenium.options.compressedTerrain"),
                                value -> {
                                    config.compressedTerrain = value;
                                    // The vertex format is chosen when the section dispatcher is created
                                    Minecraft minecraft = Minecraft.getInstance();
                                    if (minecraft.level != null) {
                                        ((SectionDispatcherReset) minecraft.levelRenderer).ruthenium$recreateSectionDispatcher();
                                        minecraft.levelExtractor.allChanged();
                                    }
                                },
                                () -> config.compressedTerrain)
                                .setTooltip(value -> Component.translatable("ruthenium.options.compressedTerrain.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("ruthenium.options.biomeBlendCache"),
                                value -> config.biomeBlendCache = value,
                                () -> config.biomeBlendCache)
                                .setTooltip(value -> Component.translatable("ruthenium.options.biomeBlendCache.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.sectionLightCache"),
                                value -> config.sectionLightCache = value,
                                () -> config.sectionLightCache)
                                .setTooltip(value -> Component.translatable("ruthenium.options.sectionLightCache.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.sectionStateCache"),
                                value -> config.sectionStateCache = value,
                                () -> config.sectionStateCache)
                                .setTooltip(value -> Component.translatable("ruthenium.options.sectionStateCache.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM)
                }),
                new OptionBlock("", new Option<?>[]{
                        new SwitchOption(Component.translatable("ruthenium.options.blockEntityCulling"),
                                value -> config.blockEntityCulling = value,
                                () -> config.blockEntityCulling)
                                .setTooltip(value -> Component.translatable("ruthenium.options.blockEntityCulling.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("ruthenium.options.retainedChests"),
                                value -> {
                                    config.retainedChests = value;
                                    RetainedBlockEntities.clear();
                                },
                                () -> config.retainedChests)
                                .setTooltip(value -> Component.translatable("ruthenium.options.retainedChests.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("ruthenium.options.retainedChangeChecks"),
                                value -> config.retainedChangeChecks = value,
                                () -> config.retainedChangeChecks)
                                .setTooltip(value -> Component.translatable("ruthenium.options.retainedChangeChecks.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.retainedGlowingText"),
                                value -> {
                                    config.retainedGlowingText = value;
                                    RetainedBlockEntities.clear();
                                },
                                () -> config.retainedGlowingText)
                                .setTooltip(value -> Component.translatable("ruthenium.options.retainedGlowingText.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.instancedModels"),
                                value -> {
                                    config.instancedModels = value;
                                    InstancedModels.clear();
                                },
                                () -> config.instancedModels)
                                .setTooltip(value -> Component.translatable("ruthenium.options.instancedModels.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("ruthenium.options.armorStandPlates"),
                                value -> {
                                    config.armorStandPlates = value;
                                    InstancedModels.clear();
                                },
                                () -> config.armorStandPlates)
                                .setTooltip(value -> Component.translatable("ruthenium.options.armorStandPlates.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.modelBackFaceCulling"),
                                value -> config.modelBackFaceCulling = value,
                                () -> config.modelBackFaceCulling)
                                .setTooltip(value -> Component.translatable("ruthenium.options.modelBackFaceCulling.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.transparentFaceCulling"),
                                value -> {
                                    config.transparentFaceCulling = value;
                                    TransparentFaces.clear();
                                },
                                () -> config.transparentFaceCulling)
                                .setTooltip(value -> Component.translatable("ruthenium.options.transparentFaceCulling.tooltip"))
                                .setImpact(PerformanceImpact.LOW)
                }),
                new OptionBlock("", new Option<?>[]{
                        new SwitchOption(Component.translatable("ruthenium.options.particleSpawnLimit"),
                                value -> config.particleSpawnLimit = value,
                                () -> config.particleSpawnLimit)
                                .setTooltip(value -> Component.translatable("ruthenium.options.particleSpawnLimit.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.skipIdleTicks"),
                                value -> {
                                    config.skipIdleTicks = value;
                                    unparkTickers();
                                },
                                () -> config.skipIdleTicks)
                                .setTooltip(value -> Component.translatable("ruthenium.options.skipIdleTicks.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.parkIdleTickers"),
                                value -> {
                                    config.parkIdleTickers = value;
                                    unparkTickers();
                                },
                                () -> config.parkIdleTickers)
                                .setTooltip(value -> Component.translatable("ruthenium.options.parkIdleTickers.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.cacheTickerStates"),
                                value -> config.cacheTickerStates = value,
                                () -> config.cacheTickerStates)
                                .setTooltip(value -> Component.translatable("ruthenium.options.cacheTickerStates.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.batchTableTicks"),
                                value -> {
                                    config.batchTableTicks = value;
                                    unparkTickers();
                                },
                                () -> config.batchTableTicks)
                                .setTooltip(value -> Component.translatable("ruthenium.options.batchTableTicks.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.fastEntityCulling"),
                                value -> config.fastEntityCulling = value,
                                () -> config.fastEntityCulling)
                                .setTooltip(value -> Component.translatable("ruthenium.options.fastEntityCulling.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.entityLightCache"),
                                value -> {
                                    config.entityLightCache = value;
                                    EntityLight.invalidate();
                                },
                                () -> config.entityLightCache)
                                .setTooltip(value -> Component.translatable("ruthenium.options.entityLightCache.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.fastEntityCollisions"),
                                value -> config.fastEntityCollisions = value,
                                () -> config.fastEntityCollisions)
                                .setTooltip(value -> Component.translatable("ruthenium.options.fastEntityCollisions.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.fastAnimateTick"),
                                value -> config.fastAnimateTick = value,
                                () -> config.fastAnimateTick)
                                .setTooltip(value -> Component.translatable("ruthenium.options.fastAnimateTick.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.fastBoatTicks"),
                                value -> config.fastBoatTicks = value,
                                () -> config.fastBoatTicks)
                                .setTooltip(value -> Component.translatable("ruthenium.options.fastBoatTicks.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.fastSupportingBlocks"),
                                value -> config.fastSupportingBlocks = value,
                                () -> config.fastSupportingBlocks)
                                .setTooltip(value -> Component.translatable("ruthenium.options.fastSupportingBlocks.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.fastRestingCollisions"),
                                value -> config.fastRestingCollisions = value,
                                () -> config.fastRestingCollisions)
                                .setTooltip(value -> Component.translatable("ruthenium.options.fastRestingCollisions.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.nameTagTextCache"),
                                value -> config.nameTagTextCache = value,
                                () -> config.nameTagTextCache)
                                .setTooltip(value -> Component.translatable("ruthenium.options.nameTagTextCache.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.textDisplayTextCache"),
                                value -> config.textDisplayTextCache = value,
                                () -> config.textDisplayTextCache)
                                .setTooltip(value -> Component.translatable("ruthenium.options.textDisplayTextCache.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.textDisplayCulling"),
                                value -> config.textDisplayCulling = value,
                                () -> config.textDisplayCulling)
                                .setTooltip(value -> Component.translatable("ruthenium.options.textDisplayCulling.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.reuseSortOrder"),
                                value -> config.reuseSortOrder = value,
                                () -> config.reuseSortOrder)
                                .setTooltip(value -> Component.translatable("ruthenium.options.reuseSortOrder.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.playerPoseDraws"),
                                value -> {
                                    config.playerPoseDraws = value;
                                    PlayerPoseDraws.clear();
                                },
                                () -> config.playerPoseDraws)
                                .setTooltip(value -> Component.translatable("ruthenium.options.playerPoseDraws.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.retainedLabels"),
                                value -> {
                                    config.retainedLabels = value;
                                    RetainedLabels.clear();
                                },
                                () -> config.retainedLabels)
                                .setTooltip(value -> Component.translatable("ruthenium.options.retainedLabels.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.cloudFaceTexture"),
                                value -> config.cloudFaceTexture = value,
                                () -> config.cloudFaceTexture)
                                .setTooltip(value -> Component.translatable("ruthenium.options.cloudFaceTexture.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.fastScreenshots"),
                                value -> config.fastScreenshots = value,
                                () -> config.fastScreenshots)
                                .setTooltip(value -> Component.translatable("ruthenium.options.fastScreenshots.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.emptyArmorStandLayers"),
                                value -> config.emptyArmorStandLayers = value,
                                () -> config.emptyArmorStandLayers)
                                .setTooltip(value -> Component.translatable("ruthenium.options.emptyArmorStandLayers.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.renderTypeCache"),
                                value -> config.renderTypeCache = value,
                                () -> config.renderTypeCache)
                                .setTooltip(value -> Component.translatable("ruthenium.options.renderTypeCache.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.reuseEntityStates"),
                                value -> config.reuseEntityStates = value,
                                () -> config.reuseEntityStates)
                                .setTooltip(value -> Component.translatable("ruthenium.options.reuseEntityStates.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.blockModelMeshCache"),
                                value -> {
                                    config.blockModelMeshCache = value;
                                    BlockModelMeshCache.clear();
                                },
                                () -> config.blockModelMeshCache)
                                .setTooltip(value -> Component.translatable("ruthenium.options.blockModelMeshCache.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.hashedDrawLookup"),
                                value -> config.hashedDrawLookup = value,
                                () -> config.hashedDrawLookup)
                                .setTooltip(value -> Component.translatable("ruthenium.options.hashedDrawLookup.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.mergeDraws"),
                                value -> config.mergeDraws = value,
                                () -> config.mergeDraws)
                                .setTooltip(value -> Component.translatable("ruthenium.options.mergeDraws.tooltip"))
                                .setImpact(PerformanceImpact.LOW),
                        new SwitchOption(Component.translatable("ruthenium.options.instancedParts"),
                                value -> {
                                    config.instancedParts = value;
                                    InstancedParts.clear();
                                },
                                () -> config.instancedParts)
                                .setTooltip(value -> Component.translatable("ruthenium.options.instancedParts.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("ruthenium.options.instancedArmor"),
                                value -> {
                                    config.instancedArmor = value;
                                    InstancedParts.clear();
                                },
                                () -> config.instancedArmor)
                                .setTooltip(value -> Component.translatable("ruthenium.options.instancedArmor.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.reuseLayerPoses"),
                                value -> config.reuseLayerPoses = value,
                                () -> config.reuseLayerPoses)
                                .setTooltip(value -> Component.translatable("ruthenium.options.reuseLayerPoses.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.instancedItems"),
                                value -> {
                                    config.instancedItems = value;
                                    InstancedParts.clear();
                                },
                                () -> config.instancedItems)
                                .setTooltip(value -> Component.translatable("ruthenium.options.instancedItems.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("ruthenium.options.instancedBlockModels"),
                                value -> {
                                    config.instancedBlockModels = value;
                                    InstancedParts.clear();
                                },
                                () -> config.instancedBlockModels)
                                .setTooltip(value -> Component.translatable("ruthenium.options.instancedBlockModels.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("ruthenium.options.directBakedQuads"),
                                value -> config.directBakedQuads = value,
                                () -> config.directBakedQuads)
                                .setTooltip(value -> Component.translatable("ruthenium.options.directBakedQuads.tooltip"))
                                .setImpact(PerformanceImpact.LOW)
                })
        };
    }

    /** Puts parked block entity tickers back into the ticker list; they are parked again if parking is still on */
    private static void unparkTickers() {
        if (Minecraft.getInstance().level instanceof TickerParkingLevel level) {
            level.ruthenium$unparkAll();
        }
    }

    /** The upper bound of an int range option, which depends on the memory available at startup */
    private static int rangeMax(OptionInstance<Integer> instance, int fallback) {
        return instance.values() instanceof OptionInstance.IntRangeBase range ? range.maxInclusive() : fallback;
    }

    enum WindowMode {
        WINDOWED("ruthenium.options.windowMode.windowed"),
        BORDERLESS_FULLSCREEN("ruthenium.options.windowMode.borderlessFullscreen"),
        EXCLUSIVE_FULLSCREEN("ruthenium.options.windowMode.exclusiveFullscreen");

        final String translationKey;

        WindowMode(String translationKey) {
            this.translationKey = translationKey;
        }

        static WindowMode current() {
            if (!mcOptions.fullscreen().get()) {
                return WINDOWED;
            }
            return mcOptions.exclusiveFullscreen().get() ? EXCLUSIVE_FULLSCREEN : BORDERLESS_FULLSCREEN;
        }
    }
}
