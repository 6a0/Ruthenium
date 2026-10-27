package net.vulkanmod.config.option;

import com.mojang.blaze3d.platform.Monitor;
import com.mojang.blaze3d.platform.VideoMode;
import com.mojang.blaze3d.platform.Window;
import net.minecraft.client.*;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ParticleStatus;
import net.vulkanmod.Initializer;
import net.vulkanmod.config.Config;
import net.vulkanmod.config.gui.OptionBlock;
import net.vulkanmod.interfaces.SectionDispatcherReset;
import net.vulkanmod.render.blockentity.InstancedModels;
import net.vulkanmod.render.blockentity.RetainedChests;

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
                Component.translatable("vulkanmod.options.pages.video").getString(),
                Options.getVideoOpts()
        );
        optionPages.add(page);

        page = new OptionPage(
                Component.translatable("vulkanmod.options.pages.graphics").getString(),
                Options.getGraphicsOpts()
        );
        optionPages.add(page);

        page = new OptionPage(
                Component.translatable("vulkanmod.options.pages.optimizations").getString(),
                Options.getOptimizationOpts()
        );
        optionPages.add(page);

        return optionPages;
    }

    public static OptionBlock[] getVideoOpts() {
        Window window = minecraft.getWindow();

        var windowModeOption = new CyclingOption<>(Component.translatable("vulkanmod.options.windowMode"),
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
                        new SwitchOption(Component.translatable("vulkanmod.options.terrainFaceCulling"),
                                value -> {
                                    config.terrainFaceCulling = value;
                                    // Sections are grouped by facing when they compile
                                    Minecraft.getInstance().levelExtractor.allChanged();
                                },
                                () -> config.terrainFaceCulling)
                                .setTooltip(value -> Component.translatable("vulkanmod.options.terrainFaceCulling.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("vulkanmod.options.compressedTerrain"),
                                value -> {
                                    config.compressedTerrain = value;
                                    // The vertex format is chosen when the section dispatcher is created
                                    Minecraft minecraft = Minecraft.getInstance();
                                    if (minecraft.level != null) {
                                        ((SectionDispatcherReset) minecraft.levelRenderer).vulkanmod$recreateSectionDispatcher();
                                        minecraft.levelExtractor.allChanged();
                                    }
                                },
                                () -> config.compressedTerrain)
                                .setTooltip(value -> Component.translatable("vulkanmod.options.compressedTerrain.tooltip"))
                                .setImpact(PerformanceImpact.HIGH)
                }),
                new OptionBlock("", new Option<?>[]{
                        new SwitchOption(Component.translatable("vulkanmod.options.blockEntityCulling"),
                                value -> config.blockEntityCulling = value,
                                () -> config.blockEntityCulling)
                                .setTooltip(value -> Component.translatable("vulkanmod.options.blockEntityCulling.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("vulkanmod.options.retainedChests"),
                                value -> {
                                    config.retainedChests = value;
                                    RetainedChests.clear();
                                },
                                () -> config.retainedChests)
                                .setTooltip(value -> Component.translatable("vulkanmod.options.retainedChests.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("vulkanmod.options.instancedModels"),
                                value -> {
                                    config.instancedModels = value;
                                    InstancedModels.clear();
                                },
                                () -> config.instancedModels)
                                .setTooltip(value -> Component.translatable("vulkanmod.options.instancedModels.tooltip"))
                                .setImpact(PerformanceImpact.HIGH),
                        new SwitchOption(Component.translatable("vulkanmod.options.modelBackFaceCulling"),
                                value -> config.modelBackFaceCulling = value,
                                () -> config.modelBackFaceCulling)
                                .setTooltip(value -> Component.translatable("vulkanmod.options.modelBackFaceCulling.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM)
                }),
                new OptionBlock("", new Option<?>[]{
                        new SwitchOption(Component.translatable("vulkanmod.options.particleSpawnLimit"),
                                value -> config.particleSpawnLimit = value,
                                () -> config.particleSpawnLimit)
                                .setTooltip(value -> Component.translatable("vulkanmod.options.particleSpawnLimit.tooltip"))
                                .setImpact(PerformanceImpact.MEDIUM),
                        new SwitchOption(Component.translatable("vulkanmod.options.skipIdleTicks"),
                                value -> config.skipIdleTicks = value,
                                () -> config.skipIdleTicks)
                                .setTooltip(value -> Component.translatable("vulkanmod.options.skipIdleTicks.tooltip"))
                                .setImpact(PerformanceImpact.LOW)
                })
        };
    }

    /** The upper bound of an int range option, which depends on the memory available at startup */
    private static int rangeMax(OptionInstance<Integer> instance, int fallback) {
        return instance.values() instanceof OptionInstance.IntRangeBase range ? range.maxInclusive() : fallback;
    }

    enum WindowMode {
        WINDOWED("vulkanmod.options.windowMode.windowed"),
        BORDERLESS_FULLSCREEN("vulkanmod.options.windowMode.borderlessFullscreen"),
        EXCLUSIVE_FULLSCREEN("vulkanmod.options.windowMode.exclusiveFullscreen");

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
