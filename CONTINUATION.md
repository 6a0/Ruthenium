# VulkanMod → Minecraft 26.3 port: continuation notes

This is a hand-off document for a new session. It covers what was decided, what has been built, how to test it,
what the numbers are, and what to do next. Read it all before changing code.

Last updated: 2026-10-26 (third session), at commit `7b4aff1` plus the commit that updates this file.

---

## 1. TL;DR

- **Branch:** `26.3`. The old 1.21.11 code is still on `dev` (upstream `xCollateral/VulkanMod`), and
  `git show dev:<path>` is how to pull old sources back in when porting them.
- **Strategy:** port VulkanMod's *optimisations* only. The mod no longer ships its own Vulkan device layer.
  26.x has a new GPU abstraction (`com.mojang.renderpearl`) with its own Vulkan backend, and the mod runs on
  vanilla's backends (GL or Vulkan) through public renderpearl and vanilla APIs.
- **Testing:** every change is benchmarked with the automated client gametest (`./gradlew benchVulkanMod`,
  3 runs) and checked for visual parity against vanilla screenshots, then committed with the numbers in the
  commit message. The user asked for this workflow: *commit as you test changes with the benchmark*.
- **Results so far** (median of 3 runs, vanilla Vulkan backend): static view **25 → 415 fps**, pan **61 → 516 fps**
  (pan p99 low 18 → 117). Sodium 0.9.3-beta.1 on the same benchmark: 63 / 163. On vanilla's OpenGL backend
  (`benchVulkanModGl`) the static view runs at 344 fps.
- **Closed chests are retained on the GPU** (`RetainedChests`, the user chose this in the third session): their
  geometry lives in per-section vertex buffers and is drawn with a copy of the entity pipeline that adds
  `ModelOffset`. Kill switch: `-Dvulkanmod.retainedChests=false` (`-Pvm.retainedChests=false` on bench tasks).
- **Closed enchanting table books and armor stands are GPU instances** (`InstancedModels`): model-space mesh once
  per group, a pose + light per instance per frame. Kill switch: `-Dvulkanmod.instancedModels=false`.
- **The pan p99 is not hitches.** Frames facing the chest cube (yaw 220–320, 11–13k block entities) take
  ~6 ms (third session start: ~20 ms) while the rest of the pan runs at ~1 ms (section 7). The remaining work
  there is per block entity (signs, enchanting table extraction, ticking).
- **Don't trust JFR's time attribution** (the user warned about it, and it was wrong again in the third session).
  Use the section profiler (`-Pbench.sectionProfile=true`) and temporary `System.nanoTime` counters to rank work;
  see section 6.6.
- **Next:** see section 8.

---

## 2. Background: what 26.x changed

Three read-only audits compared the 1.21.11 and 26.3 decompiled sources. The key facts:

| Area | 1.21.11 | 26.3 |
|---|---|---|
| GPU API | `com.mojang.blaze3d.systems.GpuDevice` etc.; old VulkanMod implemented these (`VkGpuDevice`…) | `com.mojang.renderpearl.api` (public) → `renderpearl.frontend` (validation, `FrontendGpuDevice`) → `renderpearl.backend.api` (`GpuDeviceBackend`, `CommandEncoderBackend`, `RenderPassBackend`, `GpuSurfaceBackend`) with the `backend.opengl` and `backend.vulkan` implementations |
| Backend choice | — | `Minecraft.<init>` (~line 466–530) tries `PreferredGraphicsApi.getBackendsToTry()`. The launch arg `--graphicsBackend opengl\|vulkan` forces one. Vulkan is experimental in 26.2/26.3 and the default in 26.4 snapshots |
| Shaders | VulkanMod's own GLSL→SPIR-V converter | Vanilla compiles GLSL→SPIR-V (`renderpearl.frontend.shaders.GlslCompiler`, shaderc + spvc reflection) and hands backends SPIR-V |
| Windowing | GLFW | **SDL3** (`lwjgl-sdl`); lwjgl-glfw is **not** on the classpath. LWJGL 3.4.3, Java 25 |
| Depth | normal | **Reversed-Z**: clear to 0.0, `GREATER_THAN_OR_EQUAL` |
| Terrain | `LevelRenderer` | Split into `renderer.extract.LevelExtractor` (main thread: visibility, dirty sections, block-entity extraction) and `LevelRenderer.render` (draw only). Uber buffers and MultiDrawIndirect (`prepareChunkRendersIndirect`). Chunk layers are SOLID/CUTOUT/TRANSLUCENT (TRIPWIRE removed). "Improved transparency" is OIT |
| Block models | `BakedQuad` class, `ItemBlockRenderTypes` | `BakedQuad` is a record with `MaterialInfo` (render layer per quad, tint, shade override, emission). `BlockColor` → `BlockTintSource` lists. `CardinalLighting` |
| GUI | `GuiGraphics`, `Gui` | `GuiGraphicsExtractor`, `Hud` (the old Gui), a new `Gui` (screen owner: `minecraft.gui.setScreen`) |
| Fabric Renderer API | v1 8.0.0 | 17.0.17, package `net.fabricmc.fabric.api.client.renderer.v1`. `Renderer` has 3 methods (`quadEmitter`, `mutableMesh`, `altModelBlockRenderer`). Indigo internals the old code used are gone |
| Mappings | Mojang mappings via Loom | Unobfuscated; no mappings |
| Misc renames | `LightTexture` | `Lightmap`; `DynamicUniforms` → `DynamicGpuData`; `TextureFormat` → `renderpearl.api.GpuFormat`; `VertexFormat`/`VertexFormatElement` → `renderpearl.api.vertex`; `VertexFormat.Mode` → `renderpearl.api.pipeline.PrimitiveTopology`; `IndexType` → `renderpearl.api.pipeline.IndexType` |

Intermediary method names (`method_xxxxx`) no longer exist anywhere. Any old mixin that targeted lambdas by
intermediary name has to be retargeted.

## 3. The decision

Two options were weighed (see the chat history if available):

