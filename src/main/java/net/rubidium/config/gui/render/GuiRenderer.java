package net.rubidium.config.gui.render;

import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.render.TextureSetup;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.util.Mth;
import net.minecraft.util.Util;
import org.joml.Matrix3x2f;

import java.util.List;

/**
 * Static drawing helpers for the settings screen, forwarding to the {@link GuiGraphicsExtractor} of the frame
 * being extracted.
 */
public abstract class GuiRenderer {

    public static GuiGraphicsExtractor guiGraphics;

    public static void enableScissor(int i, int j, int k, int l) {
        guiGraphics.enableScissor(i, j, k, l);
    }

    public static void disableScissor() {
        guiGraphics.disableScissor();
    }

    public static void fillBox(int x0, int y0, int width, int height, int color) {
        fill(x0, y0, x0 + width, y0 + height, color);
    }

    public static void fill(int x0, int y0, int x1, int y1, int color) {
        guiGraphics.fill(x0, y0, x1, y1, color);
    }

    @SuppressWarnings("unused")
    public static void fillGradient(int x0, int y0, int x1, int y1, int color1, int color2) {
        guiGraphics.fillGradient(x0, y0, x1, y1, color1, color2);
    }

    public static void renderBoxBorder(int x0, int y0, int width, int height, int borderWidth, int color) {
        renderBorder(x0, y0, x0 + width, y0 + height, borderWidth, color);
    }

    public static void renderBorder(int x0, int y0, int x1, int y1, int width, int color) {
        GuiRenderer.fill(x0, y0, x1, y0 + width, color);
        GuiRenderer.fill(x0, y1 - width, x1, y1, color);

        GuiRenderer.fill(x0, y0 + width, x0 + width, y1 - width, color);
        GuiRenderer.fill(x1 - width, y0 + width, x1, y1 - width, color);
    }

    public static void drawString(Font font, Component component, int x, int y, int color) {
        drawString(font, component.getVisualOrderText(), x, y, color);
    }

    public static void drawString(Font font, FormattedCharSequence formattedCharSequence, int x, int y, int color) {
        guiGraphics.text(font, formattedCharSequence, x, y, color);
    }

    public static void drawString(Font font, Component component, int x, int y, int color, boolean shadow) {
        drawString(font, component.getVisualOrderText(), x, y, color, shadow);
    }

    public static void drawString(Font font, FormattedCharSequence formattedCharSequence, int x, int y, int color, boolean shadow) {
        guiGraphics.text(font, formattedCharSequence, x, y, color, shadow);
    }

    public static void drawCenteredString(Font font, Component component, int x, int y, int color) {
        FormattedCharSequence formattedCharSequence = component.getVisualOrderText();
        guiGraphics.text(font, formattedCharSequence, x - font.width(formattedCharSequence) / 2, y, color);
    }

    public static void drawScrollingString(Font font, Component component, int x, int y, int maxWidth, int color) {
        int textWidth = font.width(component);
        if (textWidth <= maxWidth) {
            drawCenteredString(font, component, x, y, color);
        } else {
            int x0 = x - maxWidth / 2, x1 = x + maxWidth / 2;
            int scrollAmount = textWidth - maxWidth;
            double currentTimeInSeconds = (double) Util.getMillis() / 1000.0;
            double scrollSpeed = Math.max(scrollAmount * 0.5, 3.0);
            double scrollingOffset = Math.sin((Math.PI / 2) * Math.cos((Math.PI * 2) * currentTimeInSeconds / scrollSpeed)) / 2.0 + 0.5;
            double horizontalScroll = Mth.lerp(scrollingOffset, 0.0, scrollAmount);

            enableScissor(x0 - 1, 0, x1, Minecraft.getInstance().getWindow().getGuiScaledHeight());
            drawString(font, component, (int) (x0 - horizontalScroll), y, color);
            disableScissor();
        }
    }

    public static int getMaxTextWidth(Font font, List<FormattedCharSequence> list) {
        int maxWidth = 0;
        for (var text : list) {
            int width = font.width(text);
            if (width > maxWidth) {
                maxWidth = width;
            }
        }
        return maxWidth;
    }

    public static void blit(Identifier texture, int x, int y, int width, int height) {
        guiGraphics.blit(RenderPipelines.GUI_TEXTURED, texture, x, y, 0.0f, 0.0f, width, height, width, height);
    }

    /**
     * A triangle, as a quad of the GUI's quad pipeline with its last vertex repeated.
     */
    public static void submitTriangle(float[][] vertices, int color) {
        submitPolygon(RenderPipelines.GUI, TextureSetup.noTexture(),
                new float[][]{vertices[0], vertices[1], vertices[2], vertices[2]}, color);
    }

    public static void submitPolygon(RenderPipeline renderPipeline, TextureSetup textureSetup, float[][] vertices, int color) {
        guiGraphics.guiRenderState.addGuiElement(
                new PolygonRenderState(
                        renderPipeline, textureSetup, new Matrix3x2f(guiGraphics.pose()), vertices, color, guiGraphics.scissorStack.peek()
                )
        );
    }
}
