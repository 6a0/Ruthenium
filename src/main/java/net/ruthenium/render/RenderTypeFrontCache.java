package net.ruthenium.render;

import net.ruthenium.Initializer;
import net.ruthenium.config.Config;

import java.util.function.BiFunction;

/**
 * A front cache for the two-argument {@code Util.memoize} lookups of {@code RenderTypes} ({@code entityCutout(texture,
 * affectsOutline)} and the other {@code (Identifier, Boolean)} / {@code (Identifier, Identifier)} render types). Vanilla's
 * memo builds a {@code Pair} key, a capturing lambda and the pair's hash array on every call, then looks the pair up in a
 * {@code ConcurrentHashMap}; every living entity looks up its render type each frame
 * ({@code LivingEntityRenderer.getRenderType}), so in the mixed scene those were ~7% of the render thread's allocation.
 * <p>
 * Exact: the cache holds what the vanilla memo (the delegate) returned for a key, and answers only for an equal key
 * ({@code equals} on both arguments, as the memo's {@code Pair.equals}). The memo never forgets or replaces an entry, so
 * for an equal key it returns that same instance forever. A miss, a {@code null} argument or the switch off
 * ({@code renderTypeCache}) asks the memo. Two-way set associative and bounded, and safe from any thread: an entry is an immutable
 * object stored into the array with one reference write.
 */
public final class RenderTypeFrontCache<T, U, R> implements BiFunction<T, U, R> {
    /** Sets of two entries, the most recently added first */
    private static final int SETS = 512;

    private final BiFunction<T, U, R> memo;
    private final Entry<?, ?, ?>[] entries = new Entry<?, ?, ?>[SETS * 2];

    private record Entry<T, U, R>(T first, U second, R result) {
        boolean matches(Object first, Object second) {
            return (this.first == first || this.first.equals(first)) && (this.second == second || this.second.equals(second));
        }
    }

    public RenderTypeFrontCache(BiFunction<T, U, R> memo) {
        this.memo = memo;
    }

    @Override
    @SuppressWarnings("unchecked")
    public R apply(T first, U second) {
        Config config = Initializer.CONFIG;
        if (first == null || second == null || config == null || !config.renderTypeCache) {
            return this.memo.apply(first, second);
        }
        int hash = first.hashCode() * 31 + second.hashCode();
        int index = ((hash ^ (hash >>> 9) ^ (hash >>> 18)) & (SETS - 1)) * 2;
        Entry<?, ?, ?> newest = this.entries[index];
        Entry<?, ?, ?> entry = newest;
        if (entry != null && !entry.matches(first, second)) {
            entry = this.entries[index + 1];
            if (entry != null && !entry.matches(first, second)) {
                entry = null;
            }
        }
        if (entry != null) {
            return (R) entry.result;
        }
        R result = this.memo.apply(first, second);
        this.entries[index + 1] = newest;
        this.entries[index] = new Entry<>(first, second, result);
        return result;
    }

    @Override
    public String toString() {
        return this.memo.toString();
    }
}