- **A. Custom backend:** implement `GpuBackend` + `GpuDeviceBackend` with VulkanMod's device code, wrapped by
  vanilla's `FrontendGpuDevice`, and inject it via `PreferredGraphicsApi.getBackendsToTry()`. This is an XL rewrite
  (command encoder, render pass, swapchain on SDL, device created before the window, transient memory, layout
  tracking).
- **B. Optimisations only.** **Chosen by the user on 2026-10-26.** Write optimisations against vanilla and the
  public renderpearl API, so they work on both vanilla backends.

Revisit A only if profiling shows vanilla's Vulkan backend itself is the bottleneck. A terrain renderer written
against renderpearl would still work with a custom backend, so B is not a dead end.

**Sodium 26.3 exists** (`mc26.3-0.9.3-beta.1`), so the mod has to beat it to be worth using. It currently does on
the benchmark scene. **Sodium is PolyForm Shield licensed: do not copy its code.** Profiling Sodium for ideas is
fine; implementations must be our own.

---

## 4. Current state of the branch

```
7b4aff1 Draw armor stands posed alike as GPU instances
fe7306b Update continuation notes with the instanced books
447a7c9 Draw closed enchanting table books as GPU instances
4aca36a Update continuation notes with the retained chests
dfb1e97 Add a benchmark task for this mod on vanilla's OpenGL backend
24d1189 Keep closed chest geometry on the GPU across frames
caeb831 Update continuation notes with the third session's changes
bf555e7 Skip block entity ticks that would do nothing
ba2855a Extract visible block entities without loading culled ones
ca038aa Speed up model space writes and pair vertex stores
a6d5d24 Skip sign text on sides facing away from the camera
b96ae59 Copy staged vertex data into the staging buffer on a second thread
8547630 Add a section profiler option to the benchmark
3b68123 Cache closed enchanting table books in model space
200bd24 Update continuation notes with the culling refinements
436c1dd Cull block entities from packed section positions
311a059 Classify 4x4x4 cells in sections that cross the frustum
5738073 Update continuation notes after the second session
70b8b99 Keep sign text caches on the sign lines instead of a global map
2efa2dd Add a static phase yaw option and time windows to jfr_tree
12b63f8 Reuse rest pose model geometry across submits
4dbc8de Skip back faces of culled model cubes on the CPU
51715ed Cache sign text width and glyph layout across frames
a23cf6b Write sorted quad indices directly to memory
6a8e464 Classify sections before per block entity culling
eb5cbc4 Record per-frame pan series and a yaw 270 screenshot in the benchmark
3ea406d Add port continuation notes and benchmark analysis scripts
ce23d4c Frustum cull block entities individually
3b1173e Stop spawning particles their group can't take
a642fed Add direct vertex writing fast paths for entities, particles and text
ae1fe5a Add client gametest benchmark harness
bb25d7a Port build to Minecraft 26.3 and strip the 1.21.11 renderer
087ef24 Refactor PipelineState   <- last 1.21.11 commit (dev)
```

### New code (`src/main/java/net/vulkanmod/`)

