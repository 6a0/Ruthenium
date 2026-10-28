package net.ruthenium.render.entity;

import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.model.Model;
import net.minecraft.client.model.geom.ModelPart;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.LayeredModel;
import net.ruthenium.interfaces.PoseRecordHolder;
import net.ruthenium.interfaces.ReplayablePart;
import net.ruthenium.mixin.render.entity.model.PoseAccessor;
import org.joml.Matrix3f;
import org.joml.Matrix4f;
import org.jspecify.annotations.Nullable;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * Layers that submit a model again with the same render state and pose (villager clothing, spider and enderman eyes,
 * horse markings, wolf and cat collars) take the poses of its parts from the model's first submit of that state
 * instead of posing the model again ({@code setupAnim} and the part tree).
 * <p>
 * Every model submit of an entity render state stamps the state with its model (two per state); a second submit of
 * the same model and state in a frame marks the model as layered. From then on, the first submit of a layered
 * model for a state records the pose of each part it writes (copies of the part tree's poses), and later submits
 * of that model and state with a bit-identical submit pose write the same parts with the recorded poses. Vanilla
 * models pose their parts from the render state alone ({@code setupAnim} starts with {@code resetPose}), so the
 * poses are those the layer would compute; only vanilla model classes are recorded. Render thread only.
 */
public final class ModelPoseReplay {
    private static final List<Record> POOL = new ArrayList<>();
    private static int used;
    private static int frame = 1;
    private static @Nullable Record recording;

    private ModelPoseReplay() {
    }

    /** One model's submit for a render state in a frame, and the poses of its parts if they were recorded */
    public static final class Record {
        int frame;
        @Nullable Object state;
        @Nullable Model<?> model;
        boolean complete;
        final Matrix4f submitPose = new Matrix4f();
        final Matrix3f submitNormal = new Matrix3f();
        int submitProperties;
        boolean submitTrustedNormals;
        ModelPart[] parts = new ModelPart[0];
        PoseStack.Pose[] poses = new PoseStack.Pose[0];
        int count;

        boolean matches(PoseStack.Pose pose) {
            return pose.pose().properties() == this.submitProperties && ((PoseAccessor) (Object) pose).ruthenium$trustedNormals() == this.submitTrustedNormals
                    && equalBits(pose.pose(), this.submitPose) && equalBits(pose.normal(), this.submitNormal);
        }

        void add(ModelPart part, PoseStack.Pose pose) {
            if (this.count == this.parts.length) {
                int size = Math.max(16, this.count * 2);
                this.parts = Arrays.copyOf(this.parts, size);
                this.poses = Arrays.copyOf(this.poses, size);
            }
            PoseStack.Pose copy = this.poses[this.count];
            if (copy == null) {
                copy = this.poses[this.count] = new PoseStack.Pose();
            }
            copy.set(pose);
            this.parts[this.count++] = part;
        }

        /** Writes the recorded parts as {@code ModelPart.compile} would in the part tree */
        public void replay(VertexConsumer buffer, int lightCoords, int overlayCoords, int color) {
            ModelPart[] parts = this.parts;
            PoseStack.Pose[] poses = this.poses;
            for (int i = 0, n = this.count; i < n; i++) {
                ((ReplayablePart) (Object) parts[i]).ruthenium$compile(poses[i], buffer, lightCoords, overlayCoords, color);
            }
        }
    }

    /** Called before the world's features are prepared */
    public static void beginFrame() {
        frame++;
        for (int i = 0; i < used; i++) {
            Record record = POOL.get(i);
            record.state = null;
            record.model = null;
            Arrays.fill(record.parts, 0, record.count, null);
        }
        used = 0;
        recording = null;
    }

    /**
     * A model submit is about to pose its parts.
     *
     * @return the record of an earlier submit of the model and state to replay instead, or null to pose the model
     * (then call {@link #end})
     */
    public static @Nullable Record begin(Model<?> model, Object state, PoseStack.Pose pose) {
        recording = null;
        // The world's features only: the frame (and the record pool) advances with them
        if (!(state instanceof PoseRecordHolder holder) || !ModelCulling.isWorldPass() || !Initializer.CONFIG.reuseLayerPoses) {
            return null;
        }
        Record first = holder.ruthenium$getPoseRecord(0);
        Record second = holder.ruthenium$getPoseRecord(1);
        // Records are pooled across frames, so a state kept across frames (reused render states) may hold one that is now another's
        Record mine = first != null && first.frame == frame && first.state == state && first.model == model ? first
                : second != null && second.frame == frame && second.state == state && second.model == model ? second : null;
        if (mine != null) {
            if (mine.complete) {
                return mine.matches(pose) ? mine : null;
            }
            // A second submit of the model and state: record the model's first submits from the next frame on
            ((LayeredModel) model).ruthenium$setLayered();
            return null;
        }

        int slot;
        if (first == null || first.frame != frame || first.state != state) {
            slot = 0;
        } else if (second == null || second.frame != frame || second.state != state) {
            slot = 1;
        } else {
            return null;
        }
        Record record = take();
        record.frame = frame;
        record.state = state;
        record.model = model;
        record.complete = false;
        record.count = 0;
        holder.ruthenium$setPoseRecord(slot, record);
        if (((LayeredModel) model).ruthenium$isLayered() && model.getClass().getName().startsWith("net.minecraft.")) {
            record.submitPose.set(pose.pose());
            record.submitNormal.set(pose.normal());
            record.submitProperties = pose.pose().properties();
            record.submitTrustedNormals = ((PoseAccessor) (Object) pose).ruthenium$trustedNormals();
            recording = record;
        }
        return null;
    }

    /** The model submit has been posed and written */
    public static void end() {
        Record record = recording;
        if (record != null) {
            record.complete = true;
            recording = null;
        }
    }

    /** Called when a part is written ({@code ModelPart.compile}) */
    public static void record(ModelPart part, PoseStack.Pose pose) {
        Record record = recording;
        if (record != null) {
            record.add(part, pose);
        }
    }

    public static boolean isRecording() {
        return recording != null;
    }

    private static Record take() {
        if (used == POOL.size()) {
            POOL.add(new Record());
        }
        return POOL.get(used++);
    }

    private static boolean equalBits(Matrix4f a, Matrix4f b) {
        return bits(a.m00()) == bits(b.m00()) && bits(a.m01()) == bits(b.m01()) && bits(a.m02()) == bits(b.m02()) && bits(a.m03()) == bits(b.m03())
                && bits(a.m10()) == bits(b.m10()) && bits(a.m11()) == bits(b.m11()) && bits(a.m12()) == bits(b.m12()) && bits(a.m13()) == bits(b.m13())
                && bits(a.m20()) == bits(b.m20()) && bits(a.m21()) == bits(b.m21()) && bits(a.m22()) == bits(b.m22()) && bits(a.m23()) == bits(b.m23())
                && bits(a.m30()) == bits(b.m30()) && bits(a.m31()) == bits(b.m31()) && bits(a.m32()) == bits(b.m32()) && bits(a.m33()) == bits(b.m33());
    }

    private static boolean equalBits(Matrix3f a, Matrix3f b) {
        return bits(a.m00()) == bits(b.m00()) && bits(a.m01()) == bits(b.m01()) && bits(a.m02()) == bits(b.m02())
                && bits(a.m10()) == bits(b.m10()) && bits(a.m11()) == bits(b.m11()) && bits(a.m12()) == bits(b.m12())
                && bits(a.m20()) == bits(b.m20()) && bits(a.m21()) == bits(b.m21()) && bits(a.m22()) == bits(b.m22());
    }

    private static int bits(float value) {
        return Float.floatToRawIntBits(value);
    }
}
