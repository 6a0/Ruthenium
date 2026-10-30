package net.ruthenium.mixin.render.feature;

import it.unimi.dsi.fastutil.ints.Int2ObjectAVLTreeMap;
import net.minecraft.client.renderer.SubmitNodeCollection;
import net.minecraft.client.renderer.SubmitNodeStorage;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/**
 * Every submit of the storage goes through {@code order(0)}, whose {@code computeIfAbsent} gets a new capturing lambda
 * per call; an order that has its collection already returns it without one (what {@code computeIfAbsent} returns)
 */
@Mixin(SubmitNodeStorage.class)
public class SubmitNodeStorageOrderM {
    @Shadow @Final private Int2ObjectAVLTreeMap<SubmitNodeCollection> submitsPerOrder;

    @Inject(method = "order", at = @At("HEAD"), cancellable = true)
    private void existingOrder(int order, CallbackInfoReturnable<SubmitNodeCollection> cir) {
        SubmitNodeCollection collection = this.submitsPerOrder.get(order);
        if (collection != null) {
            cir.setReturnValue(collection);
        }
    }
}
