package net.thallium.interfaces;

public interface ParticleSpawnLimiter {

    void resetLastAddRejected();

    /**
     * @return true if the last particle added to the engine was dropped because its group can't take more
     * particles this tick
     */
    boolean wasLastAddRejected();
}
