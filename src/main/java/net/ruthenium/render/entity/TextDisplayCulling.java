package net.ruthenium.render.entity;

import com.mojang.math.Transformation;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.util.Mth;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.Display;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.ruthenium.Initializer;
import net.ruthenium.mixin.render.blockentity.FrustumAccessor;
import net.ruthenium.mixin.render.entity.TextDisplayAccessor;
import org.joml.Matrix4fc;

/**
 * Frustum culling of text displays by the extent of their text ({@code textDisplayCulling}).
 * <p>
 * Vanilla culls a display entity by its culling box only when the display has a width and a height; with the default
 * size 0 ({@code Display.updateCulling}: {@code noCulling}) it is extracted, submitted and its text laid out and written
 * every frame within its view range, wherever the camera looks. Hologram plugins leave the size at 0, so every text
 * display of a lobby costs the same with the camera facing away.
 * <p>
 * All geometry of a text display (glyphs, their shadows and outlines, the background) lies within a sphere around its
 * render position: {@code TextDisplayRenderer.submitInner} draws in a rectangle of the line count and the widest
 * line ({@code CachedInfo}), around the local origin after its own rotation, scale (1/40 block per pixel) and offset,
 * and before that the display's {@link Transformation} (an affine matrix: |M v| <= |t| + ||A||_F |v|) and the
 * billboard rotation (which keeps lengths) apply. The rectangle is widened by 8 pixels and 1 pixel deep for glyphs
 * that overhang their advance, shadows, bold and outlines. A display whose sphere's box, widened by vanilla's half
 * block for entity boxes plus 2% of its distance (view bobbing tilts the drawn view by under 1 degree against the
 * culling frustum), is outside the culling frustum draws nothing in view and is skipped.
 * <p>
 * Displays are tested only when nothing else is drawn for them and their extent is known: no passenger offset, name
 * tag, shadow or glow, a constant transformation (not interpolating), and text already split (the first frame of a
 * new text takes vanilla's path). While the camera entity is hurt or dying, or the player sees nausea or a portal
 * (rotations and distortions the culling frustum doesn't have), nothing is culled. Render thread only.
 */
public final class TextDisplayCulling {
    /** Pixels added on each side of the text rectangle and its depth, in the display's text space */
    private static final float GLYPH_MARGIN = 8.0F, DEPTH_MARGIN = 1.0F;
    private static final float PIXEL = 0.025F;

    private TextDisplayCulling() {
    }

    /** True if the text display certainly draws nothing in the frustum (it passed vanilla's own test) */
    public static boolean isHidden(Display.TextDisplay display, Frustum frustum, double camX, double camY, double camZ, float partialTicks) {
        if (!Initializer.CONFIG.textDisplayCulling || display.affectedByCulling() || display.isPassenger() || display.hasCustomName()
                || display.isCurrentlyGlowing() || !cameraMatchesFrustum()) {
            return false;
        }
        Display.RenderState renderState = display.renderState();
        Display.TextDisplay.CachedInfo info = ((TextDisplayAccessor) display).ruthenium$cachedInfo();
        if (renderState == null || info == null || display.textRenderState() == null) {
            return false;
        }
        if (renderState.shadowRadius().get(0.0F) != 0.0F || renderState.shadowRadius().get(1.0F) != 0.0F) {
            return false;
        }
        Transformation transformation = renderState.transformation().get(0.0F);
        if (transformation != renderState.transformation().get(1.0F)) {
            return false;
        }
        double radius = radius(info, transformation);
        if (!Double.isFinite(radius)) {
            return false;
        }

        double x = Mth.lerp(partialTicks, display.xOld, display.getX());
        double y = Mth.lerp(partialTicks, display.yOld, display.getY());
        double z = Mth.lerp(partialTicks, display.zOld, display.getZ());
        double dx = x - camX, dy = y - camY, dz = z - camZ;
        double r = radius + 0.5 + 0.02 * Math.sqrt(dx * dx + dy * dy + dz * dz);
        int result = ((FrustumAccessor) frustum).invokeCubeInFrustum(x - r, y - r, z - r, x + r, y + r, z + r);
        return result >= 0;
    }

    /**
     * The radius around a text display's render position that holds all of its geometry, in blocks (before the
     * margins for the frustum test)
     */
    public static double radius(Display.TextDisplay.CachedInfo info, Transformation transformation) {
        int width = info.width();
        int height = info.lines().size() * 10 - 1;
        // TextDisplayRenderer: rotate(pi, y), scale(-1/40), translate(1 - width / 2, -height, 0), then the background from
        // (-1, -1) to (width, height) and glyphs within [0, width] x [0, height]
        double halfX = width / 2.0 + 1.0 + GLYPH_MARGIN;
        double halfY = Math.max(height, 0) + 1.0 + GLYPH_MARGIN;
        double local = PIXEL * Math.sqrt(halfX * halfX + halfY * halfY + DEPTH_MARGIN * DEPTH_MARGIN);

        Matrix4fc m = transformation.getMatrix();
        double frobenius = Math.sqrt(sq(m.m00()) + sq(m.m01()) + sq(m.m02()) + sq(m.m10()) + sq(m.m11()) + sq(m.m12())
                + sq(m.m20()) + sq(m.m21()) + sq(m.m22()));
        double translation = Math.sqrt(sq(m.m30()) + sq(m.m31()) + sq(m.m32()));
        return (translation + frobenius * local) * 1.001 + 0.001;
    }

    private static double sq(double v) {
        return v * v;
    }

    /** Whether the drawn view is the culling frustum's up to view bobbing (no hurt or death tilt, nausea or portal) */
    private static boolean cameraMatchesFrustum() {
        Minecraft minecraft = Minecraft.getInstance();
        Entity camera = minecraft.getCameraEntity();
        if (camera instanceof LivingEntity living && (living.hurtTime > 0 || living.deathTime > 0 || living.isDeadOrDying())) {
            return false;
        }
        LocalPlayer player = minecraft.player;
        return player == null || player.portalEffectIntensity <= 0.0F && player.oPortalEffectIntensity <= 0.0F
                && !player.hasEffect(MobEffects.NAUSEA);
    }
}