| File | What it does |
|---|---|
| `Initializer.java` | Minimal client entrypoint (version and logging only). The old config, Platform and FRAPI registration are not ported yet |
| `interfaces/ExtendedVertexBuilder.java` | `canWriteDirect(VertexFormat)` and `reserveVertices(int)`, for writing whole quads into a `BufferBuilder` |
| `mixin/render/vertex/BufferBuilderM.java` | Implements the above (handles the `ensureBuilding`/`endLastVertex`/vertex count/`vertexPointer` bookkeeping; LE only; non-LINES topology) |
| `mixin/render/entity/model/ModelPartCubeM.java` | `@Overwrite Cube.compile`: transforms the distinct corners once, writes ENTITY-format faces directly, unwraps `SpriteCoordinateExpander`; vanilla fallback for other formats |
| `mixin/render/particle/QuadParticleRenderStateM.java` | `@Overwrite renderRotatedQuad`: rotates 2 axes and writes PARTICLE-format quads directly |
| `mixin/render/text/BakedSheetGlyphM.java` | `@Overwrite render(...)`: direct writes for POSITION_TEX_LIGHTMAP_COLOR (world text) and POSITION_TEX_COLOR (GUI text) |
| `mixin/render/vertex/VertexSortingM.java` + `render/vertex/DistanceSorting.java` | Replaces `VertexSorting.byDistance` with a packed-long sort (<256) or a stable LSD radix sort with per-thread scratch. Output is **identical** to vanilla (tested against the real 26.3 class) |
| `mixin/render/particle/ParticleEngineM.java` + `ClientPacketListenerM.java` + `interfaces/ParticleSpawnLimiter.java` | Drops particles whose group is full (exact) and caps queued candidates per group per tick at 8×16384 (peak count within ~1% of vanilla). The packet loop stops once dropping starts |
| `mixin/render/blockentity/LevelExtractorM.java` + `FrustumAccessor.java` + `render/blockentity/BlockEntityCulling.java` | `@Overwrite extractVisibleBlockEntities`: vanilla's loop with per-block-entity frustum culling inlined. Conservative box: block ±1, +2 extra on top. **Vanilla block entity types only.** Each section is classified once (inside/outside/partial with the same margins), then 4³ cells of partial sections, and only block entities in partial cells are tested individually. Positions and cullability come from `CompiledSectionMeshM` (`interfaces/BlockEntityPositions`, packed once per compiled mesh) by list index, so culled block entities are not even loaded from the list. The destruction progress map is skipped while empty. Fabric API doesn't inject into this method (checked) |
| `mixin/render/vertex/MeshDataSortStateM.java` | `@Overwrite` `writeSortedIndexBuffer`/`buildSortedIndexBuffer`: sorted quad indices written with `MemoryUtil` puts instead of an `IntConsumer` per index. Sign text (`text_polygon_offset`) is sorted on upload and needs 32-bit indices with ~500 signs |
| `render/text/SignTextCache.java` + `mixin/render/text/{SignTextM,AbstractSignRendererM,TextFeatureRendererM,FontSetM}.java` | `SignText.getRenderMessages` wraps each line (only when recomputed) in a `FormattedCharSequence` that carries its width and `Font.PreparedText` (plain/shadowed/outline, keyed by x/y/colours). Obfuscated lines are not cached; `FontSet.resetTextures` bumps a generation. A first version with a global identity map was slower than no cache. `AbstractSignRendererM` also skips the text of a sign side whose plane faces away from the eye by ≥ 0.55 blocks (`ModelCulling.isSignTextHidden`): polygon-offset text pipelines (and their OIT sets) cull back faces, checked at runtime. Glyph quads wind like (0,0),(0,1),(1,1), so the side's facing is `column1 × column0` of the pose |
| `render/entity/ModelCulling.java` + `mixin/render/feature/LevelRendererM.java` | Flags the world's `FeatureRenderDispatcher.prepareFrame`, `LevelRenderer.submitBlockEntities` (for sign text) and the current model's render type. When the pipeline culls (`RenderPipeline.isCull()`), `ModelPartCubeM` skips faces the GPU would cull: `(v1-v0)x(v2-v0)` of the transformed corners points away from every eye position within 0.5 blocks of the origin (camera-relative positions; bobbing moves the eye ~0.1). Degenerate faces are kept. `skipDuplicateQuads()` is true for render types without blending or upload sorting (drawing a quad twice is then idempotent) |
| `render/entity/ModelGeometryCache.java` + `mixin/render/feature/ModelFeatureRendererM.java` | `@Overwrite prepareModel`. Models whose state is `Float` 0 (closed chests, …) are captured once per pose rotation + normal matrix, overlay, tint and `UvMapping` (translation removed, through a recording `VertexConsumer` on the vanilla path), then written with the translation added, the submit's light and precomputed face planes for the back-face test. **Model space entries** (`modelSpaceKey`: currently closed `BookModel.State`, openness 0, where the page flips move nothing) are captured with the identity pose and transformed per write: back faces are tested in model space (eye moved into model space when the pose's linear part is a similarity, else the transformed corners are tested), only corners of written quads are transformed, exact duplicate quads are skipped when allowed. Vertices are written as 4 longs + 1 int. Only rounding differs from vanilla; a temporary check against vanilla's model rendering of 214k book submits found no extra quads and only culled or duplicate missing ones. Max 16 entries per model; models whose capture fails are remembered |
| `mixin/render/vertex/StagedVertexBufferM.java` + `render/vertex/ParallelUploadCopy.java` | Redirects the per-slice `ByteBuffer.put` in `StagedVertexBuffer.uploadDrawsToBuffers` to record the copies; a worker thread and the render thread copy them in 1 MB chunks, starting when the first sorted draw is decoded (sorting overlaps the copy) and joined before the staging buffer is unmapped. Each upload is its own `Job`, so a late worker can't take chunks of the next upload. 1 worker: 3.3 → 1.9 ms for 34 MB; more workers don't help (memory bandwidth) |
| `render/blockentity/RetainedChests.java` + `ModelSubmitCapture.java` + `mixin/render/blockentity/{ClientLevelM,BlockEntityRenderDispatcherM,RenderPipelinesM}.java` + `assets/vulkanmod/shaders/core/retained_entity.vsh` | Closed chests kept on the GPU. Per render section (keyed by `SectionPos`), all chests at rest (exact vanilla chest/trapped/ender classes, `ChestRenderer` exactly, lid idle incl. the other half of a double chest, no breaking progress, one `Float` 0 model on `ENTITY_CUTOUT_CULL`) are built into one vertex buffer relative to the section origin: the real renderer runs into `ModelSubmitCapture` (a `SubmitNodeCollector` that accepts exactly one model submit) and the model is written through `ModelGeometryCache` with all faces. `LevelExtractorM` skips members; batches are drawn after `featureFrame.executeSolid` in `LevelRendererM` with `PIPELINE` (vanilla `entity_cutout_cull` + a vertex shader adding `DynamicTransforms.ModelOffset`; registered via `RenderPipelines.optionalPipelines`, so a compile failure just turns it off). Transforms are written after `prepareFrame` with fresh vectors (`DynamicGpuDataStorageMapped` dedups against the last record by `equals`), and the sequential index buffer is grown there too (not inside a pass). Rebuilt when the section's mesh changes, when the section or a neighbour hits `LevelExtractor.setSectionDirty(IIIZ)` (block and light changes; a double chest's light can come from the neighbour), on chest `triggerEvent`, `ClientLevel.destroyBlockProgress`, and when a left out (moving) chest comes to rest. Sections partly within the chest view distance (64) use the vanilla path that frame. Budget 2048 chests rebuilt per frame; batches unused for 1200 frames are freed. Cleared on level change, `allChanged` and resource reload |
| `render/blockentity/InstancedModels.java` + `assets/vulkanmod/shaders/core/instanced_entity.vsh` | GPU instancing of models that only differ by pose. Two entry points: (1) the extraction (`LevelExtractorM`) calls `tryAdd(dispatcher, state, …)` after `tryExtractRenderState` for `EnchantTableRenderer`: the real renderer submits into `ModelSubmitCapture`, and a single model with a `modelSpaceKey` (closed book) becomes an instance instead of a submitted render state; (2) `ModelFeatureRendererM.prepareModel` calls `tryAdd(submit)` for models in `POSED_MODELS` (`ArmorStandModel`): after `setupAnim`, a snapshot of all parts' transforms/visibility keys the group (exact for pure `ModelPart` trees; max 32 groups per model) and the mesh is `model.root().render` with the identity pose and CPU culling off. An instance is 3 pose rows + light (52 B, `INSTANCE_FORMAT`, step rate 1). Poses must be orthogonal (rotations, or the `scale(-1,-1,1)` mirror of living entities, where vanilla's normal matrix equals the pose). Instanced copies of `entity_solid`, `entity_cutout` (per-face lighting, no cull) and `entity_cutout_cull` (`PIPELINES`). Per group (model, key, render type, UV mapping, overlay, tint): a model-space mesh from `ModelGeometryCache.writeModelSpace` (identity pose, all faces, duplicates skipped) and a `MappableRingBuffer` for the instances (rotated at the next prepare, so its fence covers the frame that used it). Drawn after `RetainedChests` with `PIPELINE` (vanilla `entity_solid` + an instanced vertex shader). Cleared with the retained chests |
| `mixin/world/{LevelM,BoundTickingBlockEntityM,RebindableTickingBlockEntityWrapperM,ChestBlockEntityM,EnderChestBlockEntityM,ChestLidControllerM,SignBlockEntityM}.java` + `interfaces/IdleTicker.java` | `Level.tickBlockEntities` skips tickers whose tick is a no-op: exact vanilla chest/trapped/ender chest classes with a closed lid that stays closed, and sign/hanging sign classes with no editing player. Also skips the world border, block state and validity checks in front of them. 1.6 → 1.0 ms per tick with ~14k block entities |

