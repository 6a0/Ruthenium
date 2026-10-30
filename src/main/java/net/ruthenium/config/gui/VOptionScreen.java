package net.ruthenium.config.gui;

import com.mojang.blaze3d.platform.InputConstants;
import com.google.common.collect.Lists;
import net.minecraft.client.Minecraft;
import net.minecraft.client.TextureFilteringMethod;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.ConfirmLinkScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.options.VideoSettingsScreen;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.network.chat.CommonComponents;
import net.minecraft.network.chat.Component;
import net.minecraft.util.FormattedCharSequence;
import net.ruthenium.config.gui.render.GuiRenderer;
import net.ruthenium.config.gui.util.SearchHelper;
import net.ruthenium.config.gui.util.VGuiConstants;
import net.ruthenium.config.gui.widget.*;
import net.ruthenium.config.option.*;
import net.ruthenium.config.gui.util.ColorUtil;

import java.net.URI;
import java.util.ArrayList;
import java.util.List;

public class VOptionScreen extends Screen {
    public final static int MARGIN = 10;
    private static final URI KOFI_URI = URI.create("https://ko-fi.com/xcollateral");

    private final Screen parent;

    private final List<ModSettingsEntry> modSettingsEntries;

    private final List<OptionPage> optionPages;
    private OptionPage searchResultsPage;

    private int currentListIdx = 0;
    private boolean isSearchActive = false;

    private int tooltipX;
    private int tooltipY;
    private int tooltipWidth;

    private VButtonWidget applyButton;
    private VButtonWidget undoButton;

    private VTextInputWidget searchField;

    private final List<VAbstractWidget> iconWidgets = Lists.newArrayList();
    private final List<VButtonWidget> pageButtons = Lists.newArrayList();
    private final List<VButtonWidget> buttons = Lists.newArrayList();

    // Texture options that need a texture reload when changed, applied when the screen closes as vanilla does
    private final int oldMipmaps;
    private final int oldAnisotropyBit;
    private final TextureFilteringMethod oldTextureFiltering;

    public VOptionScreen(Component title, Screen parent) {
        super(title);
        this.parent = parent;

        this.optionPages = new ArrayList<>();
        this.modSettingsEntries = new ArrayList<>(ModSettingsRegistry.INSTANCE.getModEntries());

        var options = Minecraft.getInstance().options;
        this.oldMipmaps = options.mipmapLevels().get();
        this.oldAnisotropyBit = options.maxAnisotropyBit().get();
        this.oldTextureFiltering = options.textureFiltering().get();
    }

    @Override
    protected void init() {
        this.initOptionsPages();

        if (this.optionPages.isEmpty()) {
            throw new IllegalStateException("Default Options weren't added!");
        }

        int top = 32;
        int bottom = 60;
        int itemHeight = 20;

        int leftMargin = MARGIN + VGuiConstants.PAGE_BUTTON_WIDTH + 6;
        int listWidth = Math.min(this.width - leftMargin - MARGIN, 420);
        int listHeight = this.height - top - bottom;

        this.buildLists(leftMargin, top, listWidth, listHeight, itemHeight);

        this.searchField = createSearchField();

        int x = leftMargin + listWidth + 6;
        int tooltipWidth = Math.min(this.width - x - 10, 420);
        int y = top + itemHeight + 6;

        if (tooltipWidth < 200) {
            x = leftMargin + 3;
            tooltipWidth = listWidth;
            y = this.height - bottom + 10;
        }

        this.tooltipX = x;
        this.tooltipY = y;
        this.tooltipWidth = tooltipWidth;

        this.buildPage();
    }

    private void initOptionsPages() {
        this.optionPages.clear();

        for (var modPageSet : this.modSettingsEntries) {
            modPageSet.initPages();

            this.optionPages.addAll(modPageSet.getPages());
        }
    }

    private VTextInputWidget createSearchField() {
        int rightMargin = 10;
        int padding = 10;
        int kofiWidth = Minecraft.getInstance().font.width(Component.translatable("ruthenium.options.buttons.kofi")) + padding;
        int topBarRight = this.width - kofiWidth - rightMargin;

        int leftMargin = VGuiConstants.PAGE_BUTTON_WIDTH + MARGIN + 6;
        int width = Math.min(topBarRight - leftMargin - 4, 413);

        return new VTextInputWidget(
                leftMargin, 4,
                width, VGuiConstants.WIDGET_HEIGHT,
                Component.translatable("ruthenium.options.searchFieldPlaceholder"),
                widget -> performSearch(widget.getInput())
        );
    }

