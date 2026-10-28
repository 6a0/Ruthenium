package net.thallium.render.entity;

import net.minecraft.client.model.HumanoidModel;
import net.minecraft.client.renderer.entity.ArmorStandRenderer;
import net.minecraft.client.renderer.entity.EntityRenderDispatcher;
import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.client.renderer.entity.state.ArmedEntityRenderState;
import net.minecraft.client.renderer.entity.state.ArmorStandRenderState;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.thallium.Initializer;
import net.thallium.mixin.render.entity.EntityRendererAccessor;
import org.jspecify.annotations.Nullable;

import java.util.ArrayList;
import java.util.List;

/**
 * Armor stands are extracted into render states kept from the previous frame instead of new ones. Vanilla allocates
 * a new state per entity and frame ({@code EntityRenderer.createRenderState(Entity, float)}); for an armor stand
 * that call took ~1 µs, while the same extraction into an existing state took ~0.2 µs (and allocating alone
 * ~0.08 µs): compiled together, the allocation made the extraction several times slower. The states also were
 * ~1 KB of garbage per armor stand and frame.
 * <p>
 * A level's entity states are submitted and cleared within the frame they are extracted in
 * ({@code LevelRenderer.submitFeatures}), so a state can be handed out again by the next frame's extraction. The
 * extraction writes every field of an armor stand state except the name tag attachment (only when the name is
 * shown), the camera distance and name tags (only with a camera) and the arm poses of {@link ArmedEntityRenderState}
 * (only by humanoid mobs), which are reset to a new state's values first. Only used from the render thread, for
 * vanilla's armor stand class and renderer.
 */
public final class ReusedRenderStates {
    private static final List<ArmorStandRenderState> ARMOR_STANDS = new ArrayList<>();
    private static int armorStandCount;

    private ReusedRenderStates() {
    }

    /** The level's entity extraction starts; the states of the previous frame are no longer in use */
    public static void beginFrame() {
        armorStandCount = 0;
    }

    public static void clear() {
        ARMOR_STANDS.clear();
        armorStandCount = 0;
    }

    /**
     * {@code EntityRenderer.createRenderState(Entity, float)} into a reused state.
     *
     * @return null if the entity must be extracted as usual
     */
    public static @Nullable ArmorStandRenderState extract(EntityRenderDispatcher dispatcher, ArmorStand entity, float partialTicks) {
        if (!Initializer.CONFIG.reuseEntityStates) {
            return null;
        }
        EntityRenderer<?, ?> renderer = dispatcher.getRenderer(entity);
        if (renderer.getClass() != ArmorStandRenderer.class) {
            return null;
        }
        ArmorStandRenderer armorStandRenderer = (ArmorStandRenderer) renderer;

        ArmorStandRenderState state;
        if (armorStandCount < ARMOR_STANDS.size()) {
            state = ARMOR_STANDS.get(armorStandCount);
            state.distanceToCameraSq = 0.0;
            state.nameTag = null;
            state.scoreText = null;
            state.nameTagAttachment = null;
            // Hidden by the armor stand's own pose fields
            ArmedEntityRenderState armed = state;
            armed.rightArmPose = HumanoidModel.ArmPose.EMPTY;
            armed.leftArmPose = HumanoidModel.ArmPose.EMPTY;
        } else {
            state = newState(armorStandRenderer);
        }
        armorStandCount++;

        armorStandRenderer.extractRenderState(entity, state, partialTicks);
        ((EntityRendererAccessor) armorStandRenderer).invokeFinalizeRenderState(entity, state);
        return state;
    }

    private static ArmorStandRenderState newState(ArmorStandRenderer renderer) {
        ArmorStandRenderState state = renderer.createRenderState();
        ARMOR_STANDS.add(state);
        return state;
    }
}
