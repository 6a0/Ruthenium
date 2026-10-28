package net.ruthenium.world;

import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.vehicle.minecart.AbstractMinecart;
import net.minecraft.world.level.entity.EntityTypeTest;

/** Typed entity queries (statics live here, not in the mixins: see the design rules in CONTINUATION.md) */
public final class EntityTypeTests {
    public static final EntityTypeTest<Entity, AbstractMinecart> MINECARTS = EntityTypeTest.forClass(AbstractMinecart.class);

    private EntityTypeTests() {
    }
}
