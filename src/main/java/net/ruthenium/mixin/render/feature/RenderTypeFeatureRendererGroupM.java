package net.ruthenium.mixin.render.feature;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.renderer.StagedVertexBuffer;
import net.minecraft.client.renderer.rendertype.PreparedRenderType;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.PosedDrawGroup;
import net.ruthenium.interfaces.PosedDrawHolder;
import net.ruthenium.render.entity.PlayerPoseDraws;
import net.ruthenium.render.feature.DrawKey;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Finds a group's existing draw for a prepared render type with a hash lookup instead of vanilla's linear
 * {@code indexOf} over the group's draws, whose record {@code equals} is deep. The map holds the first index of each
 * prepared type, so it returns the draw {@code indexOf} would.
 * <p>
 * Vanilla's lookup never matches: {@code ScissorState} has no {@code equals} and {@code prepare()} copies it into a
 * new instance every call, so every render type switch adds a draw. With {@code mergeDraws} the map is keyed by
 * {@link DrawKey}, which compares the scissor state by value, so geometry of a render type that a group can reorder
 * joins its first draw with equal state (same pipeline, textures, transform slice and scissor rectangle), as the
 * lookup intends.
 * <p>
 * Also gives {@link PlayerPoseDraws} the group's draws: posed draws are added like {@code getVertexBuilder} adds them,
 * and a vanilla write that joins a posed draw writes its posed members first.
 */
@Mixin(targets = "net.minecraft.client.renderer.feature.RenderTypeFeatureRenderer$Group")
public class RenderTypeFeatureRendererGroupM implements PosedDrawGroup {
    @Shadow @Final private StagedVertexBuffer stagedBuffer;
    @Shadow @Final private boolean canReorder;
    @Shadow @Final private List<PreparedRenderType> drawRenderTypes;
    @Shadow private @Nullable RenderType lastRenderType;
    @Shadow private StagedVertexBuffer.@Nullable Draw lastDraw;

    @Shadow
    private StagedVertexBuffer.Draw getOrAddDraw(RenderType renderType) {
        throw new AssertionError();
    }

    /** Read once per group, so the map always covers the whole list when it is used */
    @Unique private final boolean mergeDraws = Initializer.CONFIG.mergeDraws;
    @Unique private final boolean hashedLookup = this.mergeDraws || Initializer.CONFIG.hashedDrawLookup;
    @Unique private final Map<Object, Integer> drawIndices = new HashMap<>();

    @WrapOperation(method = "getOrAddDraw", at = @At(value = "INVOKE", target = "Ljava/util/List;indexOf(Ljava/lang/Object;)I"))
    private int lookUpDraw(List<?> list, Object renderType, Operation<Integer> original) {
        if (!this.hashedLookup) {
            return original.call(list, renderType);
        }
        Integer index = this.drawIndices.get(this.key((PreparedRenderType) renderType));
        return index != null ? index : -1;
    }

    @WrapOperation(method = "getOrAddDraw", at = @At(value = "INVOKE", target = "Ljava/util/List;add(Ljava/lang/Object;)Z"))
    private boolean addDraw(List<?> list, Object element, Operation<Boolean> original) {
        if (this.hashedLookup && list == this.drawRenderTypes) {
            this.drawIndices.putIfAbsent(this.key((PreparedRenderType) element), list.size());
        }
        return original.call(list, element);
    }

    @Unique
    private Object key(PreparedRenderType renderType) {
        return this.mergeDraws ? DrawKey.of(renderType) : renderType;
    }

    @Inject(method = "<init>", at = @At("RETURN"))
    private void beginPoseGroup(CallbackInfo ci) {
        PlayerPoseDraws.beginGroup(this);
    }

    /**
     * Geometry joining a posed draw (the only way a posed draw gets a vertex builder): its posed members are written into
     * it first. A redirect, so that the render type switches of every group pay no callback object
     */
    @Redirect(method = "getVertexBuilder", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/StagedVertexBuffer;getVertexBuilder(Lnet/minecraft/client/renderer/StagedVertexBuffer$Draw;)Lcom/mojang/blaze3d/vertex/VertexConsumer;"))
    private VertexConsumer materializePoses(StagedVertexBuffer buffer, StagedVertexBuffer.Draw draw) {
        VertexConsumer builder = buffer.getVertexBuilder(draw);
        PlayerPoseDraws.PoseDraw posed = ((PosedDrawHolder) draw).ruthenium$poseDraw();
        if (posed != null) {
            PlayerPoseDraws.materialize(posed, builder);
        }
        return builder;
    }

    @Override
    public boolean ruthenium$canReorder() {
        return this.canReorder;
    }

    @Override
    public StagedVertexBuffer.@Nullable Draw ruthenium$joiningDraw(RenderType renderType) {
        // getVertexBuilder keeps the last draw for the same render type when consecutive geometry consolidates
        return this.lastDraw != null && this.lastRenderType == renderType && renderType.canConsolidateConsecutiveGeometry() ? this.lastDraw : null;
    }

    @Override
    public StagedVertexBuffer.Draw ruthenium$addDraw(RenderType renderType) {
        StagedVertexBuffer.Draw draw = this.getOrAddDraw(renderType);
        this.lastDraw = draw;
        this.lastRenderType = renderType;
        return draw;
    }

    @Override
    public PreparedRenderType ruthenium$lastPreparedType() {
        return this.drawRenderTypes.getLast();
    }
}
