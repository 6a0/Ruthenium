# VulkanMod → Minecraft 26.3 port: continuation notes

This is a hand-off document for a new session. It covers what was decided, what has been built, how to test it,
what the numbers are, and what to do next. Read it all before changing code.

Last updated: 2026-10-27 (sixth session), with the GPU probe findings and the faster section compiles (biome blend,
light, block state and tint caches). Seventh session: exact software occlusion culling of terrain sections on the CPU
was built, measured and rejected (section 7, "Rejected: CPU occlusion culling"); the retained chests were generalised
to shulker boxes, decorated pots and skulls (`RetainedBlockEntities`), with a block entity scene to measure them
(section 6.8). Eighth session (2026-10-27): the settings switches were A/B tested and cost nothing; closed enchanting
table books skip extraction and their light is cached; **sign text is retained on the GPU** (static view 436 -> 908
fps). See section 7, "Eighth session". Ninth session: armor stands are extracted into reused render states
(static 964 -> 1032 fps interleaved; section 7, "Ninth session"). Tenth session: an entity benchmark scene (section
6.9; the server scene has only armor stands), measurements that ruled out cheaper entity culling and generic render
state reuse, **cached block model quads of entities** (item frames etc.; entity scene static 121.8 -> 127.5 fps), a
hashed draw lookup (126.8 -> 130.9) and baked quads written a quad at a time (132.3 -> 135.6).
Eleventh session: the per switch cost of feature renderer groups was measured (~115 us/frame in the entity scene)
and a draw reuse cache rejected (no gain; section 7, "Eleventh session"). **Posed model parts are GPU instances** (`InstancedParts`; entity
scene static 137.7 -> 168.5 fps), with positions bit-identical to vanilla's.
Twelfth session (section 7, "Twelfth session"): **measurement tools that don't misattribute** (a light section
profiler, a profiler-free tick probe, GPU frame and main pass timestamps, frame times split by ticks), which showed
that the server scene's static view is now **close to GPU bound** and that its **p99 low is the frames that run a
client tick**. Then: idle block entity tickers parked, validated ticker block states kept, armor stand pushes typed
to minecarts, retained block entity faces grouped by facing, and the terrain draw list packed per mesh. Server scene
in the same tree, start -> end: static 995.6 -> 1048.9 fps (p99 low 328.0 -> 406.5), pan 951.5 -> 995.1 (p99
294.5 -> 361.5).

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
- **Results so far** (median of 3 runs, vanilla Vulkan backend): static view **25 → 1032 fps**, pan **61 → 998 fps**
  (pan p99 low 18 → 294). Sodium 0.9.3-beta.1 on the same benchmark: 63 / 163. On vanilla's OpenGL backend
  (`benchVulkanModGl`) the static view ran at 344 fps (before the eighth session; not re-measured).
