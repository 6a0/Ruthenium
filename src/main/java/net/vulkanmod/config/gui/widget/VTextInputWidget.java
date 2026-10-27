package net.vulkanmod.config.gui.widget;

import com.mojang.blaze3d.platform.InputConstants;
import net.minecraft.util.Util;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.ComponentPath;
import net.minecraft.client.gui.navigation.FocusNavigationEvent;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.network.chat.Component;
import net.minecraft.util.StringUtil;
import net.vulkanmod.config.gui.render.GuiRenderer;
import net.vulkanmod.config.gui.util.VGuiConstants;
import net.vulkanmod.config.gui.util.ColorUtil;
import org.jspecify.annotations.Nullable;

import java.util.function.Consumer;

public class VTextInputWidget extends VAbstractWidget {
    public boolean selected = false;
    Consumer<VTextInputWidget> onSearch; // when the search is "activated", like pressing enter
    private String text;
    private final Component placeholder;

    private int cursorPos = 0;
    private int selectionEnd = 0;
    private long lastBlinkTime = 0;
    private boolean showCursor = true;

    private static final int CURSOR_BLINK_INTERVAL = 500; // ms

    public VTextInputWidget(int x, int y, int width, int height, Component placeholder, Consumer<VTextInputWidget> onSearch) {
        this.setPosition(x, y, width, height);

        this.placeholder = placeholder;
        this.onSearch = onSearch;
        this.text = "";
    }

    @Override
    public void renderWidget(double mouseX, double mouseY) {
        if (!this.isVisible()) return;

        boolean hasText = !this.text.isEmpty();
        boolean isFocused = this.focused || this.selected;

        int backgroundColor = ColorUtil.ARGB.multiplyAlpha(VGuiConstants.COLOR_BLACK, 0.45f);

        int textColor = hasText ? VGuiConstants.COLOR_WHITE : VGuiConstants.COLOR_GRAY;

        GuiRenderer.fill(this.x, this.y, this.x + this.width, this.y + this.height, backgroundColor);

        this.renderHovering(0, 0);

        if (isFocused && cursorPos != selectionEnd) {
            int start = Math.min(cursorPos, selectionEnd);
            int end = Math.max(cursorPos, selectionEnd);
            String before = text.substring(0, start);
            String selected = text.substring(start, end);

            int xBefore = this.x + 8 + Minecraft.getInstance().font.width(before);
            int xSelected = Minecraft.getInstance().font.width(selected);

            int selColor = ColorUtil.ARGB.multiplyAlpha(VGuiConstants.COLOR_RED, 0.55f);
            GuiRenderer.fill(xBefore, this.y + 4, xBefore + xSelected, this.y + this.height - 4, selColor);
        }

        Component displayText = hasText ? Component.literal(this.text) : this.placeholder;
        GuiRenderer.drawString(Minecraft.getInstance().font, displayText,
                this.x + 8, this.y + (this.height - 8) / 2, textColor | 0xFF000000);

        if (isFocused && showCursor) {
            String beforeCursor = text.substring(0, cursorPos);
            int cursorX = this.x + 8 + Minecraft.getInstance().font.width(beforeCursor);

            GuiRenderer.fill(cursorX, this.y + 6, cursorX + 1, this.y + this.height - 6,
                    VGuiConstants.COLOR_WHITE);
        }

        if (isFocused) {
            int borderColor = ColorUtil.ARGB.multiplyAlpha(VGuiConstants.COLOR_RED, 0.8f);
            GuiRenderer.renderBorder(this.x, this.y, this.x + this.width, this.y + this.height, 1, borderColor);
        }

        if (isFocused) {
            long time = Util.getMillis();
            if (time - lastBlinkTime > CURSOR_BLINK_INTERVAL) {
                showCursor = !showCursor;
                lastBlinkTime = time;
            }
        } else {
            showCursor = true;
        }
    }

