# VulkanMod → Minecraft 26.3 port: continuation notes

This is a hand-off document for a new session. It covers what was decided, what has been built, how to test it,
what the numbers are, and what to do next. Read it all before changing code.

Last updated: 2026-10-26, at commit `ce23d4c` plus the commit that adds this file.

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
- **Results so far** (median of 3 runs, vanilla Vulkan backend): static view **25 → 121 fps**, pan **61 → 293 fps**.
  Sodium 0.9.3-beta.1 on the same benchmark: 63 / 163.
- **Next:** pan p99 hitches, porting the config/settings screen, then the terrain renderer (section 8).

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
| `mixin/render/blockentity/LevelExtractorM.java` + `FrustumAccessor.java` + `render/blockentity/BlockEntityCulling.java` | Per-block-entity frustum culling inside `extractVisibleBlockEntities`. Conservative box: block ±1, +2 extra on top. **Vanilla block entity types only.** This is the biggest win |

Design rules used so far:
- Hot paths use `@Overwrite` with an inlined vanilla fallback, because a cancellable `@Inject` allocates a
  `CallbackInfo` per call, which showed up in allocation profiles.
- Fast paths only engage for the exact vertex format they write (`ExtendedVertexBuilder.of(consumer, format)`).
- Output must match vanilla visually (screenshot diff, section 6.5).

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
  particle spot) → particles (10 s)**. It writes JSON and TXT to `build/run/bench/results/<label>-<time>.*` and
  takes a screenshot of the static view to `build/run/bench/<label>/screenshots/`.
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
Options: `-PbenchJfr` (JFR profile to `results/<label>.jfr`), `-PbenchWindow=WxH` (default 1920x1080),
`-PbenchJavaVendor=microsoft -Porg.gradle.java.installations.paths=<jdk>` (run on another JDK), and
`-Pbench.<name>=value` → `-Dvulkanmod.bench.<name>` (`renderDistance`, `preset`, `panSeconds`, `initialSeconds`,
`staticSeconds`, `particleSeconds`, `server`, `particleCommand`).

**Workflow per change:** archive the previous `vulkanmod-*` results (`mkdir results/vulkanmod-vN && mv vulkanmod-2026* …`),
run 3×, summarize, check the screenshot, then commit with the numbers.

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
map to spot missing geometry. Every change so far is within the noise.

### 6.6 Profiling
```bash
jfr print --json --stack-depth 96 --events jdk.ExecutionSample build/run/bench/profiles/X.jfr > exec.json
python scripts/jfr_tree.py exec.json main          # inclusive % per method on the render thread
jfr view --width 200 allocation-by-site X.jfr       # allocation hot spots
jfr view gc-pauses X.jfr
```
Saved profiles: `build/run/bench/profiles/{vulkanmod-v2,vulkanmod-v4,sodium,vulkanmod}.jfr`. The vanilla
profile was archived under `results/old-nostatic/`.

---

## 7. Results so far

Median of 3 runs at 1920×1080, Fancy preset, render distance 16. VulkanMod runs on vanilla's Vulkan backend;
Sodium runs on its default (OpenGL).

| Phase (avg / p95 low / p99 low) | vanilla GL | vanilla Vulkan | Sodium | VulkanMod v4 (HEAD) |
|---|---|---|---|---|
| initial | 19.3 / 13.9 / 7.8 | 22.9 / 15.1 / 9.1 | 49.1 / 30.1 / 18.1 | **93.3 / 59.0 / 30.0** |
| pan | 50.1 / 18.1 / 16.0 | 61.2 / 20.8 / 17.8 | 162.9 / 38.2 / 29.3 | **293.2 / 71.3 / 30.9** |
| static | 21.1 / 17.3 / 15.9 | 25.3 / 21.1 / 20.1 | 63.4 / 52.5 / 41.8 | **121.4 / 93.7 / 77.7** |
| particles | 20.6 / 18.4 / 15.7 | 24.3 / 20.9 / 18.0 | 60.6 / 51.0 / 42.7 | **115.5 / 86.4 / 72.2** |
| alloc at static view | ~16.5 MB/frame | ~16.7 MB/frame | ~11.3 MB/frame | **~4.6 MB/frame** |
| block entities / frame (static) | ~14 075 | ~14 075 | ~8 417 | **~3 862** |

History of the static view: v1 (cube + particle writers + primitive sort) 30.6 → v2 (+glyph writer, radix sort,
particle cap) 34.3 → v3 (overwrites, pooled sort scratch) 35.1 → v4 (block entity frustum culling) 121.4.
Peak particles: vanilla ~16 349, VulkanMod ~16 250.

### Where the render thread goes now (v4 JFR, inclusive)
- `ModelFeatureRenderer`: entity and block-entity models, 24.7%. `Cube.compile` is 19.4% and already uses the
  fast path.
- `LevelExtractor.extract` 18%: `extractVisibleBlockEntities` 12.2%, of which the per-block-entity frustum test
  is 5.8% and `Long2ObjectOpenHashMap.get` 6.1%. The latter is vanilla's `level.destructionProgress().get(pos)`,
  looked up **for every block entity before our cull**.
- `StagedVertexBuffer.upload` 15% (`writeSortedIndexBuffer` 6.1%).
- Text 7.4% (`Font.prepareText`, `StringDecomposer`).
- `prepareChunkRendersIndirect` 4% (vanilla terrain).
- GC: 128 pauses in ~45 s, median 2.8 ms, p99 15.6 ms. Max frame in the pan is about 52 ms.

---

## 8. What's next (in priority order)

1. **Pan p99 hitches.** The pan p99 low is ~31 fps against a 293 fps average. Use the JFR timeline
   (`jdk.GarbageCollection`, chunk uploads, class loading: `KnotClassLoader` showed 4%) to correlate frame spikes.
   Candidates: G1 pauses (try reducing allocation further, or test `-XX:+UseZGC` only as a diagnostic), section
   compile/upload bursts as new sections come into view, and first-time class loading or mixin application
   during the pan (warm-up). Consider recording the per-frame time series in the JSON to see where spikes land.
2. **Cheaper block-entity culling.** Move the cull before the destruction-progress lookup (e.g. `@Redirect` the
   `destructionProgress().get` or restructure via an overwrite of the section loop). Also skip whole sections
   whose box is entirely outside the frustum (vanilla's section frustum is coarse) and look at occlusion culling
   of block entities. Sodium's count shows vanilla's occlusion graph is looser than it could be.
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
   - Text: `BakedSheetGlyph.createGlyph` was the top allocator, with per-glyph instance records every frame.
     Consider caching prepared sign text per block entity.
   - `PoseStack.Pose` allocation.
   - `StagedVertexBuffer.uploadDrawsToBuffers` copies.
   - `CompactVectorArray` allocated per sort.
   - Entity culling: Sodium and vanilla render the same ~188 entities.
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
  `DistanceSorting` uses a `ThreadLocal` because sorting can run on worker threads.
- **26.4 snapshots** already change renderpearl further (Vulkan is the default there). Expect API churn when moving
  past 26.3.
- **Memory files** for future sessions: `~/.claude/projects/D--dev-VulkanMod/memory/` (`port-26-3-strategy.md`,
  `benchmark-protocol.md`).
