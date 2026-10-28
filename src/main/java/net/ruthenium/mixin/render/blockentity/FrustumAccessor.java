package net.ruthenium.mixin.render.blockentity;

import net.minecraft.client.renderer.culling.Frustum;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Invoker;

@Mixin(Frustum.class)
public interface FrustumAccessor {

    /**
     * @return -2 if fully inside, -1 if intersecting, otherwise the index of the plane the box is outside of
     */
    @Invoker("cubeInFrustum")
    int invokeCubeInFrustum(double minX, double minY, double minZ, double maxX, double maxY, double maxZ);
}
