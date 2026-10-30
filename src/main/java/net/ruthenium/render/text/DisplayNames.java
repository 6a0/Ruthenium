package net.ruthenium.render.text;

import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.scores.PlayerTeam;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.KeptNameTags;
import net.ruthenium.mixin.render.text.PlayerTeamAccessor;
import org.jspecify.annotations.Nullable;

import java.lang.reflect.Method;
import java.util.Optional;
import java.util.UUID;

/**
 * An entity's name tag without building its display name every frame ({@code keepDisplayNames}).
 * <p>
 * {@code EntityRenderer.getNameTag} returns {@code entity.getDisplayName()}: {@code PlayerTeam.formatNameForTeam(getTeam(),
 * getName())} with a hover event ({@code createHoverEvent}: type, UUID and {@code getName()}) and the UUID as insertion,
 * {@code getName()} being the custom name with its click event removed, or the type's description. All of it is new
 * every frame (~15% of the players scene's render thread allocation), and {@link EntityTextCache} then finds it equal to
 * the one the entity showed last. For renderers and entity classes that keep vanilla's methods for all of these, the
 * result depends only on: the team, its prefix, suffix and color, the custom name, the type and the UUID. While each of
 * them is the same object as when the kept name tag was built, the kept one is returned without calling
 * {@code getNameTag} (it is equal to what vanilla would build: same inputs, and components aren't changed in place
 * once set). Render thread only.
 */
public final class DisplayNames {
    private static final ClassValue<Boolean> PLAIN_RENDERER = new ClassValue<>() {
        @Override
        protected Boolean computeValue(Class<?> type) {
            return declaredBy(type, EntityRenderer.class, "getNameTag", Entity.class);
        }
    };
    private static final ClassValue<Boolean> PLAIN_ENTITY = new ClassValue<>() {
        @Override
        protected Boolean computeValue(Class<?> type) {
            return declaredBy(type, Entity.class, "getDisplayName") && declaredBy(type, Entity.class, "getName")
                    && declaredBy(type, Entity.class, "getTypeName") && declaredBy(type, Entity.class, "getCustomName")
                    && declaredBy(type, Entity.class, "createHoverEvent") && declaredBy(type, Entity.class, "getTeam")
                    && declaredBy(type, Entity.class, "getScoreboardName");
        }
    };

    private DisplayNames() {
    }

    /** What the kept name tag was built from */
    public record Inputs(@Nullable PlayerTeam team, @Nullable Component prefix, @Nullable Component suffix, @Nullable Optional<?> color,
                         @Nullable Component customName, EntityType<?> type, UUID uuid, Component nameTag) {
        boolean matches(@Nullable PlayerTeam team, @Nullable Component customName, EntityType<?> type, UUID uuid) {
            return this.team == team && this.customName == customName && this.type == type && this.uuid == uuid
                    && (team == null || this.prefix == team.getPlayerPrefix() && this.suffix == team.getPlayerSuffix() && this.color == teamColor(team));
        }
    }

    /** The kept name tag if the entity's display name would be built from the same objects again, else null */
    public static @Nullable Component kept(EntityRenderer<?, ?> renderer, Entity entity) {
        if (!Initializer.CONFIG.keepDisplayNames || !RenderSystem.isOnRenderThread()) {
            return null;
        }
        Inputs inputs = ((KeptNameTags) entity).ruthenium$displayNameInputs();
        if (inputs == null || !PLAIN_RENDERER.get(renderer.getClass()) || !PLAIN_ENTITY.get(entity.getClass())) {
            return null;
        }
        PlayerTeam team = entity.getTeam();
        if (team != null && team.getClass() != PlayerTeam.class) {
            return null;
        }
        if (!inputs.matches(team, entity.getCustomName(), entity.getType(), entity.getUUID())) {
            return null;
        }
        return inputs.nameTag;
    }

    /** The name tag built (and kept) for the entity this frame */
    public static void built(EntityRenderer<?, ?> renderer, Entity entity, @Nullable Component nameTag) {
        if (!Initializer.CONFIG.keepDisplayNames || !RenderSystem.isOnRenderThread()) {
            return;
        }
        KeptNameTags holder = (KeptNameTags) entity;
        if (nameTag == null || !PLAIN_RENDERER.get(renderer.getClass()) || !PLAIN_ENTITY.get(entity.getClass())) {
            holder.ruthenium$setDisplayNameInputs(null);
            return;
        }
        PlayerTeam team = entity.getTeam();
        if (team != null && team.getClass() != PlayerTeam.class) {
            holder.ruthenium$setDisplayNameInputs(null);
            return;
        }
        Inputs old = holder.ruthenium$displayNameInputs();
        Component customName = entity.getCustomName();
        if (old != null && old.nameTag == nameTag && old.matches(team, customName, entity.getType(), entity.getUUID())) {
            return;
        }
        holder.ruthenium$setDisplayNameInputs(new Inputs(team, team != null ? team.getPlayerPrefix() : null, team != null ? team.getPlayerSuffix() : null,
                team != null ? teamColor(team) : null, customName, entity.getType(), entity.getUUID(), nameTag));
    }

    private static @Nullable Optional<?> teamColor(PlayerTeam team) {
        return ((PlayerTeamAccessor) team).ruthenium$color();
    }

    private static boolean declaredBy(Class<?> type, Class<?> owner, String name, Class<?>... parameters) {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            try {
                Method method = c.getDeclaredMethod(name, parameters);
                return method.getDeclaringClass() == owner;
            } catch (NoSuchMethodException e) {
                // Look further up
            }
        }
        return false;
    }
}
