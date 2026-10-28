package net.vulkanmod.interfaces;

/** Whether a model was submitted twice for one render state in a frame ({@code ModelPoseReplay}) */
public interface LayeredModel {
    boolean vulkanmod$isLayered();

    void vulkanmod$setLayered();
}