    private void buildLists(int left, int top, int listWidth, int listHeight, int itemHeight) {
        for (OptionPage page : this.optionPages) {
            page.createList(left, top, listWidth, listHeight, itemHeight);
            page.updateOptionStates();
        }
    }

    private void performSearch(String query) {
        if (query == null || query.trim().isEmpty()) {
            isSearchActive = false;
            this.currentListIdx = 0;
            buildPage();
            return;
        }

        String searchTerm = query.toLowerCase().trim();
        List<OptionBlock> searchResults = new ArrayList<>();

        for (OptionPage page : this.optionPages) {
            List<Option<?>> matchingOptions = new ArrayList<>();

            for (OptionBlock block : page.optionBlocks) {
                for (Option<?> option : block.options()) {
                    boolean matches = false;

                    String optionName = option.getName().getString().toLowerCase();
                    String optionTooltip = option.getTooltip() != null ?
                            option.getTooltip().getString().toLowerCase() : "";
                    String displayedValue = option.getDisplayedValue().getString().toLowerCase();

                    if (optionName.contains(searchTerm) ||
                            optionTooltip.contains(searchTerm) ||
                            displayedValue.contains(searchTerm)) {
                        matches = true;
                    }

                    else if (option instanceof CyclingOption<?> cycling) {
                        if (SearchHelper.matchesAnyValue(cycling, searchTerm)) {
                            matches = true;
                        }
                    }

                    if (matches) {
                        matchingOptions.add(option);
                    }
                }
            }

            if (!matchingOptions.isEmpty()) {
                searchResults.add(new OptionBlock("§l" + page.name,
                        matchingOptions.toArray(new Option<?>[0])));
                searchResults.add(new OptionBlock("", new Option<?>[0]));
            }
        }

        searchResultsPage = new OptionPage(
                "Search Results",
                searchResults.toArray(new OptionBlock[0])
        );

        int top = 32;
        int bottom = 60;
        int itemHeight = 20;
        int leftMargin = MARGIN + VGuiConstants.PAGE_BUTTON_WIDTH + 6;
        int listWidth = Math.min(this.width - leftMargin - MARGIN, 420);
        int listHeight = this.height - top - bottom;

        searchResultsPage.createList(leftMargin, top, listWidth, listHeight, itemHeight);

        isSearchActive = true;
        buildPage();
    }

    private void buildPage() {
        this.buttons.clear();
        this.pageButtons.clear();
        this.iconWidgets.clear();

        String savedInput = this.searchField != null ? this.searchField.getInput() : "";
        boolean savedFocused = this.searchField != null && this.searchField.focused;
        boolean savedSelected = this.searchField != null && this.searchField.selected;

        this.clearWidgets();

        int x = MARGIN;
        int y = 4;

        int width = VGuiConstants.PAGE_BUTTON_WIDTH;
        int j = 0;
        for (var modEntry : this.modSettingsEntries) {
            ModIconWidget iconWidget = new ModIconWidget(modEntry.modName, modEntry.getIcon(), x, y, width, 28);
            this.iconWidgets.add(iconWidget);
            this.addWidget(iconWidget);
            y += 28;

            var pages = modEntry.getPages();
            for (OptionPage page : pages) {
                final int finalIdx = j;
                VButtonWidget widget = new VButtonWidget(x, y, width, VGuiConstants.WIDGET_HEIGHT, Component.nullToEmpty(page.name), button -> this.setOptionList(finalIdx));
                widget.setTextLayout(false, 12);
                this.buttons.add(widget);
                this.pageButtons.add(widget);
                this.addWidget(widget);

                y += VGuiConstants.WIDGET_HEIGHT;
                j++;
            }
        }

        if (!isSearchActive) {
            this.pageButtons.get(this.currentListIdx).setSelected(true);
            VOptionList currentList = this.optionPages.get(this.currentListIdx).getOptionList();
            this.addWidget(currentList);
        } else {
            if (searchResultsPage != null) {
                VOptionList searchList = searchResultsPage.getOptionList();
                this.addWidget(searchList);
                searchResultsPage.updateOptionStates();
            }
        }

        this.addButtonsWithSearchBar();

        this.searchField.setInput(savedInput);
        if (savedFocused) {
            this.searchField.setFocused(true);
            this.searchField.setSelected(savedSelected);
        }

        // The buttons were recreated
        this.updateState();
    }

