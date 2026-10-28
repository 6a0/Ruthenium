package net.thallium.render.entity;

/**
 * The block and sky light at an entity's light probe position, which {@code EntityRenderer.getPackedLightCoords} reads
 * from the client's light engine for every entity and frame, kept on the entity while its position, its level and
 * the client's light data stay the same.
 * <p>
 * Readers see a light engine's data through its storage's visible map, which only changes when
 * {@code LayerLightSectionStorage.swapSectionMap} publishes a new copy (writes go to the updating map, copying each
 * section first). The client's storages bump {@link #generation()} whenever they publish one, so a kept value is used
 * only while no light data changed since it was read: the same value the lookup returns. Render thread only.
 */
public final class EntityLight {
    private static volatile int generation = 1;

    private EntityLight() {
    }

    public static int generation() {
        return generation;
    }

    /** The client's light data changed (or kept values must go): every kept value is stale */
    public static void invalidate() {
        int next = generation + 1;
        generation = next == 0 ? 1 : next;
    }
}
