package net.ruthenium.interfaces;

/** Whether a model was submitted twice for one render state in a frame ({@code ModelPoseReplay}) */
public interface LayeredModel {
    boolean ruthenium$isLayered();

    void ruthenium$setLayered();
}
