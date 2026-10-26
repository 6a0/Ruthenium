package net.vulkanmod.mixin.world;

import net.minecraft.world.level.block.entity.ChestLidController;
import net.vulkanmod.interfaces.IdleTicker;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;

@Mixin(ChestLidController.class)
public class ChestLidControllerM implements IdleTicker {
    @Shadow private boolean shouldBeOpen;
    @Shadow private float openness;
    @Shadow private float oOpenness;

    /** {@code tickLid} only copies {@code openness} to {@code oOpenness} for a closed lid that stays closed */
    @Override
    public boolean vulkanmod$isTickIdle() {
        return !this.shouldBeOpen && this.openness == 0.0F && this.oOpenness == 0.0F;
    }
}