Design rules used so far:
- Hot paths use `@Overwrite` with an inlined vanilla fallback, because a cancellable `@Inject` allocates a
  `CallbackInfo` per call, which showed up in allocation profiles.
- Fast paths only engage for the exact vertex format they write (`ExtendedVertexBuilder.of(consumer, format)`).
- Output must match vanilla visually (screenshot diff, section 6.5). Skipping geometry is only done when the GPU
  provably discards it (back faces of culling pipelines, beyond a margin) or it is an exact, idempotent repeat.
- Static fields with initializers in a mixin class broke the target's instance field initialisation once
  (`cellStamps` was null); keep diagnostic statics in a separate plain class.

### Resources
- `vulkanmod.mixins.json`: client mixins listed above, `compatibilityLevel` `JAVA_25`.
- `vulkanmod.accesswidener`: header `classTweaker v1 official`, no entries yet.
- `fabric.mod.json`: depends on `minecraft ~26.3`, `java >=25`, `fabricloader >=0.19.3`. The
  `fabric-renderer-api-v1:contains_renderer` flag was removed until FRAPI is ported.
- `assets/vulkanmod/shaders/*` are leftover 1.21.11 shaders (Vulkan-flavoured GLSL). Nothing uses them yet;
  rework them when porting terrain.

### Not ported yet (all still on `dev`)
`config/**` (settings GUI and options), `render/chunk/**` (terrain renderer: section graph, AreaBuffer,
DrawBuffers, QuadFacing culling, light pipelines, FRAPI impl, fluid renderer), `render/vertex/**`
(`CustomVertexFormat`, `TerrainBuilder`, `QuadSorter`), `render/sky/CloudRenderer`, `render/profiling` (overlay),
`mixin/debug/**`, `mixin/screen/**`, `UpdateChecker`, `Platform`. The device, GL compat, window, wayland and
shader-converter code is **obsolete under strategy B** and should not be ported.

---

## 5. Build and tooling

- Gradle 9.8.2 (wrapper), Loom **1.19.3**, plugin id `net.fabricmc.fabric-loom` (no remap), Java 25. The machine
  has Oracle JDK 25.0.4.1 at `C:\Program Files\Java\jdk-25.0.4.1` (no `javap`/`jfr` on PATH; call them by full path).
- Dependencies use `implementation`, not `modImplementation`. There is no `mappings` line.
- `loom.runs.configureEach` adds **Mojang's launcher JVM args**, and the benchmark tasks add them too:
  `-XX:StackShadowPages=32`, `--add-exports java.base/jdk.internal.misc=ALL-UNNAMED`, and on Windows
  `-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump`.
  **Without them ~35% of launches crashed natively** (`0xC0000005` / `0xC0000409` / `0xC0000374`) during
  startup with no `hs_err` and no WER dump. This was found by comparing with Prism Launcher's launch, and 8 of 8
  launches were clean after adding the args. Loom's `ClientProductionRunTask` does not apply manifest JVM args.
- Use the **Bash tool** (Git Bash) to run benchmark tasks. One PowerShell-launched run crashed, though that may
  have been the crash above; Bash runs have been reliable since.
- In Git Bash, prefix Windows tools that take `/flags` with `MSYS_NO_PATHCONV=1` (e.g. `powercfg /q`).
- Write multi-line Python edit scripts to a file rather than a Bash heredoc; quotes inside heredocs broke once.

### Decompiled sources for reference
The previous session used temp copies that may be gone. To regenerate:
```bash
# Vineflower 1.12.0 from Maven Central
MC=~/.gradle/caches/fabric-loom/minecraftMaven/net/minecraft/minecraft-merged-deobf/26.3/minecraft-merged-deobf-26.3.jar
java -jar vineflower.jar -dgs=1 -log=WARN --only=com/mojang --only=net/minecraft/client "$MC" src-26.3
# 1.21.11 (Mojang-mapped) for comparing against the old code:
OLD=~/.gradle/caches/fabric-loom/minecraftMaven/net/minecraft/minecraft-merged/1.21.11-loom.mappings.1_21_11.layered+hash.2198-v2/minecraft-merged-1.21.11-loom.mappings.1_21_11.layered+hash.2198-v2.jar
```
Fabric API sources: `https://maven.fabricmc.net/net/fabricmc/fabric-api/<module>/<version>/<module>-<version>-sources.jar`
(26.3 BOM: `fabric-api-0.165.0+26.3.pom`; e.g. renderer-api-v1 `17.0.17+79385d0b5d`, indigo `9.1.25+b9d63e335d`,
client-gametest-api-v1 `6.0.11+131f8d475d`).

---

## 6. Benchmark

### 6.1 The user's protocol (fixed; don't change it)
1. Join **`62.141.62.9:10002`** as **`6a0`**. It is an offline-mode Velocity proxy running 26.3, and `6a0` has op.
   Offline UUID `476b897b-bd6d-3ab1-b22e-1c1126c9f94e`.
