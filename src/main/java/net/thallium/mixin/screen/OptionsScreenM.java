package net.thallium.mixin.screen;

import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.options.OptionsScreen;
import net.minecraft.network.chat.Component;
import net.thallium.config.gui.VOptionScreen;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyArg;

import java.util.function.Supplier;

/**
 * Opens the mod's settings screen from the video settings button. Shift+P there opens vanilla's.
 */
@Mixin(OptionsScreen.class)
public class OptionsScreenM {

    @Shadow @Final private static Component VIDEO;

    @ModifyArg(method = "init", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/gui/screens/options/OptionsScreen;openScreenButton(Lnet/minecraft/network/chat/Component;Ljava/util/function/Supplier;)Lnet/minecraft/client/gui/components/Button;"),
            index = 1)
    private Supplier<Screen> openVideoOptionScreen(Component message, Supplier<Screen> screenSupplier) {
        if (message != VIDEO) {
            return screenSupplier;
        }

        OptionsScreen self = (OptionsScreen) (Object) this;
        return () -> new VOptionScreen(Component.translatable("options.videoTitle"), self);
    }
}
