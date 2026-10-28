package net.thallium.interfaces;

/**
 * Implemented by block entity tickers (and the vanilla block entities they tick) whose next tick is known to
 * change nothing.
 */
public interface IdleTicker {
    boolean thallium$isTickIdle();
}