2. Run `/tp 6a0 -98.5 69 189.5 0 0` and wait 5 s for FPS to settle. This is the "initial" phase, which is measured.
3. Pan a full 360°.
4. Run `/particle minecraft:happy_villager -97.5 68 199 2 2 2 1 500000` (the user calls it a hellish entity
   stress test).
5. Report average, 95% low and 99% low FPS for each phase, plus allocation rate.

Use the Fabric client gametest API and the `fast-ip-ping` mod; without it, literal IPs add a ~5 s reverse DNS stall.

### 6.2 Implementation (`src/gametest`)
- `BenchmarkClientGameTest`: sets options (Fancy preset, render distance 16, unlimited fps, no vsync), connects,
  checks that `/tp` and `/particle` parse with the server's command tree (fails fast if `6a0` lost op), then
  teleports and runs **initial (5 s) → pan (10 s, driven per frame) → static (5 s, added reference at the
  particle spot) → particles (10 s)**. It writes JSON and TXT to `build/run/bench/results/<label>-<time>.*`, the
  pan's per-frame series (frame ms, yaw, entities, block entities, GCs so far) to
  `results/frames/<label>-<time>-pan.csv`, and screenshots to `build/run/bench/<label>/screenshots/`: one at
  yaw 270 (the chest cube, `bench.extraShotYaws`, taken between pan and static) and one of the static view.
- `FrameRecorder`: frame times from a `Minecraft.renderFrame` HEAD hook, allocation via
  `ThreadMXBean.getTotalThreadAllocatedBytes` (all threads) and the render thread, GC counts, and entities and
  block entities per frame (hooked at the end of `LevelExtractor.extract`).
- `PhaseResult`: avg fps, p95/p99 low (FPS at the 95th/99th percentile frame time), worst 5%/1% average,
  median and max frame time, MB/s.
- `DisplayKeepAwake`: `SetThreadExecutionState(ES_CONTINUOUS|ES_DISPLAY_REQUIRED|ES_SYSTEM_REQUIRED)` via FFM.
  The user's displays blank after 5 minutes; they asked for them to stay on during benchmarks. The in-process
  approach changes no system settings. A `powercfg` toggle was tried first and abandoned.
- `StartupProbe` (diagnostic): `-PbenchProbe=SECONDS` launches without gametest mode and quits after N seconds.
  It was used to bisect the startup crashes.

### 6.3 Running it
```bash
./gradlew benchVulkanMod          # this mod on vanilla's Vulkan backend (label vulkanmod)
./gradlew benchVanillaVulkan      # baselines: benchVanillaGl, benchVanillaVulkan, benchSodium (default backend = GL)
scripts/run-matrix.sh 3 benchVulkanMod   # N runs each, logs in build/bench-logs, prints BUILD/NTSTATUS per run
python scripts/bench_summary.py build/run/bench/results   # medians per label and phase
```
`./gradlew benchVulkanModGl` runs this mod on vanilla's OpenGL backend (label `vulkanmod-gl`).
Options: `-PbenchJfr` (JFR profile to `results/<label>.jfr`), `-PbenchWindow=WxH` (default 1920x1080),
`-PbenchJavaVendor=microsoft -Porg.gradle.java.installations.paths=<jdk>` (run on another JDK), and
`-Pbench.<name>=value` → `-Dvulkanmod.bench.<name>` (`renderDistance`, `preset`, `panSeconds`, `initialSeconds`,
`staticSeconds`, `particleSeconds`, `server`, `particleCommand`, `extraShotYaws`, `staticYaw`, `sectionProfile`,
`chestEvents`, `zoomShot`), and `-Pvm.<name>=value` → `-Dvulkanmod.<name>` for mod switches (`retainedChests=false`,
`instancedModels=false`). `zoomShot` adds a 30° FOV screenshot at yaw 270 (`zoom270`), where the enchanting table
books are large enough to compare.
`chestEvents` opens the lid of a chest in the crosshair and adds breaking progress to another one (client-side)
before the static phase, with `chestopen`/`chestclosed` screenshots; compare runs with `-Pvm.retainedChests=false`
to check the retained geometry's dynamic paths.
`-Pbench.staticYaw=270` runs static and particles facing the chest cube, for profiling the slow pan direction.
`-Pbench.label` does **not** change the label (the task sets it), so move such runs out of `results/` (e.g. to
`results/heavy270/`) before summarizing. `python scripts/frames_by_yaw.py <csv> [bin°]` buckets the pan by yaw
and lists frames much slower than their neighbours.

**Workflow per change:** archive the previous `vulkanmod-*` results and frame CSVs
(`mkdir -p results/vulkanmod-vN/frames && mv vulkanmod-2026* vulkanmod-vN/ && mv frames/vulkanmod-* vulkanmod-vN/frames/`),
run 3×, summarize, diff the static and yaw 270 screenshots, then commit with the numbers. Archives so far go up to
`vulkanmod-v18`; the reference screenshots for parity are the v4 ones from `20261026-201821`. Runs with
`staticYaw=270`, `sectionProfile` or temporary instrumentation go to `results/heavy270/` or `results/sectprof/`.

### 6.4 Gotchas
- **Don't compile or run anything CPU-heavy while a benchmark matrix is running.** Each bench task also rebuilds
  the jars, so editing `src/main` mid-matrix can mix builds or break later runs. Do read-only work in the
  meantime.
- Vanilla's numbers are **bimodal run to run** (static ~21.5 or ~25.5 fps); Sodium and VulkanMod are stable.
  Use medians of ≥3 runs and compare against the 7–8 run vanilla baseline.
- Bench tasks set `outputs.upToDateWhen { false }`. Without it Gradle skipped re-runs as UP-TO-DATE.
- The JFR render thread is named **`main`** (the game log calls it "Render thread").
- The scene is **CPU-bound**: 296 armor stands, a wall of chests, slime/honey blocks, signs, and a building full
  of chests on the left (~14k block entities extracted per frame by vanilla). The machine has an RTX 4060 Laptop
  GPU, Intel UHD iGPU (hybrid), a 3440×1440 primary display and a 2560×1600 @150% laptop panel.