    @SuppressWarnings("DuplicatedCode")
    private void addButtonsWithSearchBar() {
        int rightMargin = 10;
        int padding = 10;
        int buttonWidth = Minecraft.getInstance().font.width(CommonComponents.GUI_DONE) + 2 * padding;
        int x0 = (this.width - buttonWidth - rightMargin);
        int y0 = this.height - VGuiConstants.WIDGET_HEIGHT - 7;

        VButtonWidget doneButton = new VButtonWidget(x0, y0, buttonWidth, VGuiConstants.WIDGET_HEIGHT,
                CommonComponents.GUI_DONE, button -> Minecraft.getInstance().gui.setScreen(this.parent));

        buttonWidth = Minecraft.getInstance().font.width(Component.translatable("ruthenium.options.buttons.apply")) + 2 * padding;
        x0 -= (buttonWidth + VGuiConstants.WIDGET_MARGIN);
        this.applyButton = new VButtonWidget(x0, y0, buttonWidth, VGuiConstants.WIDGET_HEIGHT,
                Component.translatable("ruthenium.options.buttons.apply"), button -> this.applyOptions());

        buttonWidth = Minecraft.getInstance().font.width(Component.translatable("ruthenium.options.buttons.undo")) + 2 * padding;
        x0 -= (buttonWidth + VGuiConstants.WIDGET_MARGIN);
        this.undoButton = new VButtonWidget(x0, y0, buttonWidth, VGuiConstants.WIDGET_HEIGHT,
                Component.translatable("ruthenium.options.buttons.undo"), button -> undo());

        int kofiWidth = Minecraft.getInstance().font.width(Component.translatable("ruthenium.options.buttons.kofi")) + padding;

        int kofiX = this.width - kofiWidth - rightMargin;
        VButtonWidget supportButton = new VButtonWidget(kofiX, 4, kofiWidth, VGuiConstants.WIDGET_HEIGHT,
                Component.translatable("ruthenium.options.buttons.kofi"),
                button -> ConfirmLinkScreen.confirmLinkNow(this, KOFI_URI));

        this.buttons.add(this.applyButton);
        this.buttons.add(doneButton);
        this.buttons.add(supportButton);
        this.buttons.add(this.undoButton);

        this.addWidget(this.applyButton);
        this.addWidget(doneButton);
        this.addWidget(supportButton);
        this.addWidget(this.undoButton);
        this.addWidget(this.searchField);
    }

    @Override
    public boolean mouseClicked(MouseButtonEvent event, boolean bl) {
        for (GuiEventListener element : this.children()) {
            if (element.mouseClicked(event, bl)) {
                this.setFocused(element);
                if (event.button() == InputConstants.MOUSE_BUTTON_LEFT) {
                    this.setDragging(true);
                }

                this.updateState();
                return true;
            }
        }

        return false;
    }

    @Override
    public boolean mouseReleased(MouseButtonEvent event) {
        this.setDragging(false);
        this.updateState();
        return this.getChildAt(event.x(), event.y())
                   .filter(guiEventListener -> guiEventListener.mouseReleased(event))
                   .isPresent();
    }

    @Override
    public void removed() {
        var options = this.minecraft.options;
        if (options.mipmapLevels().get() != this.oldMipmaps
                || options.maxAnisotropyBit().get() != this.oldAnisotropyBit
                || options.textureFiltering().get() != this.oldTextureFiltering) {
            this.minecraft.updateMaxMipLevel(options.mipmapLevels().get());
            this.minecraft.delayTextureReload();
        }

        options.save();
    }

    @Override
    public void onClose() {
        Minecraft.getInstance().gui.setScreen(this.parent);
    }

    @Override
    public void extractRenderState(GuiGraphicsExtractor guiGraphics, int mouseX, int mouseY, float delta) {
        GuiRenderer.guiGraphics = guiGraphics;

        VOptionList currentList;
        if (isSearchActive && searchResultsPage != null) {
            currentList = searchResultsPage.getOptionList();
        } else {
            currentList = this.optionPages.get(this.currentListIdx).getOptionList();
        }

        currentList.updateState(mouseX, mouseY);
        currentList.renderWidget(mouseX, mouseY);

        for (var widget : iconWidgets) {
            widget.render(mouseX, mouseY);
        }

        for (VButtonWidget button : buttons) {
            button.updateState(mouseX, mouseY);
            button.render(mouseX, mouseY);
        }
        searchField.updateState(mouseX, mouseY);
        searchField.render(mouseX, mouseY);

        VAbstractWidget hoveredWidget = null;

        for (var b : buttons) {
            if (b.isMouseOver(mouseX, mouseY)) {
                hoveredWidget = b;
                break;
            }
        }

        if (hoveredWidget == null) {
            hoveredWidget = currentList.getHoveredWidget(mouseX, mouseY);
        }

        if (hoveredWidget != null) {
            this.renderTooltip(hoveredWidget, this.tooltipX, this.tooltipY);
        }
    }

