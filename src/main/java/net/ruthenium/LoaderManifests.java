package net.ruthenium;

import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.nio.file.Path;
import java.security.CodeSource;
import java.util.Map;
import java.util.jar.Manifest;

/**
 * Fabric Loader's class loader (Knot) keeps a parsed copy of each code source's manifest ({@code KnotClassDelegate.metadataCache},
 * one {@code Metadata} per jar or directory). The 26.3 client jar runs unremapped with Mojang's signature, so its manifest has a
 * SHA-384 section for each of its 32875 entries: ~6 MB of {@code Attributes}, {@code HashMap}s and {@code String}s kept for the
 * whole session.
 * <p>
 * Nothing reads those sections (Fabric Loader 0.20.1): Knot defines classes with the cached {@code CodeSource} only and packages
 * with null attributes, and the cached manifest is otherwise only returned by {@code FabricLauncher.getManifest(Path)}, which
 * neither the loader nor Fabric API, Mixin or MixinExtras call. {@code JarURLConnection.getManifest()} gave Knot a deep copy
 * ({@code URLJarFile.getManifest}), so the JDK's verifier and its own cached jar file keep theirs. Each cached manifest with
 * per-entry sections is therefore replaced by one with its main attributes only, under the same {@code CodeSource}.
 * <p>
 * Runs once at client init, when the game jar's manifest is cached (its classes are loaded long before). Code sources first
 * used later keep their sections; they are small. If the loader's internals differ, it logs a warning and changes nothing.
 */
final class LoaderManifests {
    private LoaderManifests() {
    }

    static void trim() {
        try {
            ClassLoader knot = LoaderManifests.class.getClassLoader();
            Object delegate = read(knot.getClass(), "delegate", knot);
            @SuppressWarnings("unchecked")
            var cache = (Map<Path, Object>) read(delegate.getClass(), "metadataCache", delegate);

            Field manifestField = null;
            Field codeSourceField = null;
            Constructor<?> constructor = null;
            int manifests = 0;
            int sections = 0;
            for (var entry : cache.entrySet()) {
                Object metadata = entry.getValue();
                if (manifestField == null) {
                    Class<?> type = metadata.getClass();
                    manifestField = accessible(type.getDeclaredField("manifest"));
                    codeSourceField = accessible(type.getDeclaredField("codeSource"));
                    constructor = type.getDeclaredConstructor(Manifest.class, CodeSource.class);
                    constructor.setAccessible(true);
                }

                var manifest = (Manifest) manifestField.get(metadata);
                if (manifest == null || manifest.getEntries().isEmpty()) {
                    continue;
                }

                var trimmed = new Manifest();
                trimmed.getMainAttributes().putAll(manifest.getMainAttributes());
                Object replacement = constructor.newInstance(trimmed, codeSourceField.get(metadata));
                if (cache.replace(entry.getKey(), metadata, replacement)) {
                    manifests++;
                    sections += manifest.getEntries().size();
                }
            }
            Initializer.LOGGER.info("Dropped {} per-entry sections of {} jar manifests cached by the class loader", sections, manifests);
        } catch (ReflectiveOperationException | RuntimeException e) {
            Initializer.LOGGER.warn("Can't trim the class loader's cached jar manifests ({}); they keep their per-entry sections", e.toString());
        }
    }

    private static Object read(Class<?> type, String name, Object instance) throws ReflectiveOperationException {
        return accessible(type.getDeclaredField(name)).get(instance);
    }

    private static Field accessible(Field field) {
        field.setAccessible(true);
        return field;
    }
}
