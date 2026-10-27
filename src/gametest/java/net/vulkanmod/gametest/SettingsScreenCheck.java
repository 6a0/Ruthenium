package net.vulkanmod.gametest;

import com.mojang.blaze3d.platform.InputConstants;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.client.gui.screens.options.OptionsScreen;
import net.minecraft.client.gui.screens.options.VideoSettingsScreen;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.input.MouseButtonInfo;
import net.minecraft.network.chat.CommonComponents;
import net.minecraft.network.chat.Component;
import net.vulkanmod.Initializer;
import net.vulkanmod.config.gui.VOptionScreen;
import net.vulkanmod.config.gui.util.VGuiConstants;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Drives the settings screen from the title screen ({@code -Pbench.settingsScreen=true}): opens it from the
 * options screen's video button, screenshots every page, toggles an optimization and applies it, searches, and
 * opens vanilla's video settings with Shift+P.
 */
final class SettingsScreenCheck {
    private static final Logger LOGGER = LoggerFactory.getLogger("VulkanMod Settings Check");
    private static final int CTRL = 0x40;
    private static final int SHIFT = 0x1;

    private SettingsScreenCheck() {
    }

    static void run(ClientGameTestContext context, String label) {
        context.setScreen(() -> new OptionsScreen(new TitleScreen(), Minecraft.getInstance().options));
        context.clickScreenButton("options.video");
        context.waitForScreen(VOptionScreen.class);
        context.waitTicks(5);
        context.takeScreenshot(label + "-settings-video");

        // Page buttons are below the mod icon (28 high), one widget height apart
        for (int page = 1; page <= 2; page++) {
            int y = 4 + 28 + page * VGuiConstants.WIDGET_HEIGHT + VGuiConstants.WIDGET_HEIGHT / 2;
            context.runOnClient(mc -> click(mc.gui.screen(), VOptionScreen.MARGIN + 20, y));
            context.waitTicks(3);
            context.takeScreenshot(label + "-settings-page" + page);
        }

        // Hover the fourth option of the page (row pitch 23) for its tooltip
        double guiScale = context.computeOnClient(mc -> (double) mc.getWindow().getGuiScale());
        context.getInput().setCursorPos(250 * guiScale, (32 + 3 * 23 + 10) * guiScale);
        context.waitTicks(10);
        context.takeScreenshot(label + "-settings-hover");
        context.getInput().setCursorPos(0, 0);

        // Optimizations page: switch off the first option (block entity culling) and apply
        boolean before = context.computeOnClient(_ -> Initializer.CONFIG.blockEntityCulling);
        context.runOnClient(mc -> {
            Screen screen = mc.gui.screen();
            click(screen, firstControlX(screen) + 10, 32 + 10);
        });
        context.waitTicks(3);
        context.takeScreenshot(label + "-settings-changed");
        context.runOnClient(mc -> clickApply(mc, mc.gui.screen()));
        context.waitTicks(3);
        boolean after = context.computeOnClient(_ -> Initializer.CONFIG.blockEntityCulling);
        String written = readConfig();
        LOGGER.info("blockEntityCulling {} -> {}, config file: {}", before, after, written.replaceAll("\\s+", " "));
        check(after != before, "applying the switch changed the config");
        check(written.contains("\"blockEntityCulling\": " + after), "the config file has the new value");

        // Restore it
        context.runOnClient(mc -> {
            Screen screen = mc.gui.screen();
            click(screen, firstControlX(screen) + 10, 32 + 10);
            clickApply(mc, screen);
        });
        context.waitTicks(3);
        check(context.computeOnClient(_ -> Initializer.CONFIG.blockEntityCulling) == before, "the switch was restored");

        // Search: Ctrl+L focuses the field, typed characters filter the options
        context.runOnClient(mc -> {
            Screen screen = mc.gui.screen();
            screen.keyPressed(new KeyEvent(InputConstants.KEY_L, 'l', CTRL));
            for (char c : "cull".toCharArray()) {
                screen.charTyped(new CharacterEvent(c));
            }
        });
        context.waitTicks(3);
        context.takeScreenshot(label + "-settings-search");

        // Escape leaves the search, then Shift+P opens vanilla's screen
        context.runOnClient(mc -> {
            Screen screen = mc.gui.screen();
            screen.keyPressed(new KeyEvent(InputConstants.KEY_ESCAPE, 0, 0));
            screen.keyPressed(new KeyEvent(InputConstants.KEY_P, 'p', SHIFT));
        });
        context.waitForScreen(VideoSettingsScreen.class);
        context.waitTicks(3);
        context.takeScreenshot(label + "-settings-vanilla");
        context.setScreen(TitleScreen::new);
        LOGGER.info("Settings screen check passed");
    }

    private static void click(Screen screen, double x, double y) {
        var event = new MouseButtonEvent(x, y, new MouseButtonInfo(0, 0));
        screen.mouseClicked(event, false);
        screen.mouseReleased(event);
    }

    /** Where the controls of the option list's widgets start, see {@code OptionWidget.setDimensions} */
    private static int firstControlX(Screen screen) {
        int left = VOptionScreen.MARGIN + VGuiConstants.PAGE_BUTTON_WIDTH + 6;
        int itemWidth = Math.min(screen.width - left - VOptionScreen.MARGIN, 420) - 7;
        int controlWidth = Math.min((int) (itemWidth * 0.5f) - 8, 120);
        return left + itemWidth - controlWidth - 8;
    }

    private static void clickApply(Minecraft mc, Screen screen) {
        int padding = 10;
        int doneWidth = mc.font.width(CommonComponents.GUI_DONE) + 2 * padding;
        int applyWidth = mc.font.width(Component.translatable("vulkanmod.options.buttons.apply")) + 2 * padding;
        int applyX = screen.width - doneWidth - 10 - VGuiConstants.WIDGET_MARGIN - applyWidth;
        int y = screen.height - VGuiConstants.WIDGET_HEIGHT - 7;
        click(screen, applyX + applyWidth / 2.0, y + VGuiConstants.WIDGET_HEIGHT / 2.0);
    }

    private static String readConfig() {
        try {
            return Files.readString(Path.of("config", "vulkanmod_settings.json"));
        } catch (Exception e) {
            return "<" + e + ">";
        }
    }

    private static void check(boolean condition, String what) {
        if (!condition) {
            throw new AssertionError("Settings screen check failed: " + what);
        }
        LOGGER.info("OK: {}", what);
    }
}