### 6.5 Visual parity check
```bash
java scripts/ImageDiff.java <vanilla.png> <vulkanmod.png> 400 12 diffmap.png
```
Screenshots taken in different runs differ in lighting rings (time of day), clouds and hand-bob. Compare the
count to a **vanilla-vs-vanilla** pair: about 0.10% of pixels at threshold 12. Use threshold 40 and look at the
map to spot missing geometry. Every change so far is within the noise. Since the second session the reference is
the v4 VulkanMod screenshot pair (`*-20261026-201821.png`, itself checked against vanilla): v4-vs-v4 is ~0.03%
at rows ≥ 400 / threshold 12, and the full-frame threshold-40 map shows only clouds and the animated enchanting
table books (red specks in the chest cube).

### 6.6 Profiling
```bash
jfr print --json --stack-depth 96 --events jdk.ExecutionSample build/run/bench/profiles/X.jfr > exec.json
python scripts/jfr_tree.py exec.json main          # inclusive % per method on the render thread
python scripts/jfr_tree.py exec.json main --from 20:47:13 --to 20:47:18   # one phase (times from the game log)
jfr view --width 200 allocation-by-site X.jfr       # allocation hot spots
jfr view gc-pauses X.jfr
```
Saved profiles: `build/run/bench/profiles/{vulkanmod-v2..v8,heavy270*,sodium}.jfr`. The vanilla
profile was archived under `results/old-nostatic/`. The profile setting samples at only ~80 Hz, so use a whole
phase (≥ 400 samples) before trusting a percentage.

**JFR attributes time inside inlined code to the nearest debug scope, which is often wrong.** Seen so far:
16% "in" `ByteBufferBuilder.ensureCapacity` that was really `Cube.compile`'s vertex writes, `Font.width`
samples under a cache *hit* path that instrumented counters proved never ran, 5.8% in
`RenderTypeFeatureRenderer$Group.getVertexBuilder` whose slow path runs ~5 times per frame, and 20% for the book
model space write that nanoTime put at ~12%. `jdk.ExecutionSample` also never sees native time. Inclusive totals
of non-inlined callers are better, but rank work with measurements:

- **Section profiler:** `-Pbench.sectionProfile=true` records vanilla's debug profiler (`debugClientMetricsStart`)
  over the static phase into `build/run/bench/vulkanmod/debug/profiling/*.zip`; `client/profiling.txt` has
  nanoTime-based shares per section (`prepare` → `Entity Model`/`Text`, `uploadSharedVertexBuffer`,
  `submitFeatures`, `extract` → `blockEntities`, `tick`). The profiler slows frames, and per block entity pushes
  inflate `tick` → `blockEntities` heavily, so read it for proportions only.
- **Temporary nanoTime counters** around a call (in a separate plain class, see the design rules), printing
  averages every N calls, for exact costs. Examples from this session: book write 640 ns (130 transform / 185
  culling / 370 writing), chest cached write 123 ns, `tryExtractRenderState` 137 ns per chest, upload 3.3 ms for
  34 MB, `tickBlockEntities` 1.6 ms per tick. Remove them before benchmarking for a commit.
- **Cross-checks against vanilla** for geometry changes: render the same submit through the vanilla path into a
  recording `VertexConsumer` and compare quad sets (used for the model space cache).

---

## 7. Results so far

Median of 3 runs at 1920×1080, Fancy preset, render distance 16. VulkanMod runs on vanilla's Vulkan backend;
Sodium runs on its default (OpenGL).

| Phase (avg / p95 low / p99 low) | vanilla GL | vanilla Vulkan | Sodium | VulkanMod v4 | VulkanMod HEAD |
|---|---|---|---|---|---|
| initial | 19.3 / 13.9 / 7.8 | 22.9 / 15.1 / 9.1 | 49.1 / 30.1 / 18.1 | 93.3 / 59.0 / 30.0 | **262.1 / 140.7 / 48.2** |
| pan | 50.1 / 18.1 / 16.0 | 61.2 / 20.8 / 17.8 | 162.9 / 38.2 / 29.3 | 293.2 / 71.3 / 30.9 | **515.7 / 162.1 / 117.2** |
| static | 21.1 / 17.3 / 15.9 | 25.3 / 21.1 / 20.1 | 63.4 / 52.5 / 41.8 | 121.4 / 93.7 / 77.7 | **415.2 / 251.7 / 192.2** |
| particles | 20.6 / 18.4 / 15.7 | 24.3 / 20.9 / 18.0 | 60.6 / 51.0 / 42.7 | 115.5 / 86.4 / 72.2 | **375.7 / 228.0 / 157.7** |
| alloc at static view | ~16.5 MB/frame | ~16.7 MB/frame | ~11.3 MB/frame | ~4.6 MB/frame | **~1.3 MB/frame** |
| block entities extracted / frame (static) | ~14 075 | ~14 075 | ~8 417 | ~3 862 | **~507** (+ ~3.1k retained chests, ~250 instanced books) |

