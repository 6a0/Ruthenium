package net.ruthenium.config.gui.widget;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.sounds.SoundManager;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FormattedText;
import net.minecraft.resources.Identifier;
import net.ruthenium.config.gui.render.GuiRenderer;
import net.ruthenium.config.gui.util.VGuiConstants;
import net.ruthenium.config.gui.util.ColorUtil;

public class ModIconWidget extends VAbstractWidget {
    final FormattedText name;
    final Identifier icon;

    public ModIconWidget(FormattedText name, Identifier icon, int x0, int y0, int width, int height) {
        this.name = name;
        this.icon = icon;
        this.x = x0;
        this.y = y0;
        this.width = width;
        this.height = height;
    }

    @Override
    public void playDownSound(SoundManager soundManager) {
    }

    public void render(double mX, double mY) {
        int backgroundColor = ColorUtil.ARGB.multiplyAlpha(VGuiConstants.COLOR_BLACK, 0.6f);
        int width = this.width;
        int height = this.height;
        GuiRenderer.fill(this.x, this.y, this.x + width, this.y + height, backgroundColor);


        int iconSize = this.height - 8;
        int iconX = this.x + 6;
        int iconY = this.y + (height - iconSize) / 2;
        GuiRenderer.blit(icon, iconX, iconY, iconSize, iconSize);
        Font textRenderer = Minecraft.getInstance().font;

        int size = this.height;
        int maxWidth = this.width - iconSize - 8;

        GuiRenderer.drawScrollingString(textRenderer, (Component) this.name, this.x + size + maxWidth / 2 + 3, this.y + this.height / 2 - 4, maxWidth - 4, 0xffffffff);
    }
}