    private void renderTooltip(VAbstractWidget widget, int x, int y) {
        var list = this.getWidgetTooltip(widget);

        if (list.isEmpty()) {
            return;
        }

        int lines = list.size();

        int padding = 3;
        int width = GuiRenderer.getMaxTextWidth(this.font, list);
        int height = lines * 10;
        float intensity = 0.05f;
        int color = ColorUtil.ARGB.pack(intensity, intensity, intensity, 0.6f);
        GuiRenderer.fill(x - padding, y - padding, x + width + padding, y + height + padding, color);

        color = VGuiConstants.COLOR_PRIMARY;
        GuiRenderer.renderBorder(x - padding, y - padding, x + width + padding, y + height + padding, 1, color);

        int yOffset = 0;
        for (var text : list) {
            GuiRenderer.drawString(this.font, text, x, y + yOffset, 0xffffffff);
            yOffset += 10;
        }
    }

    private List<FormattedCharSequence> getWidgetTooltip(VAbstractWidget widget) {
        var tooltip = widget.getTooltip();
        var impact = widget.getImpact();

        List<FormattedCharSequence> textList = new ArrayList<>();
        if (tooltip != null) {
            textList.addAll(this.font.split(tooltip, this.tooltipWidth));
        }

        if (impact != null) {
            textList.addAll(this.font.split(Component.translatable("ruthenium.options.performanceImpact", impact.component()), this.tooltipWidth));
        }

        return textList;
    }

    private void updateState() {
        if (this.applyButton == null | this.undoButton == null) return;
        boolean modified = false;
        for (var page : this.optionPages) {
            modified |= page.optionChanged();
        }

        if (modified) {
            for (var page : this.optionPages) {
                page.optionChanged();
            }
        }

        this.applyButton.active = modified;
        this.undoButton.visible = modified;
    }

    private void setOptionList(int i) {
        this.currentListIdx = i;
        this.isSearchActive = false;

        this.searchField.setInput("");
        this.searchField.setFocused(false);

        this.buildPage();

        this.pageButtons.get(i).setSelected(true);
    }

    private void undo() {
        for (OptionPage page : this.optionPages) {
            page.resetToOriginalState();
            page.updateOptionStates();
        }

        buildPage();
    }

    private void applyOptions() {
        List<OptionPage> pages = List.copyOf(this.optionPages);
        for (var page : pages) {
            page.applyOptionChanges();
        }

        // Applied options can change others (the graphics preset sets most of them), and a GUI scale change
        // rebuilds the pages (resize -> init) midway, so this syncs the current ones
        for (var page : this.optionPages) {
            page.syncValues();
            page.updateOptionStates();
        }

        for (var modEntry : this.modSettingsEntries) {
            modEntry.runOnApply();
        }

        this.minecraft.options.save();
        this.updateState();
    }

    @Override
    public boolean keyPressed(KeyEvent keyEvent) {
        // Shortcut keys are layout dependent SDL keycodes, which for letters are their lowercase character
        if (keyEvent.hasControlDownWithQuirk() && keyEvent.shortcutKey() == 'l') {
            this.setFocused(searchField);
            searchField.setFocused(true);
            searchField.setSelected(true);

            return true;
        }

        if (keyEvent.isEscape() && this.isSearchActive) {
            this.isSearchActive = false;
            this.searchField.setInput("");
            this.searchField.setFocused(false);
            this.buildPage();
            this.pageButtons.get(this.currentListIdx).setSelected(true);
            return true;
        }


        if (!this.searchField.focused
                && keyEvent.shortcutKey() == 'p'
                && keyEvent.hasShiftDown()) {
            Minecraft.getInstance().gui.setScreen(new VideoSettingsScreen(this, Minecraft.getInstance(), Minecraft.getInstance().options));

            return false;
        }

        return super.keyPressed(keyEvent);
    }
}