History of the static view: v1 (cube + particle writers + primitive sort) 30.6 → v2 (+glyph writer, radix sort,
particle cap) 34.3 → v3 (overwrites, pooled sort scratch) 35.1 → v4 (block entity frustum culling) 121.4 →
section classification ~120 (noise; −6.8% hash lookups in JFR) → direct sorted index writes 126.7 → sign text
cache 128.3 → CPU back-face skip 139.7 → rest pose geometry cache 161.0 → per-line sign cache 163.3 → 4³ cells
167.7 → packed section positions 170.3 → (third session) closed books in model space 175.3 → parallel upload
copy 188.2 → sign text back-side skip 194.8 → faster model space writes 199.7 → extraction overwrite 214.1 →
idle tick skip 217.1 → retained chests 343.4 → instanced books 353.7 → instanced armor stands 415.2.
Rejected (no gain, reverted): an 11-bit, single-histogram radix sort (the sort's JFR share was misattributed); a
per-line cache of sign text glyph vertices (verified bit-identical, but static 353.7 → 356.5 is noise: the text
cost is writing ~3.4 MB of glyph vertices per frame, which the cache still does). Results archives: `results/vulkanmod-v5 … v21` (v21 = HEAD).
Facing the chest cube (`staticYaw=270`, 1 run each): ~30 (v4) → 47.5 → ~50 → 56.7 (books) → 61.4 (copy) →
68.5 (sign sides) → 78.5 (book writes) → 103 (retained chests) → 157.5 (instanced books). Initial p99 dropped (47.7 → 41.5): the batches
are built in the first frames after the teleport (budget 2048 chests per frame). Peak particles: vanilla ~16 349, VulkanMod ~16 250.

### The pan's p99
`frames_by_yaw.py` on the pan series: yaw 100–200 has no block entities in view and runs at ~1 ms; yaw 220–320
faces the chest cube (7–13k block entities extracted) at ~12 ms now (v4: ~33 ms, third session start ~20 ms).
GC-correlated spikes exist but stay under ~15 ms. So the p99 low is the cube's frame time, and only per block
entity work moves it. Counted with instrumentation: at the static view ~3.1k chests, ~250 enchanting tables, ~507
signs, 188 armor stands; facing the cube (yaw 270) ~5.3k chests and ~5.3k enchanting tables (closed books)
per frame, and ~80k sign glyph quads before the back-side skip (~49k after). Signs in 26.3 submit only text;
the sign board is not a block entity model. The client ticks ~8.3k chests, ~5.5k enchanting tables and ~2.3k
signs every tick regardless of view.

### Where the render thread goes now (measured, third session)
Static view (~4.6 ms/frame at 217 fps), from the section profiler and nanoTime counters before the last few
changes: block entity extraction ~1.0 ms (`tryExtractRenderState` 0.51 ms for 3.9k visible, chests 137 ns each;
the loop over ~10k culled ones ~0.45 ms before the overwrite), model prepare ~1.2 ms (chests 3.1k × 212 ns
≈ 0.65 ms, armor stands 188 × 1.7 µs ≈ 0.33 ms, books 250 × 760 ns before the faster writes), text prepare
~0.5 ms, `submitFeatures` ~0.55 ms, upload ~0.4 ms, entity extraction ~0.25 ms, block entity ticks 1.0 ms per tick
(every ~10th frame).
Facing the chest cube (~12 ms/frame): the world upload is 34 MB per frame before the sign-side skip (books
16.4 MB, sign text 8.9 MB sorted + 1.9 MB indices, chests 6.9 MB), copied at ~17 GB/s with 2 threads; the
books' write was 640 ns each before the faster model space path (~5.3k per frame).

---

## 8. What's next (in priority order)

1. **Per block entity cost** (the pan p99 and the static view, section 7). Ideas, roughly by expected gain:
   - **Sign text** is now the largest item at the static view: ~550 µs per frame for 2028 line submits (nanoTime
     around `TextFeatureRenderer.buildGroup`), ~450 µs of it in `PreparedText.visit`, i.e. writing ~3.4 MB of
     glyph vertices (memory bound; caching the vertices didn't help, see section 7). It is translucent and sorted
     with the other translucent submits, so it can't simply be retained; fewer bytes would need a custom text
     pipeline (e.g. instancing glyph quads) drawn at the same point of the translucent order.
   - **Armor stand extraction** (~1.3 µs each, `LivingEntityRenderer.extractRenderState`) and their ticking.
   - **Enchanting tables** still pay extraction + a capture submit per frame (~5.3k facing the cube) and ~5.5k
     ticks per tick.
   - **Other retainable/instanceable block entities**: the same batching works for any block entity whose renderer submits one
     cacheable model in a cutout/solid pipeline at rest (e.g. shulker boxes, beds, banners without wind?, decorated
     pots, skulls). Sign text is static too but translucent and sorted per frame, so it can't be kept unsorted
     without risking blend-order differences.
   - **Retained chests first-frame cost**: building ~14k chests after a teleport costs ~1 µs each; the budget
     spreads it over ~7 frames. Lower it if the initial p99 matters.
   - **`tryExtractRenderState` per visible block entity**: renderer lookup (`ImmutableMap`), `isValid` (set
     lookup), `shouldRender`, render state allocation, `LightCoordsUtil.getLightCoords`, chest `combine`.
   - **Writing vertices straight into mapped memory** would remove the remaining upload copy (1.9 ms for 34 MB
     facing the cube), but sorting reads vertices back (slow from write-combined memory), sizes are only known
     after building, and the staging memory must survive until the GPU copy.
   - **Armor stands** (188 × 1.7 µs prepare + extraction): their state is a mutable render state; a model space
     entry keyed by the pose fields would work.
   - **Enchanting table ticks** (~5.5k per tick, most of the remaining 1.0 ms): they animate every tick.
   - Occlusion culling of block entities is **not** exact here: chests are 14/16 wide and the chests behind show
     through the 2/16 gaps, so culling them changes pixels.
2. **Allocation** (~2.3 MB/frame at static): `Float` boxing of chest openness per submit, submit records,
   `PoseStack.Pose`, glyph instances of non-sign text, `CompactVectorArray` per sort.
3. **Config and settings screen port** (`dev:src/main/java/net/vulkanmod/config/**`). Needed so the particle cap
   and block-entity culling become toggles. Port notes from the audit:
   - `GuiGraphics` → `GuiGraphicsExtractor` (`drawString`→`text`, `drawCenteredString`→`centeredText`,
     `submitGuiElement`→`addGuiElement`); `Screen.render` → `extractRenderState(GuiGraphicsExtractor,int,int,float)`.
   - `Minecraft.setScreen` → `minecraft.gui.setScreen`; `levelRenderer.allChanged()` → `minecraft.levelExtractor.allChanged()`.
   - Key codes are SDL scancodes (`InputConstants.KEY_*`), there is no GLFW (`glfwGetKeyName` would crash), and
     text input uses `CharacterEvent`.
   - Hook the video settings button with `@ModifyArg` on `OptionsScreen.init` → `openScreenButton` (ordinal of
     the video button). The old lambda target `method_19828` no longer exists.
   - Vanilla added a **Graphics API option** (`Options.preferredGraphicsBackend()`) and `exclusiveFullscreen()`;
     surface both in the mod's page. Device-specific options from the old config (present modes, device
     selection, frame queue) are obsolete under strategy B.
4. **Other CPU hot spots.**
   - Armor stand extraction (~5% at static, spread thin across `LivingEntityRenderer.extractRenderState`).
   - Entity culling: Sodium and vanilla render the same ~188 entities.
   - Armor stands use `entity_cutout` (no culling), so the CPU back-face skip doesn't apply to them.
5. **Terrain renderer** (old `render/chunk/**`), the large piece. Only do it if it beats vanilla 26.3, which
   already has uber buffers + MDI. VulkanMod's remaining differentiators are the **compressed vertex format**
   (`CustomVertexFormat`: int16 positions, uint16 UVs, packed colour, about half of vanilla's 28 B/vertex) and
   **per-direction face culling** (`QuadFacing`: draw only faces pointing toward the camera via separate
   multi-draw ranges), plus its light pipelines and task scheduler.
   - Write it against the renderpearl API: `GpuBuffer`, a custom `RenderPipeline` with the compressed format,
     `RenderPass.multiDrawIndexed`/`drawIndexedIndirect`, uploads through `CommandEncoder.writeToBuffer` /
     transient memory. It then works on both backends.
   - Hooks (from the audit):
     - `LevelExtractor`: `setLevel`/`allChanged` RETURN, cancel `applyFrustum`, `setSectionDirty(IIIZ)`,
       `countRenderedSections`/`sectionStatistics`.
     - `LevelRenderer`: overwrite `invalidateCompiledGeometry`, both `prepareChunkRenders*` (return null),
       `compileSections`, `isSectionCompiledAndVisible(BlockPos,long)`; make `sectionOcclusionGraph.update` a no-op.
     - `@WrapOperation` (not `@Redirect`; Fabric rendering-v1 already wraps these) on
       `ChunkSectionsToRender.renderGroup` in `executeSolid` / `executeClassicTransparency`, drawing into the
       provided vanilla `RenderPass`.
   - Call `removeTransientBlocksInSection` on upload, otherwise client-predicted blocks linger.
   - Reversed-Z in terrain shaders: depth compare `GEQUAL`.
   - Disable OIT via `GameRenderer.useImprovedTransparency()` → false until an OIT path exists.
   - Mesher changes: per-quad layer from `BakedQuad.materialInfo()`, `CardinalLighting.byFace(shadeOverride ?: face)`,
     `BlockTintSource` lists, `LightCoordsUtil` emission, `VertexFormat.builder(stepRate).addAttribute(name, GpuFormat)`.
6. **Fabric Renderer API 17.** Needed once the mod has its own mesher again. Re-port `EncodingFormat`/`QuadViewImpl`
   /`MutableQuadViewImpl` from Indigo 26.3, implement the 3-method `Renderer` plus an `AltModelBlockRenderer`,
   and register feature renderers via `FeatureRendererRegistry`. Do **not** re-add the old `SubmitNode*` /
   `ItemStackRenderState*` mixins; Fabric now does that itself.
7. **Smaller ports.** Clouds (mixin vanilla `CloudRenderer` `prepare`/`render`; there is no separate clouds pass
   any more), the debug overlay and profiler HUD (`Gui` → `Hud`, `extractRenderState`), and
   `DebugEntryMemory` (`bytesToMebibytes`).

---

## 9. Important notes

- **Commits:** the user wants a commit after each change is benchmarked, with the numbers in the message. End
  commit messages with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Git user `6a0`. Remote
  `origin` = `6a0/VulkanMod`, `upstream` = `xCollateral/VulkanMod`. Nothing has been pushed.
- **No Sodium code.** It is PolyForm Shield licensed.
- **Safety of the culling:** `BlockEntityCulling` only handles `minecraft:` block entity types. If more types
  render outside block ±1 (+2 up), widen the box rather than special-casing.
- **Thread-safety assumptions:** `ModelPartCubeM`, `BakedSheetGlyphM` and `QuadParticleRenderStateM` keep
  per-instance scratch, which matches vanilla (models and text are built on the render thread).
  `DistanceSorting` uses a `ThreadLocal` because sorting can run on worker threads. `SignTextCache`,
  `ModelCulling`, `ModelGeometryCache`, `ParallelUploadCopy` and `RetainedChests` are static, render-thread-only
  state (`ParallelUploadCopy`'s worker only copies chunks of the current job; `RetainedChests.markDirty`/`clear`
  may be called from other threads and only queue work).
- **Assumptions of the model caches/culling:** world features are camera-relative (eye at the origin) and
  drawn with a perspective projection; `Float`-state models are pure functions of that state in `setupAnim`;
  atlas sprites are replaced (not mutated) on resource reload, so identity keys stay valid.
- **Decompiled sources** from the first session are still at
  `%TEMP%/claude/D--dev-VulkanMod/28854e05-c88c-445a-b4b2-c27fb65a3ca3/scratchpad/src-26.3` (client and
  `com/mojang` only), and the third session decompiled `net/minecraft/{world,core,util}` to
  `%TEMP%/claude/D--dev-VulkanMod/269eb9c8-84bf-4573-8d0b-c12a3702d95e/scratchpad/src-world` (Vineflower is at
  `.../28854e05-.../scratchpad/tools/vineflower.jar`; ~20 s per package set).
- **26.4 snapshots** already change renderpearl further (Vulkan is the default there). Expect API churn when moving
  past 26.3.
- **Memory files** for future sessions: `~/.claude/projects/D--dev-VulkanMod/memory/` (`port-26-3-strategy.md`,
  `benchmark-protocol.md`).
