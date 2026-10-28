package net.rubidium.interfaces;

/** Whether a model was submitted twice for one render state in a frame ({@code ModelPoseReplay}) */
public interface LayeredModel {
    boolean rubidium$isLayered();

    void rubidium$setLayered();
}
