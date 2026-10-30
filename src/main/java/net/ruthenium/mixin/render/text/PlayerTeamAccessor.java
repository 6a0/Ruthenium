package net.ruthenium.mixin.render.text;

import net.minecraft.world.scores.PlayerTeam;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import java.util.Optional;

/** The team's color field as it is (a new {@code Optional} whenever the color is set) */
@Mixin(PlayerTeam.class)
public interface PlayerTeamAccessor {
    @Accessor("color")
    Optional<?> ruthenium$color();
}