    @Override
    public boolean keyPressed(KeyEvent keyEvent) {
        if (!this.focused && !this.selected) return false;

        boolean shift = keyEvent.hasShiftDown();
        int key = keyEvent.key();

        if (keyEvent.isConfirmation()) {
            this.onSearch.accept(this);
            return true;
        }

        if (cursorPos != selectionEnd) {
            if (key == InputConstants.KEY_BACKSPACE || key == InputConstants.KEY_DELETE) {
                this.insert("");
                return true;
            }
        }

        if (key == InputConstants.KEY_BACKSPACE) {
            if (cursorPos > 0) {
                this.text = text.substring(0, cursorPos - 1) + text.substring(cursorPos);
                cursorPos--;
                selectionEnd = cursorPos;
                this.onSearch.accept(this);
            }
            return true;
        }

        if (key == InputConstants.KEY_DELETE) {
            if (cursorPos < text.length()) {
                this.text = text.substring(0, cursorPos) + text.substring(cursorPos + 1);
                this.onSearch.accept(this);
            }
            return true;
        }

        if (keyEvent.isSelectAll()) {
            cursorPos = text.length();
            selectionEnd = 0;
            return true;
        }

        if (keyEvent.isPaste()) {
            this.insert(StringUtil.filterText(Minecraft.getInstance().keyboardHandler.getClipboard()));
            return true;
        }

        if (keyEvent.isLeft()) {
            if (cursorPos > 0) cursorPos--;
            if (!shift) selectionEnd = cursorPos;
            return true;
        }
        if (keyEvent.isRight()) {
            if (cursorPos < text.length()) cursorPos++;
            if (!shift) selectionEnd = cursorPos;
            return true;
        }
        if (key == InputConstants.KEY_HOME || key == InputConstants.KEY_END) {
            cursorPos = key == InputConstants.KEY_HOME ? 0 : text.length();
            if (!shift) selectionEnd = cursorPos;
            return true;
        }

        return false;
    }

    @Override
    public boolean charTyped(CharacterEvent event) {
        if (!this.focused && !this.selected) return false;

        if (!event.isAllowedChatCharacter()) {
            return false;
        }

        this.insert(event.codepointAsString());
        return true;
    }

    /** Replaces the selection (or inserts at the cursor) */
    private void insert(String s) {
        int start = Math.min(cursorPos, selectionEnd);
        int end = Math.max(cursorPos, selectionEnd);
        this.text = text.substring(0, start) + s + text.substring(end);
        cursorPos = start + s.length();
        selectionEnd = cursorPos;
        this.onSearch.accept(this);
    }

    public String getInput() {
        return this.text;
    }

    public void setInput(String input) {
        this.text = input != null ? input : "";
    }

    @SuppressWarnings("unused")
    public void setSelected(boolean selected) {
        this.selected = selected;
    }

    public boolean isVisible() {
        return visible;
    }

    @Override
    public boolean isActive() {
        return active;
    }

    public void setActive(boolean active) {
        this.active = active;
    }

    @Override
    public @Nullable ComponentPath nextFocusPath(FocusNavigationEvent event) {
        if (!this.active || !this.visible)
            return null;
        return super.nextFocusPath(event);
    }

    @Override
    public boolean mouseClicked(MouseButtonEvent event, boolean bl) {
        if (!this.active || !this.visible) return false;

        boolean clicked = this.clicked(event.x(), event.y());
        if (clicked) {
            this.setFocused(true);
            this.selected = true;

            int relX = (int) event.x() - (this.x + 8);
            int pos = 0;
            for (int i = 0; i < text.length(); i++) {
                if (Minecraft.getInstance().font.width(text.substring(0, i + 1)) > relX) break;
                pos = i + 1;
            }
            cursorPos = pos;
            selectionEnd = pos;

            return true;
        } else {
            this.setFocused(false);
            this.selected = false;
            return false;
        }
    }

    @Override
    public void setFocused(boolean focused) {
        super.setFocused(focused);
        if (!focused) {
            this.selected = false;
        }
    }
}
