package net.thallium.interfaces;

/** Whether a model was submitted twice for one render state in a frame ({@code ModelPoseReplay}) */
public interface LayeredModel {
    boolean thallium$isLayered();

    void thallium$setLayered();
}
