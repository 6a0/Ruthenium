package net.rubidium.mixin.world;

import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.rubidium.interfaces.SectionChangeStamp;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/** Stamps the sections of client chunks on block changes and chunk data, for {@code SupportingBlocks} */
@Mixin(LevelChunk.class)
public abstract class BlockChangeTrackingM {
    @Shadow @Final private Level level;

    @Inject(method = "setBlockState", at = @At("HEAD"))
    private void blockChanged(BlockPos pos, BlockState state, int flags, CallbackInfoReturnable<BlockState> cir) {
        if (this.level.isClientSide()) {
            int index = ((LevelChunk) (Object) this).getSectionIndex(pos.getY());
            LevelChunkSection[] sections = ((LevelChunk) (Object) this).getSections();
            if (index >= 0 && index < sections.length) {
                ((SectionChangeStamp) sections[index]).rubidium$changed();
            }
        }
    }

    @Inject(method = "replaceWithPacketData", at = @At("HEAD"))
    private void chunkReplaced(CallbackInfo ci) {
        for (LevelChunkSection section : ((LevelChunk) (Object) this).getSections()) {
            ((SectionChangeStamp) section).rubidium$changed();
        }
    }
}