- **Block entities at rest are retained on the GPU** (`RetainedBlockEntities`; chests since the third session, at
  the user's choice, shulker boxes, decorated pots and skulls since the seventh, sign text since the eighth): their
  geometry lives in per-section vertex buffers and is drawn with copies of the entity and text pipelines that add
  `ModelOffset`. Switch
  ("Retained Block Entities"): Optimizations page of the settings screen, or `-Dvulkanmod.retainedChests=false`
  (`-Pvm.retainedChests=false` on bench tasks; the field kept its old name).
- **Closed enchanting table books and armor stands are GPU instances** (`InstancedModels`): model-space mesh once
  per group, a pose + light per instance per frame. Since the eighth session closed books skip the extraction
  entirely (pose computed from the block entity, light cached until a section is marked dirty). Switch: settings
  screen or `-Dvulkanmod.instancedModels=false`. Since the ninth session armor stands are extracted into render
  states reused across frames (`ReusedRenderStates`, switch `-Dvulkanmod.reuseEntityStates`).
- **Entity scene** (tenth session, `-Pbench.scene=entities`, section 6.9): ~1.3k entities of 30 types. Vanilla Vulkan
  86 fps at its static view, VulkanMod ~168 (~138 before the eleventh session's instanced parts). The block model
  quads that entities render (item frames, block displays, minecart and enderman blocks) are cached across frames (`BlockModelMeshCache`, switch
  "Cache Entity Block Models"), feature renderer groups find their draw with a hash lookup
  (`RenderTypeFeatureRendererGroupM`, "Fast Draw Lookup"), and `BufferBuilder` writes baked quads (items) a quad at
  a time (`BufferBuilderM.putBakedQuad`, "Fast Item Vertices"). Section 8 lists this scene's remaining hot spots.
- **The settings screen is ported** (fourth session): the video settings button opens VulkanMod's screen with Video,
  Graphics (vanilla options through their `OptionInstance`s) and Optimizations (the mod's switches, `Config`,
  `config/vulkanmod_settings.json`). `-Dvulkanmod.<field>=true|false` overrides any switch for a launch.
- **Terrain (fifth session).** A singleplayer terrain benchmark (`-Pbench.scene=terrain`, section 6.7) and three
  terrain optimisations on vanilla's section renderer: per-direction face groups (`SectionFaceGroups`), the indirect
  draw list without per-draw lookups (`TerrainDrawList`) and 20 byte solid/cutout vertices (`CompressedTerrain`).
  Terrain static view 477 -> 670 fps, pan 658 -> 873 (vanilla Vulkan 474 / 651). Each has a switch on the
  Optimizations page. The old VulkanMod terrain renderer was **not** ported: vanilla's section compiler, uber
  buffers, occlusion graph and multi-draw-indirect are kept.
- **Section compiles (sixth session).** Exact caches in vanilla's section compile (meshes byte-identical, checked by
  `-Pbench.meshCheck=true`): biome tint blending (`BiomeTintBlend`), AO light/shade (`BlockModelLighterCacheM`) and
  block states (`RenderSectionRegionM`). Single-thread compile 681 -> ~400 us per section, terrain rebuild
  3987 -> 6588 sections/s (vanilla 3864). The GPU probe (`-Pbench.gpuProfile=true`) showed that the terrain's GPU
  time is per-vertex work, and ruled out 16 bit indices, translucent compression and draw splitting (section 7).
- **The pan p99 is not hitches.** Frames facing the chest cube (yaw 220–320, 11–13k block entities) took ~6 ms at
  the start of the eighth session (third session start: ~20 ms) and ~1.3 ms after it (static facing the cube 159 ->
  758 fps), while the rest of the pan runs at ~1 ms (section 7).
- **Don't trust JFR's time attribution** (the user warned about it, and it was wrong again in the third session).
  Use the light profiler (`-Pbench.lightProfile=DEPTH`), the tick probe, the GPU probes and temporary
  `System.nanoTime` counters to rank work, and check each number another way before acting on it; section 6.6
  lists the traps found so far (the light profiler itself inflates per entity sections, Windows thread CPU time
  is quantised, a second worktree's run directory changes GPU time).
- **Server scene, static view: ~0.78 ms render thread, ~0.875 ms GPU per frame** (twelfth session). It is close to
  GPU bound, so CPU savings there show in the render thread time and the p99, not much in the average. Its p99 low
  is set by the 2% of frames that run a client tick (~1.4 ms per tick); see section 7, "Twelfth session".
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
130bf16 Store solid and cutout terrain vertices in 20 bytes
b63badc Build the terrain's indirect draw commands without per-draw lookups
7a8f19d Skip terrain faces that face away from the camera
8e829fb Add a singleplayer terrain benchmark
e574d75 Update continuation notes with the settings screen port
afbf27e Port the settings screen to 26.3
3a018a8 Update continuation notes with the instanced armor stands
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
| `Initializer.java` | Client entrypoint: version, logging, loads `Config`. Platform and FRAPI registration are not ported yet |
| `config/Config.java` | The mod's switches (`blockEntityCulling`, `retainedChests`, `instancedModels`, `modelBackFaceCulling`, `particleSpawnLimit`, `skipIdleTicks`, all on by default), Gson to `config/vulkanmod_settings.json` (unknown fields of the old 1.21.11 file are ignored). `-Dvulkanmod.<field>=true|false` overrides a field at load (and is written back if the user applies settings in that session). The features read `Initializer.CONFIG.<field>` at use, so toggles apply immediately; retained chests and instanced models also `clear()` when toggled |
| `config/**` (settings screen) + `mixin/screen/{OptionsScreenM,OptionInstanceAccessor,TooltipAccessor}.java` | Port of the old screen (`VOptionScreen`, `VOptionList`, widgets, search, Apply/Undo, the `vulkanmod:mod_settings_registry` entrypoint API for other mods). `OptionsScreenM` swaps the supplier of the video button (`@ModifyArg` on `openScreenButton` when the message is `VIDEO`); Shift+P in the screen opens vanilla's. Pages: **Video** (window mode = vanilla `fullscreen` + `exclusiveFullscreen`, exclusive fullscreen mode of `window.findBestMonitor()`, framerate, vsync, inactivity limit, GUI scale, brightness, graphics API, attack/autosave indicators), **Graphics** (preset first, distances, chunk updates, filtering/anisotropy, particles, clouds, leaves, improved transparency, chunk fade, AO, biome blend, shadows, entity distance, mipmaps, weather radius, menu blur, vignette), **Optimizations** (the `Config` switches). `VanillaOptions` wraps a vanilla `OptionInstance`: `set` runs vanilla's listener (preset, `allChanged`, sampler reset, ...), and caption, value label (with the leading `options.generic_value` caption stripped) and tooltip come from the instance through accessors. After Apply every option re-reads its value (the preset changes the others); texture reloads (mipmaps/filtering/anisotropy) happen when the screen closes, like vanilla's video screen. Drawing goes through `GuiRenderer` → `GuiGraphicsExtractor`; the cycling arrows are `PolygonRenderState` triangles (a quad of `RenderPipelines.GUI` with the last vertex repeated; `guiRenderState`/`scissorStack` opened by the access widener). Text input uses `charTyped`; shortcut letters compare `KeyEvent.shortcutKey()` (SDL keycodes, lowercase ASCII for letters). Dropped from the old screen: device/frame queue/builder threads/wayland/texture animation options, advanced chunk culling, indirect draw, unique opaque layer, backface culling (terrain options come back with the terrain renderer), the update checker button |
| `render/chunk/SectionFaceGroups.java` + `mixin/render/chunk/{SectionCompilerM,SectionCompilerResultsM}.java` + `CompiledSectionMeshM` (`interfaces/FaceGroupsHolder`) | Solid and cutout layer meshes (sequential quads) are reordered at compile time (stably, after `BufferBuilder.build`, through a wrap of the results' `Map.put`) into 7 groups: one per axis-aligned facing (by the quad's winding normal from its diagonals; all 4 vertices must share the plane coordinate) plus the rest. Each group keeps its lowest `normal . vertex`. When the draw list is built, a layer's draw is replaced by draws of the groups the eye can be in front of (margin 1e-3), merging adjacent ones. Exact: terrain pipelines cull back faces, and a quad is back facing iff the eye is behind its plane. Switch `terrainFaceCulling` (toggling calls `allChanged`). The non-MDI path (`extractSectionDrawGroups`) gets the same through a `List.add` wrap. **Mixin pitfall seen here:** array field initializers in a mixin were not merged before a constructor `@Inject` ran once a second such field existed; the NPE happened in a compile worker, where vanilla's `runTask` catches it and requeues the task silently (symptom: a compile queue that never drains). The arrays are now allocated in code |
| `render/chunk/TerrainDrawList.java` + `mixin/render/chunk/LevelRendererM.java` (`@Overwrite prepareChunkRendersIndirect`) + `interfaces/PackedSectionDraws` | Builds vanilla's multi-draw-indirect commands: the same draw groups (per vertex/index buffer heap; translucent ones consecutive-only and back to front) and commands, without the 2 hash lookups per section and layer in `UberGpuBuffer.getAllocation` and without a record per command. Since the twelfth session a compiled mesh's solid and cutout draws are packed on first use into one int array (drawn flag, index count, base vertex, the face groups' starts, planes with the margin applied and always-drawn mask) and a `GpuBuffer[]` on the mesh (`CompiledSectionMeshM`): their vertex allocation never moves until the mesh is released. Translucent layers (index buffers re-uploaded under the same mesh when resorted) and meshes with custom index buffers in those layers take the lookups every frame. Face group runs come from a mask of drawn groups and a table of runs per mask (`SectionFaceGroups.addVisibleCommands(int[], ...)`). Verified: old and new collect side by side every frame, identical groups/commands/section infos over 22k server and 76k terrain frames (incl. rebuilds); the mask path on 16M random cases. Commands go into reused int arrays and one bulk copy per group into a VulkanMod `MappableRingBuffer` of indirect parameters, rotated each frame. Vertex sizes per layer are read per frame (they change with `CompressedTerrain`) |
| `render/chunk/CompressedTerrain.java` + `CompressedTerrainBuilder.java` + `mixin/render/chunk/{ChunkSectionLayerM,ChunkSectionsToRenderM}.java` + `assets/vulkanmod/shaders/core/terrain_compressed.vsh` + `interfaces/SectionDispatcherReset` | Solid/cutout vertices in 20 bytes: `Position` RG32_UINT = 3x21 bit fixed point (1/65536 block, offset -8, range [-8, 24)), `Color` RGBA8, `UV0` RG16_UINT (/65536), `UV2` as vanilla. `ChunkSectionLayer.pipeline()`/`vertexFormat()` return copies of vanilla's terrain pipelines (built through a `RenderPipeline.Snippet` of the vanilla pipeline's properties, with the vertex shader and binding 0 replaced; also for the wireframe override). The section compiler gets `CompressedTerrainBuilder` (a `BufferBuilder` subclass handling the 11-arg `addVertex`, chained `addVertex(x,y,z)` and `setUv`) through a `NEW` wrap in `getOrBeginLayer`. The format is chosen when vanilla creates the section dispatcher (`invalidateCompiledGeometry` with a null dispatcher), because the uber buffers are sized per vertex. The switch `compressedTerrain` calls `LevelRenderer.vulkanmod$recreateSectionDispatcher` (like `resetLevelRenderData`, but keeping the occlusion graph's loaded chunks and empty sections, which only fill as chunks load) and then `allChanged`. Translucent layers keep vanilla's format (their quad sorting reads float positions). Little-endian hosts only |
| `render/chunk/BiomeTintBlend.java` + `mixin/render/chunk/{ClientLevelTintM,BiomeManagerAccessor}.java` | `ClientLevel.calculateBlockTint` (and, through `RenderSectionRegionM`, every tint a section compile asks for) from per-thread grids: the biomes of the section plus the blend radius around it (shared by all resolvers), each resolver's sample colours and its blended tints per block. Vanilla's per-block `BlockTintCache` (256 chunk columns, so it thrashed at render distance 32; a read lock and a linear search of the column's heights per lookup) is bypassed for compiles. The biomes are `BiomeManager.getBiome`'s fuzzy zoom with each quart corner's offsets (8 LCG steps) and noise biome cached; the distances use the same double arithmetic, and the sums vanilla's integer arithmetic, so results are identical (`-Pbench.tintCheck=true`: 14.2M blends, 0 mismatches). Grids are dropped through a generation counter on chunk load/unload and `clearTintCaches`. Only for vanilla's `BiomeManager` class and blend radius 1..16; otherwise vanilla's path. Switch `biomeBlendCache` |
| `mixin/render/chunk/{BlockModelLighterCacheM,BlockModelLighterAccessor}.java` + `interfaces/SectionLightWindow` + `SectionCompilerM.beginLightWindow` | While a section compiles (from vanilla's `enableCaching` in `SectionCompiler.compile`), the AO light (packed brightness) and shade brightness of the section and its 1 block border (18^3) are kept in per-thread arrays instead of vanilla's 100 entry FIFO maps (`@Overwrite` of `Cache.getLightCoords`/`getShadeBrightness`; positions outside the window and other callers use vanilla's maps). Indigo's `AoCalculator` goes through this cache. Switch `sectionLightCache` |
| `mixin/render/chunk/RenderSectionRegionM.java` | `@Overwrite getBlockState`/`getFluidState`: the region's center section and border (18^3) are read once from the `SectionCopy`s into a per-thread array (stamped per region, checked by region and thread), switch `sectionStateCache`. `@Overwrite getBlockTint`: straight to `BiomeTintBlend` when `biomeBlendCache` is on. Fabric's block getter API only adds methods to this class |
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
| `render/blockentity/RetainedBlockEntities.java` + `ModelSubmitCapture.java` + `mixin/render/blockentity/{ClientLevelM,BlockEntityRenderDispatcherM,BlockEntityDataPacketM,RenderPipelinesM}.java` + `mixin/world/{ShulkerBoxBlockEntityM,DecoratedPotBlockEntityM}.java` + `assets/vulkanmod/shaders/core/retained_entity.vsh` | Block entities at rest kept on the GPU. Candidates (exact vanilla classes and renderers): chests (lid idle incl. the other half of a double chest), shulker boxes (`CLOSED`, progress 0 at partial tick 0 and 1), decorated pots (no wobble, or game time - start > duration) and skulls (`getAnimation` constant, dragon/piglin heads not `POWERED`; heads with an owner profile never, their skin loads asynchronously). Others of these types are "moving": left out and re-checked every frame. Per render section (keyed by `SectionPos`), the real renderer runs into `ModelSubmitCapture` (a `SubmitNodeCollector` that records model submits; pots submit their 7 parts as `Model.Simple`; anything else, e.g. crumbling, rejects the block entity); a block entity is kept only if every model has no outline and a render type that maps to a retained pipeline (`entity_solid`/`_z_offset_forward`, `entity_cutout_cull`, `entity_cutout`/`_z_offset`) with `DEFAULT_TEXTURING`. Models are written with all faces, through `ModelGeometryCache` for `Float` 0 states, else with vanilla's `setupAnim` + `renderToBuffer`, relative to the section origin, into one vertex buffer per render type. `LevelExtractorM` skips members; batches are drawn after `featureFrame.executeSolid` in `LevelRendererM` with `PIPELINES` (vanilla entity pipelines + a vertex shader adding `DynamicTransforms.ModelOffset`; registered via `RenderPipelines.optionalPipelines`, so a compile failure just turns it off), the render type's textures (`prepare()` once per type per frame) and its layering transform applied to the model view (the skulls' z offset; `RenderType.state` and `RenderSetup`'s transforms are widened in the access widener). Transforms are written after `prepareFrame` with fresh vectors (`DynamicGpuDataStorageMapped` dedups against the last record by `equals`), and the sequential index buffer is grown there too (not inside a pass). Rebuilt when the section's mesh changes, when the section or a neighbour hits `LevelExtractor.setSectionDirty(IIIZ)` (block and light changes; a double chest's light can come from the neighbour), on chest, shulker box and pot `triggerEvent`, block entity data packets (pot sherds, skull profiles; they can arrive after the block), `ClientLevel.destroyBlockProgress`, and when a moving block entity comes to rest. Sections partly within the view distance (64) use the vanilla path that frame. Since the twelfth session, buffers of pipelines that cull back faces (chests, shulker boxes and pots: `entity_cutout_cull`; sign text: the polygon offset text pipelines) are sorted into `SectionFaceGroups` at build time and each frame draws only the runs of groups that can face the eye (switch `modelBackFaceCulling`; GPU 0.197 -> 0.155 ms at the server scene's static view, which was vertex bound: the same at 960x540). Budget 2048 block entities rebuilt per frame; batches unused for 1200 frames are freed. Cleared on level change, `allChanged` and resource reload. **Signs** (eighth session; `SignBlockEntity`/`HangingSignBlockEntity` with `StandingSignRenderer`/`HangingSignRenderer`, no glowing or obfuscated text on either side): `ModelSubmitCapture` records text submits; a text is retained only if vanilla would put it in the **solid** phase (`SubmitNodeCollection.canRenderAsSolid`: colour alpha 255, no background) with `POLYGON_OFFSET` and no outline, which is why the draw order can change. Its glyphs are prepared through `SignTextCache.prepareText` (as `TextFeatureRendererM` does) and written with `TextRenderable.render` into a part per render type (`renderType.format()`, POSITION_TEX_LIGHTMAP_COLOR), drawn with `TEXT`/`TEXT_GRAYSCALE`: copies of vanilla's `TEXT_POLYGON_OFFSET`/`TEXT_GRAYSCALE_POLYGON_OFFSET` (same state, built from a `Snippet` like `CompressedTerrain`) with `retained_text.vsh`. Extra rebuild triggers: `SignBlockEntity.setText` on the client (`SignBlockEntityM`; the edit screen calls it while typing), `FontSet.resetTextures` (`FontSetM`, clears everything) and a change of `Minecraft.isTextFilteringEnabled` (checked in `beginFrame`, clears) |
| `render/blockentity/InstancedModels.java` + `assets/vulkanmod/shaders/core/instanced_entity.vsh` | GPU instancing of models that only differ by pose. Two entry points: (1) the extraction (`LevelExtractorM`) calls `tryAdd(dispatcher, state, …)` after `tryExtractRenderState` for `EnchantTableRenderer`: the real renderer submits into `ModelSubmitCapture`, and a single model with a `modelSpaceKey` (closed book) becomes an instance instead of a submitted render state; (2) `ModelFeatureRendererM.prepareModel` calls `tryAdd(submit)` for models in `POSED_MODELS` (`ArmorStandModel`): after `setupAnim`, a snapshot of all parts' transforms/visibility keys the group (exact for pure `ModelPart` trees; max 32 groups per model) and the mesh is `model.root().render` with the identity pose and CPU culling off. An instance is 3 pose rows + light (52 B, `INSTANCE_FORMAT`, step rate 1). Poses must be orthogonal (rotations, or the `scale(-1,-1,1)` mirror of living entities, where vanilla's normal matrix equals the pose). Instanced copies of `entity_solid`, `entity_cutout` (per-face lighting, no cull) and `entity_cutout_cull` (`PIPELINES`). Per group (model, key, render type, UV mapping, overlay, tint): a model-space mesh from `ModelGeometryCache.writeModelSpace` (identity pose, all faces, duplicates skipped) and a `MappableRingBuffer` for the instances (rotated at the next prepare, so its fence covers the frame that used it). Drawn after `RetainedBlockEntities` with `PIPELINE` (vanilla `entity_solid` + an instanced vertex shader). Cleared with the retained chests. **Closed book fast path** (eighth session, `tryAddClosedBook`, called by `LevelExtractorM` before `tryExtractRenderState`): once one closed book went through the capture path (which records its group and renderer), vanilla `EnchantingTableBlockEntity`s with `open == oOpen == 0` and no breaking progress skip the render state and capture: the pose is built with the same JOML calls in the same order as the extraction + `EnchantTableRenderer.submit` (`translate` x3, `rotateY(-yRot)`, `rotateZ(80°)`; bit-identical, checked on 16.7M books), plus `shouldRender`'s distance test. The light and the `isValid` block state check are cached on the block entity (`EnchantingTableBlockEntityM`, `interfaces/CachedBlockEntityLight`) under a generation stamp that every `setSectionDirty` (and `clear`) bumps (`invalidateLight`); 69M cache hits matched fresh lookups |
| `render/entity/InstancedParts.java` + `mixin/render/entity/model/ModelPartM.java` + `mixin/render/feature/PreparedGroupM.java` + `interfaces/InstancedPartHolder.java` + `assets/vulkanmod/shaders/core/instanced_part.vsh` | **Instanced parts of posed models** (eleventh session, switch `instancedParts`, "Instanced Mob Parts"). While the world's features are prepared (`ModelCulling.isWorldPass`), a `FeatureRenderDispatcher$PreparedGroup` that isn't strictly ordered activates it (`PreparedGroupM` on `prepare` HEAD/RETURN), and `ModelFeatureRendererM.prepareModel` turns it on around vanilla's `setupAnim` + `renderToBuffer` for submits without a sheeted decal whose render type uses `ENTITY_SOLID`, `ENTITY_CUTOUT` or `ENTITY_CUTOUT_CULL` (no blending, so always the solid phase). The `ModelPart.compile` overwrite then adds one instance per part instead of writing its cubes: the pose's rows, the normal matrix's rows, light + overlay (RGBA16_SINT) and color (96 B, `INSTANCE_FORMAT`). Meshes (`PartBatch`, a linked list on the `ModelPart`, per render type and UV mapping) are the part's cubes written once with the identity pose, all faces; max 4096, dropped by `clear()` (resource reload, switch) through a generation stamp. At the group's `prepare` RETURN its batches' instances are copied into the frame's buffer as one draw per batch (render type prepared once per frame); the draws run at `PreparedGroup.execute` RETURN, i.e. right after that group's own draws, so they keep their place among submit orders (`order(n)`: villager clothing is the same model one order later and relies on equal depth) and phases. Instanced copies of the three pipelines with `instanced_part.vsh`, uploaded through a `MappableRingBuffer` after `prepareFrame`. **Positions are bit-identical to vanilla's:** the shader computes `m00*x + (m10*y + (m20*z + m30))` with `precise` (no contraction; `GL_ARB_gpu_shader5`), the same operations in the same order as JOML 1.10.9's `transformPosition` (no `Math.fma` unless `joml.useMathFma`). A first version with `dot()` broke the equal-depth ties of horse markings, which are drawn by vanilla (translucent) on the instanced body. Normals are transformed in full precision and normalized (vanilla quantizes them), which changes ≤5 pixels at threshold 1 per frozen close-up |
| `render/entity/ReusedRenderStates.java` + `mixin/render/entity/{LevelExtractorEntityM,EntityRendererAccessor}.java` | `@Overwrite LevelExtractor.extractEntity`: vanilla `ArmorStand`s with `ArmorStandRenderer` are extracted into render states kept from the previous frame (a pool reset at the head of `extractVisibleEntities`) instead of `createRenderState(entity, partialTicks)`: `extractRenderState` + `finalizeRenderState` (invoker) into the reused state. Vanilla's call took ~1.05 us per armor stand, the reused one ~0.27 us: allocating the state and extracting into it in the same compiled method made the extraction ~5x slower (each step alone, or extracting into an existing state, was 0.07-0.25 us; not JIT tiers, deopts, stack banging or a mixin, all checked). States are submitted and cleared in the frame they are extracted in (`LevelRenderer.submitFeatures`). The extraction overwrites every field except `nameTagAttachment`, `distanceToCameraSq`/`nameTag`/`scoreText` (only with a camera) and `ArmedEntityRenderState`'s arm poses (hidden by the armor stand's own `Rotations` fields, only set by humanoid mobs), which are reset to a new state's values. A temporary reflective deep compare of 2.4M reused states against fresh vanilla ones (139 fields each, incl. the item states) found no difference. Exceptions fall back to vanilla's path (which reports them). Cleared on `setLevel`/`allChanged`. Switch: "Reuse Entity Render States", `-Dvulkanmod.reuseEntityStates` |
| `render/entity/BlockModelMeshCache.java` + `mixin/render/entity/BlockStateModelWrapperM.java` | The Fabric Renderer API overwrites `BlockStateModelWrapper.update` to emit the model's quads into a new `MutableMesh` on every call, i.e. per item frame, block display, minecart block and enderman's block per frame (decoding and encoding every baked quad; ~2 us per item frame). A `@WrapOperation` on its `emitQuads` call (priority 1100, after Fabric's overwrite; `require = 0`, so without the overwrite nothing changes) looks the output up by the model's geometry key (`FabricBlockStateModel.createGeometryKey` with a random of its own seeded like `update`'s, `@Local(argsOnly)` seed; by FRAPI's contract the key determines `emitQuads`' output for a cull test that culls nothing, which `update` passes), wrapped in `OpaqueKey` when Fabric's default `emitQuads` forces leaves to the solid layer (`ModelBlockRenderer.forceOpaque` with the cutout leaves option). A miss emits into a private `MutableMesh` and stores `immutableCopy()`; hit or miss, the mesh is replayed with `outputTo(emitter)`. Models without a key (custom models) keep Fabric's path. Render thread only, max 4096 entries, cleared on resource reload and when toggled. Switch `blockModelMeshCache`. A temporary check compared 1.5M hits against a fresh emission word for word: identical. **Mixin pitfall seen here:** `BlockAndTintGetter` is `net.minecraft.client.renderer.block.BlockAndTintGetter` in 26.3; with the wrong package in the target the `require = 0` injector silently did nothing |
| `mixin/render/feature/RenderTypeFeatureRendererGroupM.java` | `RenderTypeFeatureRenderer$Group.getOrAddDraw` runs on every render type switch within a group (~290 times per frame in the entity scene) and searched all of the group's draws with `List.indexOf` and the deep `PreparedRenderType` record `equals` (~0.95 us per call, groups of up to ~200 draws). A `@WrapOperation` on the `indexOf` looks the type up in a per-group `HashMap` from prepared type to its first index, filled by a `@WrapOperation` on the `List.add` calls (only the one on `drawRenderTypes` counts), so it returns what `indexOf` would (~0.1 us). The switch `hashedDrawLookup` is read once per group, so the map always covers the whole list. **Found here:** vanilla's lookup never matches, because `ScissorState` has no `equals` and `prepare()` copies it into a new instance every call, so every render type switch appends a new draw (0 hits in the entity scene). **Switch `mergeDraws` (on by default, "Merge Entity Draws"):** the map is keyed by `render/feature/DrawKey` instead, a copy of the prepared type with the scissor state compared by value (rectangle only when enabled), so geometry joins the first draw with equal state as the lookup intends. Only groups vanilla may reorder use the lookup at all (`canReorder`, i.e. not `strictlyOrdered`: the translucent phases are strictly ordered and never merge), and only types without connected primitives. Draws with different transform slices still don't merge (consecutive identical transform writes share a slice, others don't; glint and z-offset layering types write different transforms). See section 7, "Tenth session", for the visual and performance tests |
| `mixin/render/vertex/BufferBuilderM.java` (`putBakedQuad`) | Overrides `VertexConsumer.putBakedQuad` (dropped, held and framed items via `ItemFeatureRenderer`) for the block and entity formats: one `reserveVertices(4)`, the same `transformPosition`/`transformNormal` calls as vanilla into a scratch vector (allocated lazily: the mixin's field initializer didn't run for the builders in use, which crashed with a null vector), and the same color, UV, overlay and light per vertex. Other formats, big-endian hosts and switch `directBakedQuads` off call the default method. A temporary check (vanilla's write, then the fast writer into scratch memory, compared byte for byte except the normal's padding byte) passed for 59.5M quads. Items with a Fabric mesh go through Indigo's `ExtendedItemFeatureRenderer` instead and are not affected |
| `mixin/world/{LevelM,BoundTickingBlockEntityM,RebindableTickingBlockEntityWrapperM,ChestBlockEntityM,EnderChestBlockEntityM,ChestLidControllerM,SignBlockEntityM,ShulkerBoxBlockEntityM}.java` + `interfaces/IdleTicker.java` | `Level.tickBlockEntities` skips tickers whose tick is a no-op: exact vanilla chest/trapped/ender chest classes with a closed lid that stays closed, closed shulker boxes (progress and old progress 0; client and server), and sign/hanging sign classes with no editing player. Also skips the world border, block state and validity checks in front of them. 1.6 → 1.0 ms per tick with ~14k block entities |
| `mixin/world/{LevelM,RebindableTickingBlockEntityWrapperM,BlockEntityM,BoundTickingBlockEntityM}.java` + `interfaces/{ParkableTicker,ParkedTickerHolder,BlockEntityTicker,TickerParkingLevel}.java` | **Parked idle tickers** (twelfth session, switch `parkIdleTickers`, "Park Idle Block Entities", needs `skipIdleTicks`). On the client, `LevelM` replaces `tickBlockEntities` with vanilla's loop that compacts the list in place and drops idle tickers: the wrapper (`RebindableTickingBlockEntityWrapper`) is flagged parked and its block entity remembers it. Woken (re-added with `Level.addBlockEntityTicker`) by every event that can end the idle state: `triggerEvent` of chests, ender chests and shulker boxes, `SignBlockEntity.setAllowedPlayerEditor`, and any `rebind` of the wrapper (new block state or block entity; a rebind to vanilla's null ticker on removal or chunk unload just drops it). The level keeps a list of parked tickers (compacted as it grows) so switching parking or idle skipping off unparks all. The benchmark's chest events assert that the lid opens to 1.0 and closes to 0.0 (only possible if the woken ticker runs). **Validated ticker state** (switch `cacheTickerStates`, "Fast Block Entity Ticks"): `BoundTickingBlockEntityM` redirects the per tick `LevelChunk.getBlockState` + `BlockEntityType.isValid` and keeps the first valid state for the bound ticker's lifetime on the client: a successful `LevelChunk.setBlockState` at a block entity's position either removes it or binds a new ticker, and chunk data packets create new block entities (2M cached states compared with the chunk's: no difference). The world border check stays per tick |
| `mixin/world/ArmorStandM.java` + `world/EntityTypeTests.java` | `ArmorStand.pushEntities` queries every entity in its box with a rideable-minecart predicate; the redirect asks for `AbstractMinecart` only (same entities in the same order: class buckets keep insertion order, the armor stand and dragon parts aren't minecarts). No switch (no behaviour change) |

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
`render/chunk/**` (the old terrain renderer: section graph, AreaBuffer, DrawBuffers, light pipelines, FRAPI impl,
fluid renderer; its face culling and compressed vertices now exist on top of vanilla's renderer, see above), `render/vertex/**`
(`CustomVertexFormat`, `TerrainBuilder`, `QuadSorter`), `render/sky/CloudRenderer`, `render/profiling` (overlay),
`mixin/debug/**`, `UpdateChecker`, `Platform`. The device, GL compat, window, wayland and
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
`chestEvents`, `zoomShot`, `settingsScreen`), and `-Pvm.<name>=value` → `-Dvulkanmod.<name>`, which overrides that
`Config` switch (`retainedChests=false`, `instancedModels=false`, `blockEntityCulling=false`, ...).
`-Pbench.settingsScreen=true` skips the benchmark: from the title screen it opens the settings screen through the
options screen's video button, screenshots each page (`settings-*`), toggles and applies a switch (checking
`Config` and the written file), hovers an option for its tooltip, searches with Ctrl+L and opens vanilla's screen
with Shift+P (`SettingsScreenCheck`). No server needed, ~20 s. `zoomShot` adds a 30° FOV screenshot at yaw 270 (`zoom270`), where the enchanting table
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
- **A rejected or interrupted Bash command can leave its Gradle build running** (the daemon keeps going). Check
  `jps -l` for a `KnotClient` before starting runs; a running one holds the terrain world's `session.lock`.
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

**Twelfth session tools (use these first):**
- `-Pbench.lightProfile=DEPTH` (`LightProfiler`, installed through `Minecraft.constructProfiler`): vanilla's
  profiler sections timed with `System.nanoTime` down to DEPTH levels, deeper pushes only counted, logged per phase
  as a tree (`ms/frame`, self, calls and deep pushes per frame, `incrementCounter` amounts). Extra sections from
  gametest mixins: `prepareChunkRendersIndirect` → `collect`/`writeCommands`/`writeChunkSectionsInstanced`,
  `submit` → `awaitGpu` (vanilla's wait for the submit two frames back) and `endCommandBuffer`, and a `tickers`
  counter on the block entity tick. Frame level sections (one push per frame) are accurate, with no measurable fps
  cost. **Sections below a per entity or per block entity push are not:** with any profiler active, every
  vanilla push costs a ThreadLocal lookup and a call, so timing the tick at depth 8 gave ~3.0 ms per tick and at
  depth 2 ~1.9 ms, while the tick probe says 1.4 ms. Use depth 2 or the tick probe for ticks.
- **Tick probe** (always on, `TickProbe`): `Minecraft.tick` and, at their call sites, the client level's entity and
  block entity ticks, one nanoTime pair each, logged per phase (`static: ticks 101: 1.42 ms per tick, entities ...`).
  Hooks at a method's RETURN don't run when a mod's HEAD inject cancels it, hence the call sites.
- **Tick split** (always on): each phase logs frame times of frames with and without a client tick; the pan CSV has
  a `ticks` column. At ~1000 fps only 2% of the frames tick, so the tick sets the p99 low.
- `-Pbench.gpuFrame=true`: `GpuFrameProbe` (timestamps at the start of `renderFrame` and before the submit: the
  GPU time of the frame's command buffer) and `GpuPassProbe` (solid terrain, solid features, retained block
  entities, instanced models inside the main pass). Compare with the frame time to see whether a view is GPU bound.
- **No per section CPU time:** Windows charges thread CPU time in scheduler ticks; per section CPU times came out
  larger than the wall time (vanilla's `submit`: 0.58 ms CPU in 0.23 ms). Only whole-phase CPU time is usable.
- **A/B across commits in a second git worktree is not valid for GPU bound views:** its run directory gave ~0.03 ms
  more GPU time per frame with identical commands (in-tree on/off showed no GPU difference). Compare CPU section
  times across worktrees, or check out the commits in turn in the main tree (fps), as the final comparison of the
  twelfth session did. A worktree's first run is also slow (pipeline caches).
- Removing work by elimination (temporary property that skips a part, one frame-level section timed) ranked costs
  reliably where per-call timers would have distorted them (the enchanting table tick, the draw list, the block
  entity extraction).

Older notes:
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

### 6.7 Terrain benchmark (fifth session)
`-Pbench.scene=terrain` on any bench task (`TerrainBenchmark`): a singleplayer world from seed `8279265` (savanna
hills, forest, a river and the ocean in view), render distance 32 (`terrainRenderDistance`), a spectator 16 blocks
above the highest block at the spawn (`terrainHeight`, `terrainSpot=x,z`), pitch 15 (`terrainPitch`), Fancy,
1920x1080. The world is generated once into `build/run/bench/worlds/terrain-<seed>` and copied into the run's saves
for every run. Results are labelled `<label>-terrain`. Phases: static (5 s), pan (10 s, after an untimed pan that
compiles every direction), rebuild (3x F3+A-style `allChanged`, reported as `rebuildSectionsPerSecond`).
Screenshots: static, yaw 90/180/270, and after the rebuilds. Joining is logged but not measured: in singleplayer the
chunks arrive as fast as the integrated server loads them (~60/s, ~70 s for the 3.7k chunks).
`-Pbench.terrainSwitchCheck=true` switches `compressedTerrain` off and on through the option in-world and
screenshots both.
- **Vanilla bug:** after `allChanged` with a still camera, the section occlusion graph often misses sections until
  its next full update (here ~1770 instead of ~2080 visible, every other F3+A, a hillside missing). Turning the
  camera doesn't help; moving 8 blocks would. This made the rebuild time bimodal (0.44/0.64 s). The benchmark
  invalidates the graph (untimed) after each rebuild and reports throughput. Not fixed in the mod.
- The static terrain view is GPU bound: render thread 0.87 ms CPU per 1.49 ms frame. At 960x540 it is only ~25%
  faster, and at render distance 4 a frame takes 0.39 ms, so most of the GPU time is per-vertex terrain work.
- Phases now report the render thread's CPU time per frame (`cpu ms/f` in `bench_summary.py`).
- Sixth session diagnostics (all off by default, on any terrain run):
  - `-Pbench.gpuProfile=true` (`GpuProbe`): GPU timestamps around each terrain layer's draws
    (`ChunkSectionsToRender.DrawIndirect.render`), quads and draws per layer, and how much of the visible geometry
    is in fully underground sections. Logged as `GPU static: ...` / `GPU pan: ...`.
  - `-Pbench.meshCheck=true` (`MeshCompareCheck`): compiles 486 sections around the player with the mesher switches
    on and off (tint caches cleared before each) and fails on any differing vertex byte.
  - `-Pbench.tintCheck=true` (`BiomeTintCheck`): `BiomeTintBlend` vs vanilla's blending for 288 sections, 4
    resolvers, radius 1/2/7 (~2.5 minutes; changing the radius reloads the chunks, so that run's fps are invalid).
  - `-Pbench.compileBench=true` (`CompileBenchmark`): compile time per section on the render thread, median of 7
    rounds of the same 486 sections, with each mesher switch off in turn (interleaved), and the same sections
    compiled by 1..19 threads at once. Far less noisy than the rebuild phase, which depends on vanilla's
    scheduling and on the occlusion graph bug.
  - The rebuild phase logs the background workers' CPU time (`Rebuild worker CPU: ...`).
- The benchmark turns off vanilla's AFK frame rate limit (30 fps after 60 s without input), which capped every
  phase of the first terrain runs.

### 6.8 Block entity benchmark (seventh session)
`-Pbench.scene=blockentities` on any bench task (`BlockEntityBenchmark`): a fresh flat world (Fabric's consistent
test settings, no template), render distance 8 (`blockEntityRenderDistance`), clouds off, a spectator at
(0.5, -46, -12.5) looking at a field built with `/fill`: 128 chests, 6144 shulker boxes (16 colours, facings up /
north / east), 3072 decorated pots (blank and three sherd sets) and 3072 skulls on top of them (7 kinds, 4
rotations, zombie wall heads). The run fails if fewer than 12k block entities arrive. Phases: static (facing the
field) and a 360 degree pan. Results are labelled `<label>-blockentities`. `-Pbench.blockEntityEvents=true` adds
client-side events before measuring: a shulker box opened, a pot wobbled and breaking progress on a skull
(`beevents` screenshot after 30 ticks), then closed/removed again (`bereset`). Compare with
`-Pvm.retainedChests=false`. `-Pbench.blockEntitySigns=true` (eighth session) adds a row of signs ~10 blocks in front
of the camera at z -4/-5 (standing signs on a stone row, wall signs on its north face, hanging signs below it; plain,
coloured, bold, underlined and back side text, one glowing sign at x 4, which stays on the vanilla path); with the
events it also changes one sign's text on the client (`besign`). It hides part of the field, so don't compare its fps
with runs without it. Crop the signs with `scripts/Crop.java` (`java scripts/Crop.java out.png x y w h
scale in1.png in2.png`, around x 740 y 600) to compare glyphs.

### 6.9 Entity benchmark (tenth session)
`-Pbench.scene=entities` on any bench task (`EntityBenchmark`): a fresh flat world, render distance 12
(`entityRenderDistance`; entity view scale 1.5), clouds off, time 6000, difficulty normal, a spectator at
(0.5, -46, -8.5), yaw 0, pitch 30. Rows of entities summoned with NBT (so `finalizeSpawn` and its random equipment
are skipped), near to far: 256 dropped items (no gravity, no pickup, no despawn, 1 block apart so they don't merge),
64 experience orbs (different values), cows/pigs/sheep (colours, some sheared)/chickens (every fourth a baby), villagers
and zombie villagers (all professions and types), zombies (every third in iron armor with a sword), skeletons with
bows, creepers (every eighth charged), spiders, endermen (half carrying grass), pillagers/vindicators/evokers/witches,
iron golems, horses (variants, half saddled), wolves and cats, minecarts/chest minecarts/boats/chest boats, text/item/
block displays, and a stone brick wall with 256 item and glow item frames and 64 paintings. Undead wear unbreakable
helmets so they don't burn; some mobs have visible name tags. 1332 entities, ~1190 extracted at the static view.
Mobs have `NoAI` unless `-Pbench.entityAi=true`. Phases: static and pan; results labelled `<label>-entities`.
Screenshots of two runs differ by ~1% even with the same settings (dropped items spin and bob, chickens flap): compare
on/off pairs against a same-setting pair. Results in `results/entities-dev/`.
`-Pbench.closeShots=N` adds seven close-ups of the mob rows after the benchmark, with `tick freeze`: N screenshots per
position (`close<k>-<n>`), pixel-identical within a run when rendering is deterministic (checked: 0 differing pixels
at threshold 1). `-Pbench.closeToggle=<Config field>` flips that boolean switch for shots 1, 3, ... (by reflection, so
vanilla runs work without it), comparing both settings on the same frozen frame. Use short phases for these runs
(`-Pbench.staticSeconds=2 -Pbench.panSeconds=2`) and move their results out of `results/`. `-Pjvm.X=v` passes `-DX=v`
to the game; Mojang's debug flags work in release builds with `-Pjvm.MC_DEBUG_ENABLED=true` plus
`-Pjvm.MC_DEBUG_<FLAG>=true` (`MC_DEBUG_PRINT_PROPERTIES=true` lists them), e.g. `SHUFFLE_MODELS`, which shuffles the
submit order of the reorderable feature phases every frame.

---

## 7. Results so far

Median of 3 runs at 1920×1080, Fancy preset, render distance 16. VulkanMod runs on vanilla's Vulkan backend;
Sodium runs on its default (OpenGL).

| Phase (avg / p95 low / p99 low) | vanilla GL | vanilla Vulkan | Sodium | VulkanMod v4 | VulkanMod HEAD |
|---|---|---|---|---|---|
| initial | 19.3 / 13.9 / 7.8 | 22.9 / 15.1 / 9.1 | 49.1 / 30.1 / 18.1 | 93.3 / 59.0 / 30.0 | **757.5 / 337.2 / 128.1** |
| pan | 50.1 / 18.1 / 16.0 | 61.2 / 20.8 / 17.8 | 162.9 / 38.2 / 29.3 | 293.2 / 71.3 / 30.9 | **998.0 / 652.4 / 294.3** |
| static | 21.1 / 17.3 / 15.9 | 25.3 / 21.1 / 20.1 | 63.4 / 52.5 / 41.8 | 121.4 / 93.7 / 77.7 | **1032.3 / 758.2 / 343.6** |
| particles | 20.6 / 18.4 / 15.7 | 24.3 / 20.9 / 18.0 | 60.6 / 51.0 / 42.7 | 115.5 / 86.4 / 72.2 | **869.0 / 427.9 / 307.6** |
| alloc at static view | ~16.5 MB/frame | ~16.7 MB/frame | ~11.3 MB/frame | ~4.6 MB/frame | **~0.17 MB/frame** (180 MB/s) |
| block entities extracted / frame (static) | ~14 075 | ~14 075 | ~8 417 | ~3 862 | **0** (~3.1k retained chests, ~507 retained signs, ~250 instanced books) |

History of the static view: v1 (cube + particle writers + primitive sort) 30.6 → v2 (+glyph writer, radix sort,
particle cap) 34.3 → v3 (overwrites, pooled sort scratch) 35.1 → v4 (block entity frustum culling) 121.4 →
section classification ~120 (noise; −6.8% hash lookups in JFR) → direct sorted index writes 126.7 → sign text
cache 128.3 → CPU back-face skip 139.7 → rest pose geometry cache 161.0 → per-line sign cache 163.3 → 4³ cells
167.7 → packed section positions 170.3 → (third session) closed books in model space 175.3 → parallel upload
copy 188.2 → sign text back-side skip 194.8 → faster model space writes 199.7 → extraction overwrite 214.1 →
idle tick skip 217.1 → retained chests 343.4 → instanced books 353.7 → instanced armor stands 415.2 → (eighth
session, see below) closed books without extraction 435.6 → retained sign text 907.9 → cached table light 917.8 → (ninth session) reused armor stand states 1032.3 (964.0
without, interleaved).
Settings screen port (fourth session, config field reads instead of constants on the hot paths): 404.3 static
(386.0 / 404.3 / 415.6 vs HEAD's 406.9 / 415.2 / 423.9), screenshots within noise (0.017% static, 0.058% yaw 270).
Later runs that session were slowed ~30% by other load on the machine (HEAD too: 364.7) and were moved to
`results/noisy-20261027/`. The interleaved A/B was done in the eighth session (below): the switches cost nothing.
Rejected (no gain, reverted): an 11-bit, single-histogram radix sort (the sort's JFR share was misattributed); a
per-line cache of sign text glyph vertices (verified bit-identical, but static 353.7 → 356.5 is noise: the text
cost is writing ~3.4 MB of glyph vertices per frame, which the cache still does). Results archives: `results/vulkanmod-v5 … v28` (v22 = the settings screen port, v23 face groups, v24 draw list,
v25 compressed terrain vertices, v26 closed books without extraction, v27 retained sign text, v28 cached table light, v29 reused armor stand states).
Facing the chest cube (`staticYaw=270`, 1 run each): ~30 (v4) → 47.5 → ~50 → 56.7 (books) → 61.4 (copy) →
68.5 (sign sides) → 78.5 (book writes) → 103 (retained chests) → 157.5 (instanced books) → (eighth session, median
of 3 interleaved where marked) 159.3 → 188.4 (closed books without extraction, interleaved) → 671.4 (retained sign
text, 1 run) → 674.9 → 757.7 (cached table light, interleaved). Initial p99 dropped (47.7 → 41.5) with the retained
chests: the batches are built in the first frames after the teleport (budget 2048 block entities per frame). Peak
particles: vanilla ~16 349, VulkanMod ~16 250.

### Eighth session (2026-10-27)
- **Settings switches A/B** (the to-do from the fourth session): `3a018a8` (constants) vs `afbf27e` (the settings
  port, `Config` field reads), interleaved in a separate worktree, median of 3: static 413.6 vs 416.7, pan 499.4 vs
  496.2 (p99 116.7 vs 119.8), particles 361.9 vs 368.4. No cost. Results in `results/ab-settings/{pre,port}`.
- **Closed books without extraction** (`6d8702e`): see `InstancedModels` in section 4. Facing the cube 159.3 →
  188.4 fps (render thread 6.21 → 5.10 ms). `results/heavy270/books/{off,on}`, `vulkanmod-v26`.
- **Retained sign text** (`431a05b`): see `RetainedBlockEntities` in section 4. The reasoning that makes it exact:
  vanilla 26.3 submits text with an opaque colour and no background to the solid phase
  (`SubmitNodeCollection.submitTextPart` → `canRenderAsSolid`), i.e. it already treats such text as order
  independent, so drawing it with the retained batches after the solid features matches vanilla's own guarantee
  (it is still sorted on upload by its render type, but in the solid phase that only matters for depth ties).
  Sign colours are `ARGB.opaque` (`DyeColor` text colours), so ordinary sign text qualifies; glowing text (camera
  dependent outline) and obfuscated text (random every frame) stay on the vanilla path. The earlier note that sign
  text "can't be kept unsorted" was wrong for this reason. Server scene static 435.6 → 907.9, pan 553.1 → 872.1
  (p99 131.7 → 275.1), initial 273.4 → 610.0; render thread at the static view 2.23 → 0.91 ms. Block entity scene
  with signs (`blockEntitySigns`, `blockEntityEvents`): retained vs vanilla path differ in the same 18 pixels as
  before in static/bereset/besign. `results/blockentities-dev/signs`, `heavy270/signs`, `vulkanmod-v27`.
- **Cached table light** (`8b3031b`): facing the cube 674.9 → 757.7 (render thread 1.39 → 1.14 ms), interleaved.
  `results/heavy270/light/{off,on}`, `vulkanmod-v28`.
- **Where the render thread goes now** (section profiler, `results/sectprof/s8b`; JFR `profiles/vulkanmod-v28.jfr`):
  the static view is at ~0.9 ms/frame. Largest items: entity extraction ~18% of the profile (188 armor stands;
  JFR puts ~20% of the static phase in `ArmorStandRenderer.extractRenderState`, most of it as self time, i.e.
  inlined code such as `HumanoidMobRenderer.extractHumanoidRenderState`, so measure with nanoTime before
  optimising), `TerrainDrawList.prepare`/`collect` (~16–19% in JFR), block entity extraction ~8.5% (the culling loop
  over sections partly beyond the 64 block view distance, whose retained batch isn't used that frame), ticks ~9%
  (block entities 4.9%, of which ~5.5k enchanting tables; armor stands 1.9%), and vanilla's frame graph
  setup/execution and present. Facing the cube, block entity extraction (the ~5.3k closed books) was ~39% before
  the light cache.

### Ninth session (2026-10-27)
- **Reused armor stand render states**: see `ReusedRenderStates` in section 4. nanoTime counters around the real
  extraction (`results/probe-armorstand`): 1050 → 267 ns per armor stand, entity extraction 288 → 91 us per frame.
  How it was found: vanilla's parts timed in isolation summed to ~0.35 us; extracting into an existing state took
  ~0.2 us, allocating ~0.08 us, but any method that allocated the state and extracted into it (vanilla's
  `createRenderState(entity, partialTicks)` or a copy of it in our code) took ~1 us, while the same statements
  inlined into a loop body took ~0.2 us. Ruled out: mixins on `EntityRenderer` (`-Dmixin.debug.export`, none),
  JIT tiers (`PrintCompilation`: all C2), deoptimization storms (JFR `jdk.Deoptimization`), stack banging
  (`StackShadowPages=20` same). The cause wasn't identified further. Benchmark, interleaved with
  `-Pvm.reuseEntityStates=false`, median of 3 each: static 964.0 → 1032.3 (p99 319.8 → 343.6), pan 945.9 → 998.0,
  initial 651.8 → 757.5, particles 811.0 → 869.0; render thread at static 0.90 → 0.78 ms/frame; allocation 334 →
  180 MB/s. Screenshots: on vs off 0.021% (static) / 0.030% (yaw 270) at rows ≥ 400, the full-frame threshold-40
  map shows only clouds; v4 vs on 0.016% / 0.033%. `results/vulkanmod-v29`, `results/ab-reuse/off`.

### Tenth session (2026-10-27)
- **Entity culling measured** (`results/probe-cull`): `isEntityVisible` over the server scene's 297 entities costs
  ~16.5 us per frame (~56 ns each; the "~50 us" in the ninth session's notes was wrong), timed as separate loops over
  the same entities: `shouldRender` ~29 ns, the section check ~45 ns (mostly `Util.getMillis()` per call). The whole
  loop is ~68 us, of which ~50 us are the 189 extractions. A cheaper cull could save ~5-10 us (~1%), below the
  benchmark's noise; not done.
- **Generic render state reuse measured and rejected** (`results/entities-dev/xprobe`): in the entity scene, per
  renderer class, vanilla's `createRenderState(entity, partialTicks)` vs extracting into a kept state
  (`extractRenderState` + `finalizeRenderState`), alternating which runs first: 1647 vs 1627 ms in total, within ~5%
  for every type (zombie 667/659 ns, item frame 1964/1990, cow 463/458). The armor stand's ~4x effect (ninth session)
  doesn't happen for other types, so reuse would save ~20 us per frame there at the risk of stale fields.
- **Entity scene profile** (JFR, `results/entities-dev/vulkanmod-e0.jfr`, static, ~8 ms/frame): model prepare
  (`Cube.compile` of posed mobs) ~27%, items (`ItemFeatureRenderer`, `putBakedQuad` ~10%) ~17%, extraction ~15%
  (item frames ~6% before the cache), upload ~9%, `RenderTypeFeatureRenderer$Group.getOrAddDraw` ~5% (see section 8),
  Indigo's `ExtendedBlockModelFeatureRenderer` ~5%. Per entity extraction costs (ns): item frame 1964, block display
  2272, enderman 1167, minecart 1009, item display 718, zombie 667, skeleton 678, pillager 801, cow 463, villager 434,
  item 334, painting 218, experience orb 175.
- **Cached entity block model quads** (`2b25f26`; see `BlockModelMeshCache` in section 4): entity scene, interleaved
  with `-Pvm.blockModelMeshCache=false`, median of 3: static 121.8 -> 127.5 (render thread 8.00 -> 7.74 ms), pan
  687.4 -> 720.1 (p99 119.2 -> 125.4). `results/entities-dev/meshcache/{off,on}`.
- **Hashed draw lookup** (`93c6359`; `RenderTypeFeatureRendererGroupM` in section 4): a probe on vanilla's `indexOf`
  (`results/probe-drawlookup`) showed ~36k calls/s at the static view, average list size 69 (max 199), ~950 ns per
  call and never a match; the map takes ~100 ns. Interleaved with `-Pvm.hashedDrawLookup=false`, median of 3: static
  126.8 -> 130.9 (render thread 7.58 -> 7.36 ms), pan 696.2 -> 699.2. `results/entities-dev/drawlookup/{off,on}`.
- **Baked quads a quad at a time** (`28e2b5e`; `BufferBuilderM.putBakedQuad` in section 4): interleaved with
  `-Pvm.directBakedQuads=false`, median of 3: static 132.3 -> 135.6 (render thread 7.44 -> 7.14 ms; allocation
  1054 -> 771 MB/s, so vanilla's two vectors per vertex were not scalar replaced), pan 722.6 -> 726.1 (p99
  128.5 -> 135.0). `results/entities-dev/bakedquads/{off,on}`. Screenshots on vs off differ by 1.3-1.7% at threshold
  12, same-setting pairs by 1.0-1.5%.
- **Draw merging tested** (`f2cf0d6`, switch `mergeDraws`, now on by default; see `RenderTypeFeatureRendererGroupM` in
  section 4). Visual: vanilla with `MC_DEBUG_SHUFFLE_MODELS` (random submit order in the reorderable phases every
  frame) gave pixel-identical frozen close-ups at all seven positions, and so did VulkanMod with `mergeDraws` flipped
  between shots of the same frame (merge hits confirmed in those frames). Draws per frame at the static view:
  ~300 -> ~105 (65% of lookups hit; the rest differ in transform slice or type). Performance, six interleaved pairs
  (`results/entities-dev/mergedraws-all/{off,on}`, median): static 137.1 -> 138.1 fps (render thread 7.10 -> 7.00 ms;
  4 of 6 pairs faster, 1 equal), pan 734.9 -> 733.1. Small, because each switch still calls `prepare()` and creates a
  `BufferBuilder` (`StagedVertexBuffer.getVertexBuilder`); merging only saves the execution of ~200 draws, which is
  cheap on this GPU. First committed off by default, then turned on at the user's request (identical visuals; a mod that
  relies on submit order within these phases can turn it off; the gain may be larger where draw calls cost more,
  e.g. integrated GPUs, not measured).

### Eleventh session (2026-10-27)
- **Per switch cost in feature renderer groups measured; draw reuse rejected** (`results/entities-dev/probe-switch`,
  `results/entities-dev/drawtargets/{off,on}`). A temporary `System.nanoTime` probe on `RenderTypeFeatureRenderer$Group`
  at the entity scene's static view (render thread ~7.2 ms/frame) found:
  - ~197 render type switches per frame, not ~290. By type: item_cutout ~100 (dropped items alternate between the block
    and item atlas types), entity_cutout ~44, eyes ~23, entity_translucent ~12.
  - `getOrAddDraw` takes ~75 us per frame in total, ~42 us of it in `prepare()`.
  - The ~197 new `BufferBuilder`s (finishing the previous one into `MeshData`) take ~39 us.
  - `Group.getVertexBuilder` runs ~13.7k times per frame (once per item quad); its fast path is a few field compares.

  The whole per switch cost is therefore ~115 us/frame (~1.6%). Of the switches, 44 are in strictly ordered groups
  (they always append), ~54 are a type's first use in its group (a draw needs a prepared type anyway) and ~99 hit an
  existing draw. All 97 appended draws per frame are of those first two kinds, so slice-keyed merging loses no
  merges here.

  A per group cache from render type to its draw, checked against the model view matrix, projection type and scissor
  (only for constant texture transforms and vanilla's layering transforms), skipped `prepare()` and the lookup on
  those ~99 hits. The probe showed it saving ~10 us/frame. Three interleaved ABBA pairs, off vs on: static
  139.1/138.7/139.5 vs 138.7/137.7/138.5 fps (render thread 7.08/7.05/6.89 vs 7.00/7.11/7.17 ms), pan
  746.8/763.6/747.2 vs 730.0/731.8/735.2. No gain, so it was not committed. The remaining ~40 us of builder switches
  could only go if merged draws had their own staging memory (a rewrite of `StagedVertexBuffer`'s single
  `ByteBufferBuilder`), or by sorting item submits by render type, whose sort would cost about what it saves.

- **Instanced parts of posed models** (`InstancedParts` in section 4, switch `instancedParts`, on by default). Profile
  before (JFR, `results/entities-dev/vulkanmod-e11.jfr`): `Cube.compile` ~66% of `ModelFeatureRenderer.prepareModel`,
  the part tree's pose rotations ~12%. Each posed part now writes one 96 byte instance instead of its vertices.
  Entity scene, three interleaved ABBA pairs (`results/entities-dev/instancedparts/{off,on}`), median off vs on:
  static 137.7 -> 168.5 fps (p99 87.0 -> 96.5, render thread 7.04 -> 5.75 ms), pan 737.5 -> 782.5 (p99
  136.8 -> 156.5). Per frame allocation is unchanged (~5.7 MB). Frozen close-ups with the switch flipped between
  shots of the same frame (`closeToggle=instancedParts`): ≤5 differing pixels at threshold 1 and 0 at threshold 12 on
  vanilla's Vulkan backend; on its OpenGL backend (`benchVulkanModGl`, pipelines compile there too) ≤9 at
  threshold 1, ≤6 at 12. The first version transformed positions with `dot()` and broke the horse markings' depth
  ties (2.5k pixels in close-up 4); matching JOML's operation order with `precise` fixed it. Block entity scene
  (`results/blockentities-dev/instancedparts`, one pair with events and signs): static 1672 vs 1666 fps; the static,
  `bereset` and `besign` shots are pixel-identical, `beevents` differs only in the fading chat message. Server scene
  (`results/server-parts`, two pairs): static 1029/1034 vs 1035/1024, initial and pan within noise; its posed models
  (armor stands, books) are already instanced or retained.

### Twelfth session (2026-10-27)
Measurements first (section 6.6 for the tools), server scene static view:
- **Frame:** ~1.0 ms wall, ~0.78 ms render thread CPU, **~0.90 ms GPU** (0.66 ms at 960x540); the render thread waited
  ~0.2 ms per frame in `awaitGpu`. Main pass GPU: solid terrain 0.42 ms, retained block entities 0.20 (vertex
  bound), instanced models 0.065, the rest (sky, translucent, hand, GUI, blit) ~0.23.
- **Render thread per frame:** `submit` ~0.23 ms (mostly `awaitGpu`), `prepareChunkRendersIndirect` 0.12 (`collect`
  0.106 for 669 sections, ~160 ns each), `present` 0.095, block entity extraction 0.09, entity extraction 0.065,
  `submitFeatures` 0.04, `prepareFeatures` 0.04, `executeFrameGraph` 0.07, GUI 0.05.
- **Ticks:** frames with a tick 3.05 ms vs 0.96 ms without; 100 of ~5000 frames, so the p99 low (~3 ms) is the tick.
  Tick 2.16 ms (depth 2): block entities ~1.0 (~16.3k tickers visited, ~10.7k idle; ~5.6k enchanting tables), entities
  ~0.5 (~300 armor stands), animateTick 0.27, textures 0.16.

Changes, each A/B tested interleaved (results in `results/{park,tickstate,retfaces}`, `blockentities-dev/retfaces-*`,
`terrain-dev/s12`):
- **Parked idle tickers** (`0dec145`): tickers per tick 16.3k -> 5.6k; tick frames ~3.2 -> ~2.8 ms; static p99 low
  319.9 -> 361.7, pan p99 297.6 -> 324.0 (three pairs).
- **Validated ticker state** (`c187201`): tick 1.674 -> 1.398 ms (median of three pairs, tick probe). An elimination
  probe had shown that vanilla's guard in `BoundTickingBlockEntity.tick`, not the book animation, was the cost.
- **Typed armor stand push** (`5f7f4f5`): entity ticks 0.49 -> 0.34 ms per tick.
- **Retained faces by facing** (`e361e11`): retained GPU 0.197 -> 0.155 ms, GPU frame 0.91 -> 0.875, static fps
  1004.6/1009.5 -> 1039.5/1040.2. Block entity scene: `bereset`/`besign` pixel-identical. The same commit raised
  `SectionFaceGroups`' margin from 1e-3 to 0.25 blocks: view bobbing is in the projection matrix and moves the eye
  up to ~0.11 blocks from the camera position the groups are tested against, so terrain faces in a plane that
  close to the eye could be skipped while walking.
- **Packed terrain draws** (`b696509`): `collect` server static 0.108 -> 0.079 ms, pan 0.096 -> 0.071; terrain scene
  static 0.356 -> 0.284, pan 0.261 -> 0.196 (that scene is GPU bound: ~0.58 ms `awaitGpu`).
- Rejected: nothing reverted; an enchanting table book animation skip was only a probe.

Session start (`2e5d5df`) vs end (`b696509`), checked out in turn in the same tree, three interleaved pairs (median;
`results/s12-start`, `results/vulkanmod-v30`): initial 719.9 -> 727.7 (p99 121.0 -> 128.3, noisy), pan 951.5 ->
995.1 (p99 294.5 -> 361.5), static 995.6 -> 1048.9 (p99 328.0 -> 406.5), particles 839.7 -> 888.9 (p99 300.0 ->
344.1). Screenshots at rows >= 400 differ like same-build pairs (0.025-0.038%). The session's static numbers are
below the eleventh session's 1032 for the same code: the machine's run to run level moves between sessions, so
only compare interleaved runs.

### Terrain (fifth session, `-Pbench.scene=terrain`, median of 3)
| Phase (avg fps) | vanilla Vulkan | VulkanMod v0 (start) | v1 face groups | v2 draw list | v3 compressed |
|---|---|---|---|---|---|
| static | 473.7 | 476.6 | 644.0 | 659.2 | **670.3** |
| pan | 651.4 | 657.7 | 829.4 | 844.2 | **873.1** |
| render thread CPU, static | 0.91 ms | 0.90 ms | 1.21 ms | 0.86 ms | 0.87 ms |
| rebuild (sections/s) | 3864 | - | - | - | 3760 (switch off: 3855) |

Archives: `results/vulkanmod-terrain-v0..v3` (+ `v3-compressed-off`), `v4-tint` and `v5-mesher` (interleaved
A/B runs, `on/` and `off/`); the vanilla terrain runs are in `results/`; dev and probe runs in `terrain-dev/`.

**Where the terrain's GPU time goes** (sixth session, `gpuProfile`, static view, 1.48 ms frames): solid 0.46 ms
(586k quads, ~5.3k draws), cutout 0.77 ms (963k quads), translucent 0.07 ms (97k quads); the rest of the frame
~0.18 ms. Both opaque layers cost ~0.8 ns per quad. With the vertex shader emitting degenerate triangles (nothing
rasterised) solid+cutout still take 1.00 ms, with a position-only shader 0.63 ms: ~50% index/position fetch and
primitive processing, ~30% the other attributes and shader outputs, ~20% rasterisation and fragments.
Measured and rejected: 16 bit indices for the terrain draws (vanilla's shared sequential buffer is 32 bit here;
0.459 vs 0.461 ms), splitting every draw in two (twice the draws, no change: per-draw cost is not the limit),
`ProjMat * (ModelViewMat * pos)` (no change). Translucent compression could save at most ~1% (0.07 ms in total).
Fully underground sections hold 11% of the solid quads and 0.3% of the cutout ones, so finer cave culling would
gain at most ~4%. The cutout layer is mostly leaves (Fancy's cutout leaves draw the faces between leaf blocks,
which show through the holes, so they can't be dropped) and grass. renderpearl 26.3 has no compute shaders or
occlusion queries, so GPU-side culling isn't available.

**Section compiles** (sixth session). JFR of the worker threads: 42% of compile time was biome blending (25 fuzzy
zoomed biome lookups per tinted block, vanilla's per-block cache thrashing), 22% Indigo's AO, ~13% light/shade
lookups, ~8% fluids. `compileBench` (render thread, us per section): all off 681 -> all on 391-413; with only
the biome blend cache off 581-597, light cache off 431-453, state cache off 459-476; face group sorting ~8.
Parallel, all on: 1 thread 410, 19 threads 1039 us per section per thread (hyperthreads, E-cores, all-core
clocks), 18.3k sections/s in total. In the game the visible (surface) sections cost ~2.1 ms worker CPU each and
13-15 of the 19 workers are busy during a rebuild. A/B, interleaved, median of 3: biome blend alone 3911 -> 5119
sections/s; all three switches 3987 -> 6588; static/pan fps unchanged.
Interleaved A/B for the compressed vertices (off -> on): static 657.2 -> 670.3, pan 844.9 -> 873.1; the server
benchmark was neutral within noise for it (`v25`, `v25-compressed-off`). The draw list took the server scene's static
view from 405 to 432 (v24). Quads drawn at the static view: ~0.59M solid + 0.96M cutout with face groups (2.2M
without). Rejected: **diagonal face groups** (the vertical 45 degree planes of cross plants, 12% of the compiled quads
here, ~5% fewer drawn): static 659.2 -> 657.4 and pan 844.2 -> 849.6, no measurable gain. It needs a 1e-5 normal
tolerance and a 0.05 block margin, because rotated vertices aren't exactly diagonal; the patch was in the fifth
session's scratchpad (`diagonal-groups.patch`).

**Rejected: CPU occlusion culling** (seventh session, user's request, dropped after measuring). Design, exact by
construction: at compile time each section kept its vertex bounds and its solid layer quads that are whole block faces,
greedy-merged per plane into rectangles (the occluders are geometry the GPU really draws, opaque, never a solid volume
whose hiding faces may sit in an undrawn neighbour). Each frame, inside `TerrainDrawList.collect` (vanilla's visible
list order, roughly front to back), a section was tested against a coverage buffer and, if drawn, its front-facing
rectangles (eye 0.01 in front of the plane) were rasterized. The buffer worked on the GPU's own samples: the frame's
final projection (captured at `RenderSystem.setProjectionMatrix` in `GameRenderer.renderLevel`, bobbing and nausea
included), 8x8 sample tiles holding a full-coverage depth (1/w, farthest over the tile's sample corners) plus a 64 bit
partial mask with its depth, so faces meeting on a tile (section borders, ledges) fill it together; edges shrunk by
1/16 px against vertex snapping, faces clipped to w >= 0.1, sections with a vertex nearer than that always drawn, and
a section hidden only if behind by a relative 1e-4. Screenshots stayed identical to the reference (differences only
in clouds, water animation and an edge that also flickers between earlier runs).
Measured (terrain scene, static view, one session, `bench.occlusionSweep` of the face size cap):

| faces rasterized / frame | sections hidden of 2426 | fps | render thread CPU |
|---|---|---|---|
| culling off | - | 674 / 679 | 0.83 / 0.72 ms |
| all, 168k | 281 | 41 | 24.3 ms |
| >= 64 px on screen, 15k | 103 | 124 | 8.0 ms |
| >= 1024 px, 1.1k | 99 | 400 | 2.5 ms |
| >= 16384 px, 54 | 99 | 681 | 1.1 ms |

The 99 sections hidden with almost no occluders are only off screen (vanilla's cull frustum is wider than the
screen); occlusion proper hides ~180 sections, ~10% of the drawn quads (solid 586k -> 527k, cutout 963k -> 868k),
worth ~0.12 ms of GPU time, and only with the whole visible surface as occluders: savanna hills are made of 1 block
terraces, so no small set of large faces hides anything. At ~145 ns per rasterized face it can't pay. The server
scene has nothing to gain either (the "building" is the chest/enchanting table cube: block entities and non-full
blocks, no solid walls). It could only pay where few large drawn faces hide a lot (walls of cities, cliffs); solid
*volume* occluders would be far cheaper but are not exact (they differ from vanilla wherever vanilla itself shows a
hole through an unbuilt or graph-culled section). The patch (with the non-MDI path, a settings switch and the sweep)
is in the seventh session's scratchpad (`terrain-occlusion.patch`); results and logs in
`results/terrain-dev/occlusion/`.

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

### Retained shulker boxes, decorated pots and skulls (seventh session)
Block entity scene (section 6.8), 3 interleaved runs each; retention off also turns off the 128 chests:

| | static fps | static p99 low | render thread CPU (static) | pan fps | pan p99 low | alloc (static) |
|---|---|---|---|---|---|---|
| vanilla Vulkan (1 run) | 41.7 | 30.6 | 23.3 ms | 246 | 39.2 | 913 MB/s |
| VulkanMod, retention off | 74.4 | 49.6 | 13.0 ms | 1281 | 85.2 | 1615 MB/s |
| VulkanMod, retention on | **1653** | **539** | **0.28 ms** | **2400** | **628** | 70 MB/s |

All 12.4k block entities are retained (0 extracted per frame). Screenshots on vs off: 14-18 pixels differ on the
edge of one skull row (rounding, as with chests); `beevents`/`bereset` agree (the opened lid and the cracks are the
same; only the fading chat line differs), and `bereset` is identical to the static view. Server scene, interleaved
A/B against the previous commit (2 + 5 runs): static 418.6 vs 422.9, pan 536.4 vs 530.4 (p99 120.8 vs 119.5), i.e.
unchanged (it has none of the new types). Results in `results/blockentities-dev/` and `results/ab-retained/`.

---

## 8. What's next (in priority order)

1. **Entity scene hot spots** (section 6.9, static ~7.1 ms/frame; tenth session profile in section 7). Done in the
   tenth session: block model mesh cache, hashed draw lookup, baked quads a quad at a time. By expected gain:
   - ~~Per switch cost in feature renderer groups~~: measured in the eleventh session at ~115 us/frame in total
     (~197 switches). Reusing draws without `prepare()` saved ~10 us and didn't show in interleaved pairs; see
     section 7, "Eleventh session". Not worth more unless a scene has far more switches.
   - ~~Posed mob models~~: done in the eleventh session (`InstancedParts`, static 137.7 -> 168.5 fps). Left there:
     `setupAnim` and the part tree's `translateAndRotate` (a `Quaternionf` per part, both matrices rotated), and
     armor (`armor_cutout_no_cull`), which stays on the vanilla path because trims draw over it with an `EQUAL` depth
     decal pipeline; it could be instanced if the decal were too. Translucent types (players, horse markings) stay
     vanilla as well.
   - Block displays (2.3 us) and minecarts (1 us) still extract slowly after the mesh cache; profile them.
2. **Per entity and block entity cost in the server scene** (twelfth session measurements in section 7). The static
   view is close to GPU bound (~0.875 ms GPU vs ~0.78 ms render thread), so CPU work there shows in the render
   thread time and the p99; GPU work (terrain 0.42 ms, retained block entities 0.155) shows in the average. Done:
   sign text retained, closed books without extraction, cached table light, reused armor stand states (8th/9th),
   parked idle tickers, validated ticker states, typed armor stand pushes, retained faces by facing, packed terrain
   draws (12th). Ruled out: cheaper entity culling, generic render state reuse (10th). Measured leads:
   - **Block entity extraction** (~0.09 ms/frame at static, all block entities retained or instanced): ~4.7k visited
     per frame, ~4.4k of them frustum culled (~14 ns each, ~0.06 ms; enchanting tables in retained sections and
     sections partly beyond the view distance) and 253 closed books (~75 ns each, ~0.02 ms). Culling whole 4³ cells
     from a per mesh list of non-member indices per cell (keeping index order for the output) could save ~0.03-0.04
     ms; facing the cube there are ~1.1k books per frame.
   - **Client tick** (~1.4 ms, sets the p99): block entities ~0.5 ms (~5.6k enchanting tables at ~90 ns: wrapper,
     bound ticker and block entity loads, the world border check, profiler push/pop), entities ~0.34 (~300 armor
     stands at ~1.1 us), animateTick ~0.27 (vanilla's 1334 random block lookups), textures ~0.16 (animated texture
     uploads), the rest ~0.1.
   - **GPU:** retained block entities still 0.155 ms (vertex work; the entity vertex format is 36 B and its shader
     does lighting per vertex); instanced models 0.065.
   - **Other retainable block entities**: glowing sign text (its outline depends on the camera distance to the sign,
     16 blocks; two variants), bells and lecterns at rest, copper golem statues; banners animate (wind), beds are
     block models in 26.x, shelves and campfires render items.
   - **Retained first-frame cost** after a teleport (budget 2048 block entities per frame).
   - Occlusion culling of block entities is **not** exact (chests are 14/16 wide; the ones behind show through).
3. **Allocation** (~0.35 MB/frame at static since the eighth session; was ~2.3): `Float` boxing of chest openness per submit, submit records,
   `PoseStack.Pose`, glyph instances of non-sign text, `CompactVectorArray` per sort.
4. **Settings screen follow-ups** (the port itself is done, section 4).
   - Give every new optimisation that changes output or could misbehave a `Config` switch and an entry on the
     Optimizations page (with a tooltip and impact), instead of a system property.
   - At GUI scale 4 on 1080p the list is too narrow for the side tooltip, so tooltips go below the list and long
     ones cover the Apply/Done buttons (inherited layout).
   - Only `en_us` has the new keys; terrain options (old advanced culling, indirect draw, ...) come back with the
     terrain renderer.
5. **Other CPU hot spots.**
   - Entity extraction (~91 us/frame at static after the ninth session: ~16 us culling, the rest extraction; see
     section 7, "Tenth session").
   - Entity culling: Sodium and vanilla render the same ~188 entities.
   - Armor stands use `entity_cutout` (no culling), so the CPU back-face skip doesn't apply to them.
6. **Terrain**, after the sixth session (section 7, "Terrain"). The view is GPU bound on per-vertex work (see
   "Where the terrain's GPU time goes"); 16 bit indices, translucent compression and draw counts were measured and
   don't matter. Ideas:
   - Fewer vertices: anything exact must skip whole invisible faces/sections (occlusion), which renderpearl can't
     do on the GPU. An exact CPU occlusion test was built and rejected in the seventh session (section 7): in this
     scene occlusion hides only ~10% of the quads, and only with ~168k occluder faces per frame.
   - Section compiles (now ~400 us per section single-threaded): Indigo's `AoCalculator` (~22% before the caches,
     Fabric internals, so mixins there are fragile), fluids (~8%), Indigo's quad transform/emit overhead.
   - The vanilla occlusion graph bug after `allChanged` (section 6.7), which also leaves holes for players who
     change a video setting while standing still.
   - Singleplayer chunk loading runs at the integrated server's speed (~60 chunks/s), not the client's.

   The notes below on a full terrain renderer predate the fifth session.
   **Terrain renderer** (old `render/chunk/**`), the large piece. Only do it if it beats vanilla 26.3, which
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
7. **Fabric Renderer API 17.** Needed once the mod has its own mesher again. Re-port `EncodingFormat`/`QuadViewImpl`
   /`MutableQuadViewImpl` from Indigo 26.3, implement the 3-method `Renderer` plus an `AltModelBlockRenderer`,
   and register feature renderers via `FeatureRendererRegistry`. Do **not** re-add the old `SubmitNode*` /
   `ItemStackRenderState*` mixins; Fabric now does that itself.
8. **Smaller ports.** Clouds (mixin vanilla `CloudRenderer` `prepare`/`render`; there is no separate clouds pass
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
  `ModelCulling`, `ModelGeometryCache`, `ParallelUploadCopy` and `RetainedBlockEntities` are static, render-thread-only
  state (`ParallelUploadCopy`'s worker only copies chunks of the current job; `RetainedBlockEntities.markDirty`/`clear`
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
