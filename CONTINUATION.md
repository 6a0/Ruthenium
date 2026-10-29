# Ruthenium (formerly VulkanMod) → Minecraft 26.3 port: continuation notes

This is a hand-off document for a new session. It covers what was decided, what has been built, how to test it,
what the numbers are, and what to do next. Read it all before changing code.

Last updated: 2026-10-28 (seventeenth session; earlier: 2026-10-27, sixth session), with the GPU probe findings and the faster section compiles (biome blend,
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
Thirteenth session (section 7, "Thirteenth session"): the entity scene's hot spots (section 8, item 1). An entity cost
probe per entity type, then **items, entity block models (item frames, block displays) and armor are GPU instances**,
cached block model meshes are submitted without copies, item bounding boxes and part rotations are kept across frames,
and zero moves skip the entity collision query. Entity scene in the same tree, start -> end: static 165.6 -> 268.5 fps
(p99 low 98.3 -> 148.3, render thread 5.77 -> 3.58 ms), pan 762.5 -> 1015.0, client tick 2.10 -> 1.55 ms. The server
scene is unchanged (it has no such entities).
Fourteenth session (section 7, "Fourteenth session"): the entity scene's model prepares (1.07 -> 0.82 ms per frame).
**Faces whose texture is transparent there are left out** (spider and enderman eyes, horse markings; exact), **layers
reuse the part poses of the model's first submit** (villager clothing, eyes, markings, collars, trims), part batches are
found through arrays, and instanced submits fetch no vertex builder. Entity scene in the same tree, start -> end: static
280.4 -> 310.0 fps (render thread 3.44 -> 3.16 ms), pan 1052.0 -> 1112.9; the server scene is unchanged.
Fifteenth session (section 7, "Fifteenth session"): the entity scene's extraction and submission. **Entity culling
without allocations or renderer lookups** (renderers kept on the entity type; the pan culls all ~1.3k loaded entities
every frame) and **light kept per entity** until it moves or the client's light data changes. Name tag attribute reads
and villager clothing identifiers were measured and binned (within noise). Entity scene in the same tree, start -> end:
static 321.7 -> 331.5 fps (render thread 3.01 -> 2.95 ms), pan 1131.8 -> 1195.9 (p99 272.3 -> 294.0); the server scene
is unchanged.
Sixteenth session (section 7, "Sixteenth session"): the entity scene's **client tick** (section 8 item 1), with Lithium
consulted for ideas (none of its code). **Entity collision searches skip entity sections without collidable entities**
(an idea from Lithium's entity class groups), **animateTick** skips ambient particle biome lookups where no biome has
them and steps its random seed without compare-and-set, **boats** decide full cube ground tests from boxes and push only
the local player, and **resting entities keep their supporting block** while their chunk sections don't change. Lithium's
direct entity section lookup and an air-only skip of inside-block checks measured within noise and were binned. Entity
scene in the same tree, start -> end: client tick 1.596 -> 1.261 ms (pan 1.465 -> 1.168), static
321.5 -> 331.1 fps (p95 219.3 -> 241.5, p99 155.2 -> 165.8), pan 1159.7 -> 1193.6; server
scene client tick 1.23 -> 1.13-1.19 ms (animateTick), fps unchanged (static 1084.8 vs 1084.5, GPU bound).
Seventeenth session (section 7, "Seventeenth session"): the server scene's per block entity cost (section 8 item 2).
**Enchanting tables tick in one loop** (client tick 1.15 -> 0.93 ms, static p99 low 459 -> 505), **retained quads are
drawn by plane** (179k -> 132k quads per frame; static 1063 -> 1095 fps, GPU bound), closed books compute their tilt once,
**lecterns, bells and copper golem statues are retained** (new `blockEntityKinds=extras` scene: static 211 -> 1220 fps),
**retained batches are only rebuilt when their blocks or light change** (extras p99 low 280 -> 715) and are kept 30 s
behind the camera. Three attempts binned (sprite pipeline repeats, cell index masks, front-to-back plane order). Same
tree, start -> end: server scene static 1085.0 -> 1126.5 fps (p99 446.3 -> 507.2), pan 1029.2 -> 1055.5 (p99 380.0 ->
436.8), client tick 1.236 -> 0.931 ms; the block entity scene's static view lost 0.6% (1773.5 -> 1763.1, GPU, the plane
order), the entity scene is unchanged.
Nineteenth session (section 7, "Nineteenth session"), two agents in parallel from `3c04a06`, merged into `26.3` afterwards.
Part 1 (on `26.3`), the four open items of section 8: **transparent face masks are kept per texture** (instanced parts
off: static 196.0 -> 221.6 fps), **glowing sign text is retained** (mixed scene with `-Pbench.glowingSigns=true`: static
773.9 -> 1288.8), **resting entities keep their block collision answer** (client tick 1.429 -> 1.360 ms with gravity);
retained builds spread over frames were binned. Part 2 (branch `worktree-agent-aa744c1ec3f1ed79f`, section 7 "Players and
holograms"): other players and holograms, which no scene had. A **players scene** (`-Pbench.scene=players`, section 6.10: 150 mannequins on
the remote player render path with every player layer, 120 armor stand and text display holograms of 4 lines). Players
were largely vanilla (their body is translucent, so not instanced; everything they wear was already instanced), name
tags and text displays fully so. **Name tags keep their text layout** (vanilla rebuilt the display name and ran ICU bidi
and two glyph layouts per name tag and frame; lobby static 269.8 -> 318.6 fps), **text display lines keep theirs**
(text holograms 1160.6 -> 1293.8), **text displays are culled by the extent of their text** (vanilla never culls size-0
displays, i.e. every hologram plugin's; lobby pan 1066.3 -> 1438.9) and **empty armor stands skip their layers** (stand
holograms 902.4 -> 949.2). Lobby: vanilla 153.6 / 613.2 fps (static / pan); Ruthenium in the same tree, start -> end
(five pairs): static 271.6 -> 326.4 (render thread 3.57 -> 2.96 ms), pan 959.8 -> 1438.5; entity scene pan 1148.8 ->
1184.4, mixed scene unchanged. Found: `TransparentFaces` costs 150 players with resource skins 0.5 ms per frame (the cache
miss of section 8; fixed by part 1, not measured in the players scene yet), and each player body's quads are sorted every
frame (1.8 us per player).


**Renamed to Ruthenium (2026-10-28, eighteenth session).** The project is now **Ruthenium**: mod id `ruthenium`, package
`net.ruthenium`, assets namespace `ruthenium`, `ruthenium.mixins.json` / `ruthenium.accesswidener`, config
`config/ruthenium_settings.json`, system properties `-Druthenium.<field>` / `-Druthenium.bench.<name>`, benchmark tasks
`benchRuthenium` / `benchRutheniumGl` (labels and run dirs `ruthenium` / `ruthenium-gl`; results are now named `ruthenium-*`,
older archives keep `vulkanmod-*`). The README was removed, then rewritten by the user (`README.MD`, Ruthenium logo, description, credits). The GUI's primary colour is **#cf2743**
(`VGuiConstants.PRIMARY_RGB` / `COLOR_PRIMARY`, used for the accents, the scrollbar and the mod name).
Mentions of "VulkanMod" further down are history: the upstream mod, or this mod before the rename.

- **New scope:** Ruthenium is an all-in-one tweak designed to work as efficiently as possible: reduce RAM usage and
  loading times, fix bugs, and increase frame rate, latency and consistency **in all scenarios**. This widens the
  earlier "port VulkanMod's optimisations" strategy (section 1): memory, load time and bug fixes are in scope too, not
  only rendering speed.
- **Icon:** `assets/ruthenium/rlogo.png` (960×960 RGBA, transparent background) is both the mod list icon
  (`fabric.mod.json`) and the settings screen icon (`ModSettingsRegistry`); VulkanMod's `vlogo*.png` were removed.
- **Still to decide:** the issue templates in `.github/` and the
  Ko-fi button on the settings screen (`VOptionScreen.KOFI_URI`) still point to upstream VulkanMod; `fabric.mod.json`
  still lists `Collateral` as the author.
- **Name history:** briefly Rubidium, then Thallium (both already used: Rubidium is a Forge Sodium port, Thallium a
  Modrinth modpack). Ruthenium was free when checked: no public Modrinth project (an old, deleted or private
  kel-cu "Ruthenium" mod may still hold the `ruthenium` slug), no CurseForge result found, and only two archived GitHub
  experiments use the name.

---

## 1. TL;DR

- **Branch:** `26.3`. The old 1.21.11 code is still on `dev` (upstream `xCollateral/VulkanMod`), and
  `git show dev:<path>` is how to pull old sources back in when porting them.
- **Strategy:** port VulkanMod's *optimisations* only. The mod no longer ships its own Vulkan device layer.
  26.x has a new GPU abstraction (`com.mojang.renderpearl`) with its own Vulkan backend, and the mod runs on
  vanilla's backends (GL or Vulkan) through public renderpearl and vanilla APIs.
- **Testing:** every change is benchmarked with the automated client gametest (`./gradlew benchRuthenium`,
  3 runs) and checked for visual parity against vanilla screenshots, then committed with the numbers in the
  commit message. The user asked for this workflow: *commit as you test changes with the benchmark*.
- **Results so far** (median of 3 runs, vanilla Vulkan backend): static view **25 → 1032 fps**, pan **61 → 998 fps**
  (pan p99 low 18 → 294). Sodium 0.9.3-beta.1 on the same benchmark: 63 / 163. On vanilla's OpenGL backend
  (`benchRutheniumGl`) the static view ran at 344 fps (before the eighth session; not re-measured).
- **Block entities at rest are retained on the GPU** (`RetainedBlockEntities`; chests since the third session, at
  the user's choice, shulker boxes, decorated pots and skulls since the seventh, sign text since the eighth, lecterns, bells and copper golem
  statues since the seventeenth, glowing sign text since the nineteenth, switch "Retained Glowing Sign Text"): their
  geometry lives in per-section vertex buffers and is drawn with copies of the entity and text pipelines that add
  `ModelOffset`. Switch
  ("Retained Block Entities"): Optimizations page of the settings screen, or `-Druthenium.retainedChests=false`
  (`-Pvm.retainedChests=false` on bench tasks; the field kept its old name).
- **Closed enchanting table books and armor stands are GPU instances** (`InstancedModels`): model-space mesh once
  per group, a pose + light per instance per frame. Since the eighth session closed books skip the extraction
  entirely (pose computed from the block entity, light cached until a section is marked dirty). Switch: settings
  screen or `-Druthenium.instancedModels=false`. Since the ninth session armor stands are extracted into render
  states reused across frames (`ReusedRenderStates`, switch `-Druthenium.reuseEntityStates`).
- **Entity scene** (tenth session, `-Pbench.scene=entities`, section 6.9): ~1.3k entities of 30 types. Vanilla Vulkan
  86 fps at its static view, VulkanMod ~330 (~310 before the fifteenth session, ~280 before the fourteenth, ~166 before the thirteenth, ~138
  before the eleventh's instanced parts). Mob parts, armor, items (dropped, framed, held, displays) and the block models of entities (item frames,
  block displays, minecart blocks) are drawn as GPU instances (`InstancedParts`; switches "Instanced Mob Parts",
  "Instanced Armor", "Instanced Items", "Instanced Entity Blocks"). The block model quads are cached across frames
  (`BlockModelMeshCache`, "Cache Entity Block Models"), feature renderer groups find their draw with a hash lookup
  (`RenderTypeFeatureRendererGroupM`, "Fast Draw Lookup"), and `BufferBuilder` writes baked quads a quad at a time
  (`BufferBuilderM.putBakedQuad`, "Fast Item Vertices"; now only for items that aren't instanced). Since the
  fourteenth session model faces that draw nothing in their texture are skipped (`TransparentFaces`, "Transparent Face
  Culling") and layers reuse the part poses of the model's first submit (`ModelPoseReplay`, "Reuse Layer Poses").
  Since the fifteenth session entities are frustum culled without allocations or renderer lookups
  (`EntityRendererCache`, "Fast Entity Culling") and keep their light across frames (`EntityLight`, "Keep Entity
  Light"). Since the sixteenth session the client tick is cheaper: entity collision searches skip sections without
  collidable entities ("Fast Entity Collisions"), animateTick ("Fast Block Animation Tick"), boats ("Fast Boat Ticks")
  and the supporting block of resting entities ("Keep Supporting Blocks"); since the nineteenth resting entities also keep
  their block collision answer ("Keep Resting Collisions") and transparent face culling keeps its masks per texture (it was
  slower than no culling with instanced parts off). Section 8 lists what is left there.
- **The settings screen is ported** (fourth session): the video settings button opens VulkanMod's screen with Video,
  Graphics (vanilla options through their `OptionInstance`s) and Optimizations (the mod's switches, `Config`,
  `config/ruthenium_settings.json`). `-Druthenium.<field>=true|false` overrides any switch for a launch.
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
- **Server scene, static view: ~0.72 ms render thread, ~0.84 ms GPU per frame** (seventeenth session; 0.78 / 0.875 in
  the twelfth). It is GPU bound, so CPU savings there show in the render thread time and the p99, not much in the average. Its p99 low
  is set by the 2% of frames that run a client tick (~1.4 ms per tick); see section 7, "Twelfth session".
- **Nineteenth session, part 1** (section 7, "Nineteenth session"): the four open items. Kept: transparent face masks per texture, glowing sign text
  retained (mixed scene with `-Pbench.glowingSigns=true`: static 774 -> 1289 fps), resting block collisions (client tick
  1.429 -> 1.360 ms with gravity). Binned: retained builds spread over frames (more slow frames after a join, not fewer).
- **Nineteenth session, part 2: players and holograms** (`-Pbench.scene=players`, section 6.10 and section 7 "Players and
  holograms"): name tags and text display lines keep their text layout ("Keep Name Tag Text", "Keep Text Display Text"),
  text displays are culled by their text ("Cull Text Displays"), empty armor stands skip their layers ("Skip Empty Armor
  Stand Layers"). A player costs ~12 us of render thread per frame (vanilla ~30), a hologram line ~1-1.7 us (vanilla
  ~1.8-3.5); the player body (translucent, 2.8 us) is the largest item left.
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

### New code (`src/main/java/net/ruthenium/`)

| File | What it does |
|---|---|
| `Initializer.java` | Client entrypoint: version, logging, loads `Config`. Platform and FRAPI registration are not ported yet |
| `config/Config.java` | The mod's switches (`blockEntityCulling`, `retainedChests`, `instancedModels`, `modelBackFaceCulling`, `particleSpawnLimit`, `skipIdleTicks`, all on by default), Gson to `config/ruthenium_settings.json` (unknown fields of the old 1.21.11 file are ignored). `-Druthenium.<field>=true|false` overrides a field at load (and is written back if the user applies settings in that session). The features read `Initializer.CONFIG.<field>` at use, so toggles apply immediately; retained chests and instanced models also `clear()` when toggled |
| `config/**` (settings screen) + `mixin/screen/{OptionsScreenM,OptionInstanceAccessor,TooltipAccessor}.java` | Port of the old screen (`VOptionScreen`, `VOptionList`, widgets, search, Apply/Undo, the `ruthenium:mod_settings_registry` entrypoint API for other mods). `OptionsScreenM` swaps the supplier of the video button (`@ModifyArg` on `openScreenButton` when the message is `VIDEO`); Shift+P in the screen opens vanilla's. Pages: **Video** (window mode = vanilla `fullscreen` + `exclusiveFullscreen`, exclusive fullscreen mode of `window.findBestMonitor()`, framerate, vsync, inactivity limit, GUI scale, brightness, graphics API, attack/autosave indicators), **Graphics** (preset first, distances, chunk updates, filtering/anisotropy, particles, clouds, leaves, improved transparency, chunk fade, AO, biome blend, shadows, entity distance, mipmaps, weather radius, menu blur, vignette), **Optimizations** (the `Config` switches). `VanillaOptions` wraps a vanilla `OptionInstance`: `set` runs vanilla's listener (preset, `allChanged`, sampler reset, ...), and caption, value label (with the leading `options.generic_value` caption stripped) and tooltip come from the instance through accessors. After Apply every option re-reads its value (the preset changes the others); texture reloads (mipmaps/filtering/anisotropy) happen when the screen closes, like vanilla's video screen. Drawing goes through `GuiRenderer` → `GuiGraphicsExtractor`; the cycling arrows are `PolygonRenderState` triangles (a quad of `RenderPipelines.GUI` with the last vertex repeated; `guiRenderState`/`scissorStack` opened by the access widener). Text input uses `charTyped`; shortcut letters compare `KeyEvent.shortcutKey()` (SDL keycodes, lowercase ASCII for letters). Dropped from the old screen: device/frame queue/builder threads/wayland/texture animation options, advanced chunk culling, indirect draw, unique opaque layer, backface culling (terrain options come back with the terrain renderer), the update checker button |
| `render/chunk/SectionFaceGroups.java` + `mixin/render/chunk/{SectionCompilerM,SectionCompilerResultsM}.java` + `CompiledSectionMeshM` (`interfaces/FaceGroupsHolder`) | Solid and cutout layer meshes (sequential quads) are reordered at compile time (stably, after `BufferBuilder.build`, through a wrap of the results' `Map.put`) into 7 groups: one per axis-aligned facing (by the quad's winding normal from its diagonals; all 4 vertices must share the plane coordinate) plus the rest. Each group keeps its lowest `normal . vertex`. When the draw list is built, a layer's draw is replaced by draws of the groups the eye can be in front of (margin 1e-3), merging adjacent ones. Exact: terrain pipelines cull back faces, and a quad is back facing iff the eye is behind its plane. Switch `terrainFaceCulling` (toggling calls `allChanged`). The non-MDI path (`extractSectionDrawGroups`) gets the same through a `List.add` wrap. **Mixin pitfall seen here:** array field initializers in a mixin were not merged before a constructor `@Inject` ran once a second such field existed; the NPE happened in a compile worker, where vanilla's `runTask` catches it and requeues the task silently (symptom: a compile queue that never drains). The arrays are now allocated in code |
| `render/chunk/TerrainDrawList.java` + `mixin/render/chunk/LevelRendererM.java` (`@Overwrite prepareChunkRendersIndirect`) + `interfaces/PackedSectionDraws` | Builds vanilla's multi-draw-indirect commands: the same draw groups (per vertex/index buffer heap; translucent ones consecutive-only and back to front) and commands, without the 2 hash lookups per section and layer in `UberGpuBuffer.getAllocation` and without a record per command. Since the twelfth session a compiled mesh's solid and cutout draws are packed on first use into one int array (drawn flag, index count, base vertex, the face groups' starts, planes with the margin applied and always-drawn mask) and a `GpuBuffer[]` on the mesh (`CompiledSectionMeshM`): their vertex allocation never moves until the mesh is released. Translucent layers (index buffers re-uploaded under the same mesh when resorted) and meshes with custom index buffers in those layers take the lookups every frame. Face group runs come from a mask of drawn groups and a table of runs per mask (`SectionFaceGroups.addVisibleCommands(int[], ...)`). Verified: old and new collect side by side every frame, identical groups/commands/section infos over 22k server and 76k terrain frames (incl. rebuilds); the mask path on 16M random cases. Commands go into reused int arrays and one bulk copy per group into a VulkanMod `MappableRingBuffer` of indirect parameters, rotated each frame. Vertex sizes per layer are read per frame (they change with `CompressedTerrain`) |
| `render/chunk/CompressedTerrain.java` + `CompressedTerrainBuilder.java` + `mixin/render/chunk/{ChunkSectionLayerM,ChunkSectionsToRenderM}.java` + `assets/ruthenium/shaders/core/terrain_compressed.vsh` + `interfaces/SectionDispatcherReset` | Solid/cutout vertices in 20 bytes: `Position` RG32_UINT = 3x21 bit fixed point (1/65536 block, offset -8, range [-8, 24)), `Color` RGBA8, `UV0` RG16_UINT (/65536), `UV2` as vanilla. `ChunkSectionLayer.pipeline()`/`vertexFormat()` return copies of vanilla's terrain pipelines (built through a `RenderPipeline.Snippet` of the vanilla pipeline's properties, with the vertex shader and binding 0 replaced; also for the wireframe override). The section compiler gets `CompressedTerrainBuilder` (a `BufferBuilder` subclass handling the 11-arg `addVertex`, chained `addVertex(x,y,z)` and `setUv`) through a `NEW` wrap in `getOrBeginLayer`. The format is chosen when vanilla creates the section dispatcher (`invalidateCompiledGeometry` with a null dispatcher), because the uber buffers are sized per vertex. The switch `compressedTerrain` calls `LevelRenderer.ruthenium$recreateSectionDispatcher` (like `resetLevelRenderData`, but keeping the occlusion graph's loaded chunks and empty sections, which only fill as chunks load) and then `allChanged`. Translucent layers keep vanilla's format (their quad sorting reads float positions). Little-endian hosts only |
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
| `render/blockentity/RetainedBlockEntities.java` + `ModelSubmitCapture.java` + `mixin/render/blockentity/{ClientLevelM,BlockEntityRenderDispatcherM,BlockEntityDataPacketM,RenderPipelinesM}.java` + `mixin/world/{ShulkerBoxBlockEntityM,DecoratedPotBlockEntityM}.java` + `assets/ruthenium/shaders/core/retained_entity.vsh` | Block entities at rest kept on the GPU. Candidates (exact vanilla classes and renderers): chests (lid idle incl. the other half of a double chest), shulker boxes (`CLOSED`, progress 0 at partial tick 0 and 1), decorated pots (no wobble, or game time - start > duration) and skulls (`getAnimation` constant, dragon/piglin heads not `POWERED`; heads with an owner profile never, their skin loads asynchronously). Others of these types are "moving": left out and re-checked every frame. Per render section (keyed by `SectionPos`), the real renderer runs into `ModelSubmitCapture` (a `SubmitNodeCollector` that records model submits; pots submit their 7 parts as `Model.Simple`; anything else, e.g. crumbling, rejects the block entity); a block entity is kept only if every model has no outline and a render type that maps to a retained pipeline (`entity_solid`/`_z_offset_forward`, `entity_cutout_cull`, `entity_cutout`/`_z_offset`) with `DEFAULT_TEXTURING`. Models are written with all faces, through `ModelGeometryCache` for `Float` 0 states, else with vanilla's `setupAnim` + `renderToBuffer`, relative to the section origin, into one vertex buffer per render type. `LevelExtractorM` skips members; batches are drawn after `featureFrame.executeSolid` in `LevelRendererM` with `PIPELINES` (vanilla entity pipelines + a vertex shader adding `DynamicTransforms.ModelOffset`; registered via `RenderPipelines.optionalPipelines`, so a compile failure just turns it off), the render type's textures (`prepare()` once per type per frame) and its layering transform applied to the model view (the skulls' z offset; `RenderType.state` and `RenderSetup`'s transforms are widened in the access widener). Transforms are written after `prepareFrame` with fresh vectors (`DynamicGpuDataStorageMapped` dedups against the last record by `equals`), and the sequential index buffer is grown there too (not inside a pass). Rebuilt when the section's mesh changes, when the section or a neighbour hits `LevelExtractor.setSectionDirty(IIIZ)` (block and light changes; a double chest's light can come from the neighbour), on chest, shulker box and pot `triggerEvent`, block entity data packets (pot sherds, skull profiles; they can arrive after the block), `ClientLevel.destroyBlockProgress`, and when a moving block entity comes to rest. Sections partly within the view distance (64) use the vanilla path that frame. Since the twelfth session, buffers of pipelines that cull back faces (chests, shulker boxes and pots: `entity_cutout_cull`; sign text: the polygon offset text pipelines) are sorted into `SectionFaceGroups` at build time and each frame draws only the runs of groups that can face the eye (switch `modelBackFaceCulling`; GPU 0.197 -> 0.155 ms at the server scene's static view, which was vertex bound: the same at 960x540). Budget 2048 block entities rebuilt per frame; batches unused for 1200 frames are freed. Cleared on level change, `allChanged` and resource reload. **Signs** (eighth session; `SignBlockEntity`/`HangingSignBlockEntity` with `StandingSignRenderer`/`HangingSignRenderer`, no glowing or obfuscated text on either side): `ModelSubmitCapture` records text submits; a text is retained only if vanilla would put it in the **solid** phase (`SubmitNodeCollection.canRenderAsSolid`: colour alpha 255, no background) with `POLYGON_OFFSET` and no outline, which is why the draw order can change. Its glyphs are prepared through `SignTextCache.prepareText` (as `TextFeatureRendererM` does) and written with `TextRenderable.render` into a part per render type (`renderType.format()`, POSITION_TEX_LIGHTMAP_COLOR), drawn with `TEXT`/`TEXT_GRAYSCALE`: copies of vanilla's `TEXT_POLYGON_OFFSET`/`TEXT_GRAYSCALE_POLYGON_OFFSET` (same state, built from a `Snippet` like `CompressedTerrain`) with `retained_text.vsh`. Extra rebuild triggers: `SignBlockEntity.setText` on the client (`SignBlockEntityM`; the edit screen calls it while typing), `FontSet.resetTextures` (`FontSetM`, clears everything) and a change of `Minecraft.isTextFilteringEnabled` (checked in `beginFrame`, clears) |
| `render/blockentity/InstancedModels.java` + `assets/ruthenium/shaders/core/instanced_entity.vsh` | GPU instancing of models that only differ by pose. Two entry points: (1) the extraction (`LevelExtractorM`) calls `tryAdd(dispatcher, state, …)` after `tryExtractRenderState` for `EnchantTableRenderer`: the real renderer submits into `ModelSubmitCapture`, and a single model with a `modelSpaceKey` (closed book) becomes an instance instead of a submitted render state; (2) `ModelFeatureRendererM.prepareModel` calls `tryAdd(submit)` for models in `POSED_MODELS` (`ArmorStandModel`): after `setupAnim`, a snapshot of all parts' transforms/visibility keys the group (exact for pure `ModelPart` trees; max 32 groups per model) and the mesh is `model.root().render` with the identity pose and CPU culling off. An instance is 3 pose rows + light (52 B, `INSTANCE_FORMAT`, step rate 1). Poses must be orthogonal (rotations, or the `scale(-1,-1,1)` mirror of living entities, where vanilla's normal matrix equals the pose). Instanced copies of `entity_solid`, `entity_cutout` (per-face lighting, no cull) and `entity_cutout_cull` (`PIPELINES`). Per group (model, key, render type, UV mapping, overlay, tint): a model-space mesh from `ModelGeometryCache.writeModelSpace` (identity pose, all faces, duplicates skipped) and a `MappableRingBuffer` for the instances (rotated at the next prepare, so its fence covers the frame that used it). Drawn after `RetainedBlockEntities` with `PIPELINE` (vanilla `entity_solid` + an instanced vertex shader). Cleared with the retained chests. **Closed book fast path** (eighth session, `tryAddClosedBook`, called by `LevelExtractorM` before `tryExtractRenderState`): once one closed book went through the capture path (which records its group and renderer), vanilla `EnchantingTableBlockEntity`s with `open == oOpen == 0` and no breaking progress skip the render state and capture: the pose is built with the same JOML calls in the same order as the extraction + `EnchantTableRenderer.submit` (`translate` x3, `rotateY(-yRot)`, `rotateZ(80°)`; bit-identical, checked on 16.7M books), plus `shouldRender`'s distance test. The light and the `isValid` block state check are cached on the block entity (`EnchantingTableBlockEntityM`, `interfaces/CachedBlockEntityLight`) under a generation stamp that every `setSectionDirty` (and `clear`) bumps (`invalidateLight`); 69M cache hits matched fresh lookups |
| `render/entity/InstancedParts.java` + `mixin/render/entity/model/ModelPartM.java` + `mixin/render/feature/PreparedGroupM.java` + `interfaces/InstancedPartHolder.java` + `assets/ruthenium/shaders/core/instanced_part.vsh` | **Instanced parts of posed models** (eleventh session, switch `instancedParts`, "Instanced Mob Parts"). While the world's features are prepared (`ModelCulling.isWorldPass`), a `FeatureRenderDispatcher$PreparedGroup` that isn't strictly ordered activates it (`PreparedGroupM` on `prepare` HEAD/RETURN), and `ModelFeatureRendererM.prepareModel` turns it on around vanilla's `setupAnim` + `renderToBuffer` for submits without a sheeted decal whose render type uses `ENTITY_SOLID`, `ENTITY_CUTOUT` or `ENTITY_CUTOUT_CULL` (no blending, so always the solid phase). The `ModelPart.compile` overwrite then adds one instance per part instead of writing its cubes: the pose's rows, the normal matrix's rows, light + overlay (RGBA16_SINT) and color (96 B, `INSTANCE_FORMAT`). Meshes (`PartBatch`, per render type and UV mapping; since the fourteenth session each `ModelPart` keeps them in `PartBatches`, arrays of the batches and of their render type and UV mapping references, compared without loading the batches: villager parts have ~28) are the part's cubes written once with the identity pose, all faces; max 4096, dropped by `clear()` (resource reload, switch) through a generation stamp. At the group's `prepare` RETURN its batches' instances are copied into the frame's buffer as one draw per batch (render type prepared once per frame); the draws run at `PreparedGroup.execute` RETURN, i.e. right after that group's own draws, so they keep their place among submit orders (`order(n)`: villager clothing is the same model one order later and relies on equal depth) and phases. Instanced copies of the three pipelines with `instanced_part.vsh`, uploaded through a `MappableRingBuffer` after `prepareFrame`. **Positions are bit-identical to vanilla's:** the shader computes `m00*x + (m10*y + (m20*z + m30))` with `precise` (no contraction; `GL_ARB_gpu_shader5`), the same operations in the same order as JOML 1.10.9's `transformPosition` (no `Math.fma` unless `joml.useMathFma`). A first version with `dot()` broke the equal-depth ties of horse markings, which are drawn by vanilla (translucent) on the instanced body. Normals are transformed in full precision and normalized (vanilla quantizes them), which changes ≤5 pixels at threshold 1 per frozen close-up. **Thirteenth session:** (1) `armor_cutout_no_cull` (armor, plain trims with their UV mapping, saddles, horse armor) gets an instanced copy (`NO_OVERLAY`, no `SAMPLER1`, per face lighting, no cull; switch `instancedArmor`); decal trims (none in vanilla) stay vanilla and test depth for equality against it. (2) **Items** (`addItem`, from `ItemFeatureRendererM`'s `prepareSubmit` overwrite; switch `instancedItems`): an item submit without foil whose quads are all of one `item_cutout` type becomes one instance of an item-space mesh per quad list (vanilla's lists belong to baked models; `IdentityHashMap`) and tint colors, with the quads' tints in the mesh color and their light emission in UV2, raised to the instance's light in `instanced_item.vsh` as `LightCoordsUtil.lightCoordsWithEmission` does (vanilla's `core/item` fragment shader). Lists of other types are remembered as not instanced. (3) **Entity block models** (`addBlockMesh`, from `ExtendedBlockModelFeatureRendererM`, a `@Pseudo` redirect of the `Mesh.outputTo` call in Indigo's `ExtendedBlockModelFeatureRenderer.buildGroup`; switch `instancedBlockModels`): per cached mesh (stable since `BlockModelRenderStateM`), render type of its first layer and tint colors, a mesh with the colors, normals (vertex or face) and light Indigo's `bufferQuad` writes; each vertex's light is raised to the submit's in the shader as Indigo's `minLightmap` (`ExtraLightCoordsUtil.smoothMax`) does, emissive quads at full brightness. `item_cutout` types use `ITEM_CUTOUT_MESH`, `entity_solid` and `entity_solid_z_offset_forward` (item frames submit their frame with it) `SOLID_MESH`, both with the `MESH_LIGHT` shader variant. All of these were pixel-identical to vanilla's path in frozen close-ups with the switch flipped, on the Vulkan and OpenGL backends |
| `render/entity/ReusedRenderStates.java` + `mixin/render/entity/{LevelExtractorEntityM,EntityRendererAccessor}.java` | `@Overwrite LevelExtractor.extractEntity`: vanilla `ArmorStand`s with `ArmorStandRenderer` are extracted into render states kept from the previous frame (a pool reset at the head of `extractVisibleEntities`) instead of `createRenderState(entity, partialTicks)`: `extractRenderState` + `finalizeRenderState` (invoker) into the reused state. Vanilla's call took ~1.05 us per armor stand, the reused one ~0.27 us: allocating the state and extracting into it in the same compiled method made the extraction ~5x slower (each step alone, or extracting into an existing state, was 0.07-0.25 us; not JIT tiers, deopts, stack banging or a mixin, all checked). States are submitted and cleared in the frame they are extracted in (`LevelRenderer.submitFeatures`). The extraction overwrites every field except `nameTagAttachment`, `distanceToCameraSq`/`nameTag`/`scoreText` (only with a camera) and `ArmedEntityRenderState`'s arm poses (hidden by the armor stand's own `Rotations` fields, only set by humanoid mobs), which are reset to a new state's values. A temporary reflective deep compare of 2.4M reused states against fresh vanilla ones (139 fields each, incl. the item states) found no difference. Exceptions fall back to vanilla's path (which reports them). Cleared on `setLevel`/`allChanged`. Switch: "Reuse Entity Render States", `-Druthenium.reuseEntityStates` |
| `render/entity/BlockModelMeshCache.java` + `mixin/render/entity/BlockStateModelWrapperM.java` | The Fabric Renderer API overwrites `BlockStateModelWrapper.update` to emit the model's quads into a new `MutableMesh` on every call, i.e. per item frame, block display, minecart block and enderman's block per frame (decoding and encoding every baked quad; ~2 us per item frame). A `@WrapOperation` on its `emitQuads` call (priority 1100, after Fabric's overwrite; `require = 0`, so without the overwrite nothing changes) looks the output up by the model's geometry key (`FabricBlockStateModel.createGeometryKey` with a random of its own seeded like `update`'s, `@Local(argsOnly)` seed; by FRAPI's contract the key determines `emitQuads`' output for a cull test that culls nothing, which `update` passes), wrapped in `OpaqueKey` when Fabric's default `emitQuads` forces leaves to the solid layer (`ModelBlockRenderer.forceOpaque` with the cutout leaves option). A miss emits into a private `MutableMesh` and stores `immutableCopy()`. Since the thirteenth session the render state keeps the cached mesh itself (`BlockModelRenderStateM`, `interfaces/CachedMeshRenderState`): its own mesh stays empty, so the Fabric Renderer API's submit (which copies the render state's mesh into a new immutable one per submit) does nothing, and `BlockModelRenderStateM`'s `submitModel` HEAD inject (priority 1100) submits the cached mesh as the FRAPI one would, with no copy (item frame extraction 1.17 -> 0.42 us, allocation 935 -> 536 MB/s in the entity scene). Models without a key (custom models) keep Fabric's path. Render thread only, max 4096 entries, cleared on resource reload and when toggled. Switch `blockModelMeshCache`. A temporary check compared 1.5M hits against a fresh emission word for word: identical. **Mixin pitfall seen here:** `BlockAndTintGetter` is `net.minecraft.client.renderer.block.BlockAndTintGetter` in 26.3; with the wrong package in the target the `require = 0` injector silently did nothing |
| `mixin/render/feature/RenderTypeFeatureRendererGroupM.java` | `RenderTypeFeatureRenderer$Group.getOrAddDraw` runs on every render type switch within a group (~290 times per frame in the entity scene) and searched all of the group's draws with `List.indexOf` and the deep `PreparedRenderType` record `equals` (~0.95 us per call, groups of up to ~200 draws). A `@WrapOperation` on the `indexOf` looks the type up in a per-group `HashMap` from prepared type to its first index, filled by a `@WrapOperation` on the `List.add` calls (only the one on `drawRenderTypes` counts), so it returns what `indexOf` would (~0.1 us). The switch `hashedDrawLookup` is read once per group, so the map always covers the whole list. **Found here:** vanilla's lookup never matches, because `ScissorState` has no `equals` and `prepare()` copies it into a new instance every call, so every render type switch appends a new draw (0 hits in the entity scene). **Switch `mergeDraws` (on by default, "Merge Entity Draws"):** the map is keyed by `render/feature/DrawKey` instead, a copy of the prepared type with the scissor state compared by value (rectangle only when enabled), so geometry joins the first draw with equal state as the lookup intends. Only groups vanilla may reorder use the lookup at all (`canReorder`, i.e. not `strictlyOrdered`: the translucent phases are strictly ordered and never merge), and only types without connected primitives. Draws with different transform slices still don't merge (consecutive identical transform writes share a slice, others don't; glint and z-offset layering types write different transforms). See section 7, "Tenth session", for the visual and performance tests |
| `mixin/render/vertex/BufferBuilderM.java` (`putBakedQuad`) | Overrides `VertexConsumer.putBakedQuad` (dropped, held and framed items via `ItemFeatureRenderer`) for the block and entity formats: one `reserveVertices(4)`, the same `transformPosition`/`transformNormal` calls as vanilla into a scratch vector (allocated lazily: the mixin's field initializer didn't run for the builders in use, which crashed with a null vector), and the same color, UV, overlay and light per vertex. Other formats, big-endian hosts and switch `directBakedQuads` off call the default method. A temporary check (vanilla's write, then the fast writer into scratch memory, compared byte for byte except the normal's padding byte) passed for 59.5M quads. Items with a Fabric mesh go through Indigo's `ExtendedItemFeatureRenderer` instead and are not affected |
| `mixin/world/{LevelM,BoundTickingBlockEntityM,RebindableTickingBlockEntityWrapperM,ChestBlockEntityM,EnderChestBlockEntityM,ChestLidControllerM,SignBlockEntityM,ShulkerBoxBlockEntityM}.java` + `interfaces/IdleTicker.java` | `Level.tickBlockEntities` skips tickers whose tick is a no-op: exact vanilla chest/trapped/ender chest classes with a closed lid that stays closed, closed shulker boxes (progress and old progress 0; client and server), and sign/hanging sign classes with no editing player. Also skips the world border, block state and validity checks in front of them. 1.6 → 1.0 ms per tick with ~14k block entities |
| `mixin/world/{LevelM,RebindableTickingBlockEntityWrapperM,BlockEntityM,BoundTickingBlockEntityM}.java` + `interfaces/{ParkableTicker,ParkedTickerHolder,BlockEntityTicker,TickerParkingLevel}.java` | **Parked idle tickers** (twelfth session, switch `parkIdleTickers`, "Park Idle Block Entities", needs `skipIdleTicks`). On the client, `LevelM` replaces `tickBlockEntities` with vanilla's loop that compacts the list in place and drops idle tickers: the wrapper (`RebindableTickingBlockEntityWrapper`) is flagged parked and its block entity remembers it. Woken (re-added with `Level.addBlockEntityTicker`) by every event that can end the idle state: `triggerEvent` of chests, ender chests and shulker boxes, `SignBlockEntity.setAllowedPlayerEditor`, and any `rebind` of the wrapper (new block state or block entity; a rebind to vanilla's null ticker on removal or chunk unload just drops it). The level keeps a list of parked tickers (compacted as it grows) so switching parking or idle skipping off unparks all. The benchmark's chest events assert that the lid opens to 1.0 and closes to 0.0 (only possible if the woken ticker runs). **Validated ticker state** (switch `cacheTickerStates`, "Fast Block Entity Ticks"): `BoundTickingBlockEntityM` redirects the per tick `LevelChunk.getBlockState` + `BlockEntityType.isValid` and keeps the first valid state for the bound ticker's lifetime on the client: a successful `LevelChunk.setBlockState` at a block entity's position either removes it or binds a new ticker, and chunk data packets create new block entities (2M cached states compared with the chunk's: no difference). The world border check stays per tick |
| `mixin/world/ArmorStandM.java` + `world/EntityTypeTests.java` | `ArmorStand.pushEntities` queries every entity in its box with a rideable-minecart predicate; the redirect asks for `AbstractMinecart` only (same entities in the same order: class buckets keep insertion order, the armor stand and dragon parts aren't minecarts). No switch (no behaviour change) |
| `mixin/world/EntityM.java` | `Entity.collide` queries the entities around an entity for their collision shapes (`Level.getEntityCollisions`) before it knows whether it moves; with zero movement it returns the movement and never reads them (the step up needs a collision). The redirect skips the query for zero movement (floating dropped items, experience orbs, boats move by zero every tick). Client and server; no switch (same result) |
| `render/entity/ItemBoundingBoxes.java` + `mixin/render/item/{ItemStackRenderStateM,ItemStackRenderStateLayerM}.java` + `interfaces/ItemLayerExtents` | `ItemStackRenderState.getModelBoundingBox` (dropped items, every frame) caches in the render state, which is extracted anew each frame, so it transformed every model point (each distinct quad vertex) per item and frame. A layer's box is kept per points array (`IdentityHashMap`, chained by item transform identity, hand and local transform), the state's box is the union (float min/max like `AABB.Builder`, so identical; 720k boxes checked). Off the render thread, vanilla's way. Cleared on resource reload. No switch (same result) |
| `mixin/render/entity/model/ModelPartM.java` (`translateAndRotate`) | Keeps each part's rotation `Quaternionf` with the raw bits of its three angles and rebuilds it only when they change (render thread only), instead of three sines and three cosines per rotated part per submit. Same quaternion, so the same pose. No switch |
| `render/entity/TransparentFaces.java` + `ModelCulling.textureAlpha` + `ModelPartCubeM` | **Transparent face culling** (fourteenth session, switch `transparentFaceCulling`, "Transparent Face Culling"). Model cube faces whose texels all draw nothing are not written by `ModelPartCubeM`'s direct path: alpha below the pipeline's `ALPHA_CUTOUT` (discarded before any color is applied; 8 bit alpha a counts if a < ceil(255 c - 0.01)), or alpha 0 in a `BlendFunction.TRANSLUCENT` pipeline without depth writes (the fragment's alpha stays 0, so it blends to the destination exactly). Only render types of vanilla's `core/entity` vertex and fragment shaders without `APPLY_TEXTURE_MATRIX`, `GLINT` or `DISSOLVE`, `TextureTransform.DEFAULT_TEXTURING`, only `Sampler0`, and a texture whose class is exactly `SimpleTexture` with one mip level and the resource's size (read once with `TextureContents.load`); never with improved transparency (OIT) or a UV mapping (atlas sprites). A face's UV rectangle is widened by one texel per side and every texel is tested at its repeated and its clamped position (nearest and linear filtering). Per render type a `TextureAlpha` (`IdentityHashMap`, last type cached), revalidated against the texture's `GpuTexture` and the texture manager's entry; per cube the masks of its last 4 texture alphas. Cleared on resource reload and when toggled. Spider and enderman eyes keep only the eye faces; horse markings lose their empty faces. Access widener: `RenderSetup.textures`, `RenderSetup$TextureBinding.location` |
| `render/entity/ModelPoseReplay.java` + `mixin/render/entity/{EntityRenderStateM,model/ModelM,model/PoseAccessor}.java` + `interfaces/{PoseRecordHolder,LayeredModel,ReplayablePart}` | **Layer pose replay** (fourteenth session, switch `reuseLayerPoses`, "Reuse Layer Poses"). Layers that submit a model again with the same render state and pose (villager and zombie villager clothing, spider and enderman eyes, horse markings, collars, armor trims and glint) skip `setupAnim` and the part tree. While the world's features are prepared, every model submit stamps its `EntityRenderState` with a pooled `Record` (two slots per state; frame, state and model identity); a second submit of the same model and state marks the `Model` as layered. From then on the first submit of a layered model of a `net.minecraft.` class records each written part and a copy of its pose (`ModelPartM.compile`), and a later submit of that model and state whose submit pose matches bit for bit (matrix, normal matrix, JOML properties, `trustedNormals`) calls each recorded part's `compile` with the recorded pose (instanced or vanilla path). Exact because vanilla models pose their parts from the state alone (`Model.setupAnim` starts with `resetPose`; checked: every model's `setupAnim` calls it except the copper golem statue's, which isn't an entity state); a temporary check posed 2.8M replayed submits again: 0 mismatches in 23.4M parts |
| `render/entity/LazyVertexBuilder.java` + `ModelFeatureRendererM` | Model submits whose parts become instances get a lazy vertex builder instead of the group's (a render type switch: `RenderType.prepare`, the draw lookup and a new `BufferBuilder`); it fetches the builder (wrapped by the UV mapping) only if a part is written after all, and `ModelPartM.compile` resolves it before writing cubes, so the direct writers see the `BufferBuilder`. Such submits add no empty draws. No switch (same image) |
| `render/entity/EntityRendererCache.java` + `mixin/render/entity/{EntityTypeM,EntityRenderDispatcherM}.java` + `LevelExtractorEntityM.ruthenium$fastCulling` + `interfaces/{EntityRendererHolder,EntityRendererMap}` | **Renderer kept on the entity type and cheaper entity culling** (fifteenth session). `EntityRenderDispatcher.getRenderer(Entity)` and `getRenderer(EntityRenderState)` look non-player renderers up in the dispatcher's `ImmutableMap` (culling, extraction, submission: up to three times per entity and frame); a `@WrapOperation` on that `Map.get` returns the renderer kept on the `EntityType` together with the map it came from (only while the dispatcher holds that map; only for `ImmutableMap`s). Each entry also classifies the renderer's culling: plain if no class below `EntityRenderer` (or `LivingEntityRenderer` for living ones) declares `shouldRender`, `affectedByCulling` or `getBoundingBoxForCulling` (reflection, once per type). A `@WrapOperation` on `isEntityVisible`'s `EntityRenderDispatcher.shouldRender` call answers for plain renderers (switch `fastEntityCulling`, "Fast Entity Culling"; players and mannequins excluded): `entity.shouldRender` (distance), then the interpolated box (`getInterpolatedBoundingBox`, virtual) through the frustum accessor without the inflated box allocations. Living renderers widen the box by another half block for a dragon head; instead of reading the head slot, the box with the widening is tested first (inside the frustum: visible either way; outside: hidden either way) and only when it crosses the edge the box without (visible: visible either way, else vanilla's path). Leash holders and non-finite boxes take vanilla's path. The frustum test is monotone in the box and the widened coordinates are computed in vanilla's `inflate` order. Checked: 16.8M answers equal vanilla's (AI on), 0.3% undecided |
| `render/entity/EntityLight.java` + `mixin/render/entity/{EntityRendererLightM,EntityLightHolderM,LayerLightSectionStorageM}.java` + `interfaces/CachedEntityLight` | **Entity light kept across frames** (fifteenth session, switch `entityLightCache`, "Keep Entity Light"). `EntityRenderer.getSkyLightLevel`/`getBlockLightLevel` read `Level.getBrightness` at the light probe position for every entity and frame; a `@WrapOperation` on those two calls returns the values kept on the entity (`Entity` fields: level, packed position, generation, block and sky light + 1) while level, position and `EntityLight.generation()` are unchanged. Readers see light through each `LayerLightSectionStorage`'s visible map, which only changes when `swapSectionMap` publishes a new copy (writes copy each section into the updating map first); the client's storages (`chunkSource instanceof ClientChunkCache`) bump the generation at `swapSectionMap` HEAD when sections changed. Renderer overrides (fire, glow frames, displays) still run around the kept values. Render thread only. Checked with `bench.lightEvents`: 29.4M kept values equal fresh lookups (2.35M differ with the invalidation disabled) |
| `world/CollidableEntities.java` + `mixin/world/{EntitySectionM,LevelEntityCollisionsM,LevelEntityGetterAdapterAccessor}.java` + `interfaces/CollidableEntityCount` | **Entity collision searches skip sections without collidable entities** (sixteenth session, switch `fastEntityCollisions`, "Fast Entity Collisions"; idea from Lithium's entity class groups, own code). `getEntityCollisions` (every moving entity's `collide`, every `noCollision`: experience orbs twice per tick, interpolated movement, player pose checks) only accepts entities whose class overrides `canBeCollidedWith(Entity)` (boats, shulkers, happy ghasts, modded ones; checked once per class by reflection). Each `EntitySection` counts those (`add`/`remove` injects, always on), and `Level` overrides the `EntityGetter` default method with vanilla's code whose search skips sections with a count of 0, through the section storage's own `forEachAccessibleNonEmptySection` and `EntitySection.getEntities` (same entities, same order), plus vanilla's dragon part loop. Only for a null source or a source using `Entity.canCollideWith` (boats and minecarts override it to collide with pushable entities: vanilla search). Checked: 800k searches on the client and the integrated server with mob AI, 0 differences. Mod compatibility: a mod injecting into `EntityGetter.getEntityCollisions` or `Entity.canBeCollidedWith` (instead of overriding it) is bypassed (Lithium touches the former) |
| `world/AnimateTick.java` + `mixin/world/{ClientLevelAnimateTickM,LegacyRandomSourceAccessor,EnvironmentAttributeSystemAccessor,EnvironmentValueSamplerAccessor}.java` | **Faster animateTick** (sixteenth session, switch `fastAnimateTick`, "Fast Block Animation Tick"). `ClientLevel.doAnimateTick` runs for 1334 random positions per tick. Its `nextInt` calls on the level's `LegacyRandomSource` (exact class) step the seed with `getPlain`/`setPlain` instead of a compare-and-set per call (same sequence; the CAS only detects use from two threads). Its ambient particle lookup (`environmentAttributes().getValue(AMBIENT_PARTICLES, pos)`: a fuzzy-zoomed biome lookup) is answered with the attribute's no-biome value (computed once per `animateTick`) when the biome palettes of the chunk sections holding the zoom's 2x2x2 quart corners (the only biomes it can pick; y clamped as `ChunkAccess.getNoiseBiome` does) contain no biome whose attribute map has an `AMBIENT_PARTICLES` entry (vanilla: four nether biomes), cached per section per call. Only while the attribute's sole positional layer is the biome lambda of `EnvironmentAttributeSystem` (checked through accessors on the sampler's layers); unloaded chunks take vanilla's lookup. Checked: 24M random results and seeds, 3.8M ambient values in the entity and terrain scenes, and the biome vanilla's zoom chose was always in a checked palette |
| `mixin/world/AbstractBoatM.java` | **Boat ticks** (sixteenth session, switch `fastBoatTicks`, "Fast Boat Ticks"). In `getGroundFriction`, full cube collision shapes (`== Shapes.block()`) skip the moved shape and the `joinIsNotEmpty` with the bottom slice: separated by more than 1e-6 on an axis = false (vanilla's own early out uses 1e-7), overlapping by more than 1e-6 on all axes = true, anything else vanilla's join (redirects on `VoxelShape.move` and `Shapes.joinIsNotEmpty` there). The push search in `tick` (`getEntities(this, box, EntitySelector.pushableBy(this))`, which on the client only accepts local players) asks `ClientLevel.getPushableEntities` on the client, as vanilla's own living entity push does. Checked: 200k cube joins and 6.1k pushes equal to vanilla's |
| `world/SupportingBlocks.java` + `mixin/world/{EntitySupportingBlockM,BlockChangeTrackingM,LevelChunkSectionM}.java` + `interfaces/SectionChangeStamp` | **Supporting block kept for resting entities** (sixteenth session, switch `fastSupportingBlocks`, "Keep Supporting Blocks"). `Entity.checkSupportingBlock`'s `findSupportingBlock` (a `BlockCollisions` sweep of a thin slab under the entity: orbs every tick, items every fourth, entities moved by server packets) is answered from the last result on the client while the slab, the entity's position and the chunk sections of the swept region (box +-1 block, at most 8 sections, the same objects) have the same change stamp. Each `LevelChunkSection` has a stamp, bumped by `LevelChunk.setBlockState` on client levels and for every section by `LevelChunk.replaceWithPacketData` (chunk data is read into the existing sections). Results are kept only when every block of the region has a collision shape that is neither dynamic (`Block.hasDynamicShape`) nor context dependent (no class below `BlockBehaviour` declares `getCollisionShape(BlockState, BlockGetter, BlockPos, CollisionContext)`), so the entity's collision context can't change it. A first version with one level-wide stamp hit ~65% (the integrated server sends ~4.5 block updates per second: grass and dirt random ticks); per section 99.5% at rest. The kept state lives in an object created on an entity's first search (`SupportingBlocks.Kept`; 13 fields on every `Entity` made the server scene's armor stand ticks ~0.02 ms slower per tick). Checked: 220k kept answers equal to a fresh search (gravity scene, with and without AI) |
| `world/EnchantingTableBatch.java` + `LevelM`, `RebindableTickingBlockEntityWrapperM`, `BoundTickingBlockEntityM` (`ParkableTicker.ruthenium$tryBatch`, `BlockEntityTicker.ruthenium$getValidState`) | **Enchanting tables ticked in one loop** (seventeenth session, switch `batchTableTicks`, "Fast Enchanting Table Ticks"; needs parking and `cacheTickerStates`). On the client, `LevelM`'s loop parks the wrapper of a vanilla `EnchantingTableBlockEntity` whose bound ticker has a validated state of the exact `EnchantingTableBlock` class into the level's batch (arrays of wrappers, tables and packed positions, in joining order). After the list, the batch runs vanilla's guards (removed: the wrapper leaves; `shouldTickBlocksAt`, `hasLevel`, the world border) and, unless a player passing `EntitySelector.NO_SPECTATORS` (positions read once per tick) is within the 3 blocks `bookAnimationTick` searches, a statement for statement copy of its branch without a player; otherwise vanilla's `bookAnimationTick`. A wrapper leaves on rebind or unpark (`remove(slot)`; slots move as the batch compacts). Order among tables only matters for their shared unseeded random source, used only with a player near. Mod compatibility: mixins into `bookAnimationTick` don't run for tables without a player near |
| `SectionFaceGroups.sortByPlanes`/`visiblePlaneRuns` (used by `RetainedBlockEntities`) | Retained batches are sorted by facing and, within a facing group, by plane (nearest to the faced side first; stable, so equal plane quads keep their order); each frame draws the prefix of each group whose planes (with the 0.25 margin) the eye can be in front of, merged into at most 7 runs. Quads within a 1e-5 tilt of an axis and 1e-4 blocks of a plane count as in it (rotated models), with their backmost coordinate as the plane. Terrain keeps `sort` and its packed per-group draws |
| `RetainedBlockEntities` (seventeenth session) | Also retains lecterns, bells (not while `shaking`; `BellBlockEntityM` marks the section dirty on `triggerEvent`) and copper golem statues. **Change checks** (switch `retainedChangeChecks`): `LevelExtractor.setSectionDirty` now calls `markBlocksChanged`; a batch remembers its block entity objects and block states and its members' light (`LightCoordsUtil.getLightCoords` at the block entity and, for double chests, at the other half), and a block or light change, or a new compile of the section, rebuilds it only if any of these differ (`isUnchanged`). Neighbours are only checked when they hold a double chest whose other half is in another section (`linked`). Block entity events still force rebuilds (`markDirty`, with neighbours). Batches are freed after 1200 frames *and* 30 s unused, or 1200 frames when more than 64 blocks beyond their view distance. Mod compatibility: a mod changing what a retained block entity renders without a block entity event, block state or light change (e.g. a field set directly on the client) leaves the batch stale until the next rebuild |
| `InstancedModels.tryAddClosedBook` | The closed book's constant 80 degree tilt: sine and cosine computed once with JOML's functions and applied with `rotateTowardsXY`, which `rotateZ` calls on a matrix that isn't a pure translation (pure translations keep `rotateZ`); bit-identical |
| `render/text/EntityTextCache.java` + `mixin/render/text/{EntityRendererNameTagM,EntityNameTagHolderM,MutableComponentM,SubmitNodeCollectionNameTagM}.java` + `interfaces/{KeptNameTags,NameTagLayout}` | **Name tag text kept** (nineteenth session, switch `nameTagTextCache`, "Keep Name Tag Text"). `EntityRenderer.extractNameTags` builds the name tag (`getNameTag` -> `getDisplayName`: team formatting, hover event, insertion) and the below-name line anew every frame; a `@WrapOperation` on both replaces a result that is `equals` (deep) to the component the entity showed last (`Entity` fields) by that kept component. `SubmitNodeCollection.submitNameTag` then measures it (`Font.width`) and orders it (`getVisualOrderText`: ICU bidi and Arabic shaping); the width is kept on the `MutableComponent` (per font, language and `SignTextCache` generation), vanilla's own visual order cache applies to the kept instance, and the visual order is wrapped once in a `SignTextCache` line (`NAME_TAG`), so `TextFeatureRendererM` reuses the prepared glyphs of both submits. Equal components lay out the same; obfuscated text is still laid out every time |
| `render/text/SignTextCache.java` (nineteenth session) | Lines keep up to 4 prepared variants (kind, x, y, colour, background, includeEmpty) instead of one per kind; lines know their source (`SIGN` always on, `NAME_TAG`, `TEXT_DISPLAY` follow their switches) |
| `mixin/render/text/TextDisplayRendererM.java` | **Text display lines kept** (nineteenth session, switch `textDisplayTextCache`, "Keep Text Display Text"): `TextDisplayRenderer.splitLines` (called only when the display's text or line width changes, `TextDisplay.cacheDisplay`) wraps each line in a `SignTextCache` line (`@ModifyArg` on `CachedLine.<init>`) |
| `render/entity/TextDisplayCulling.java` + `mixin/render/entity/TextDisplayAccessor.java` + `LevelExtractorEntityM.ruthenium$fastCulling` | **Text displays culled by their text** (nineteenth session, switch `textDisplayCulling`, "Cull Text Displays"). Vanilla culls displays only with a width and height set (`Display.noCulling`); hologram plugins leave them at 0, so every text display in range was extracted, submitted and its text written every frame. After vanilla's test passes, a `TextDisplay` without passenger offset, name tag, shadow or glow, with a constant transformation and split text (`clientDisplayCache`), is tested as a sphere: the text rectangle (widest line x line count, +1 px background, +8 px glyph margin, 1 px depth) x 1/40, times the transformation (`|t| + ||A||_F |v|`), any billboard rotation; box widened by 0.5 + 2% of the distance (bobbing). Off while the camera entity is hurt/dying or under nausea/portal |
| `mixin/render/entity/LivingEntityRendererLayersM.java` | **Empty armor stand layers skipped** (nineteenth session, switch `emptyArmorStandLayers`): `LivingEntityRenderer.submit`'s `shouldRenderLayers` answers false for vanilla's `ArmorStandRenderer` with exactly its four vanilla layers when the state's equipment, hands, head item and worn head are all empty (the layers draw nothing; the model's own submit poses the model again when prepared) |

Design rules used so far:
- Hot paths use `@Overwrite` with an inlined vanilla fallback, because a cancellable `@Inject` allocates a
  `CallbackInfo` per call, which showed up in allocation profiles.
- Fast paths only engage for the exact vertex format they write (`ExtendedVertexBuilder.of(consumer, format)`).
- Output must match vanilla visually (screenshot diff, section 6.5). Skipping geometry is only done when the GPU
  provably discards it (back faces of culling pipelines, beyond a margin) or it is an exact, idempotent repeat.
- Static fields with initializers in a mixin class broke the target's instance field initialisation once
  (`cellStamps` was null); keep diagnostic statics in a separate plain class.

### Resources
- `ruthenium.mixins.json`: client mixins listed above, `compatibilityLevel` `JAVA_25`.
- `ruthenium.accesswidener`: header `classTweaker v1 official`, no entries yet.
- `fabric.mod.json`: depends on `minecraft ~26.3`, `java >=25`, `fabricloader >=0.19.3`. The
  `fabric-renderer-api-v1:contains_renderer` flag was removed until FRAPI is ported.
- `assets/ruthenium/shaders/*` are leftover 1.21.11 shaders (Vulkan-flavoured GLSL). Nothing uses them yet;
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

### 6.1 The user's protocol
**The benchmark server is gone (2026-10-28, eighteenth session); every scene is a singleplayer world.** The user's
protocol lives on in the default **mixed scene** (`MixedBenchmark`, `-Pbench.scene=mixed`), a stand-in for the old
server scene. Its numbers do not compare with the server scene's (labels `ruthenium-mixed`, `vanilla-vulkan-mixed`, ...),
so the server scene numbers in this file are history.
1. A fresh flat world: 296 armor stands ahead (yaw 0), a cube of 7168 chests with a layer of 1024 signs inside to the
   east (yaw 270), 16 signs on its face, 5625 enchanting tables behind it (closed books), slime and honey blocks;
   render distance 16, spectator at (0.5, -58, 0.5), pitch 0.
2. "initial": the first 5 s after the teleport, while FPS settles (chunks and block entities still arriving).
3. "pan": a full 360°.
4. "static" facing the particle spot, then "particles": `/particle minecraft:happy_villager 1.5 -59 10 2 2 2 1 500000`
   (the user calls it a hellish entity stress test). The run fails if the particle group doesn't fill (peak < 12k; one
   run of the first build peaked at 1625 and wasn't reproduced; it is normally ~16.2k, as on the server).
5. Report average, 95% low and 99% low FPS for each phase, plus allocation rate.

First runs (Vulkan, one each, 2026-10-28): initial 1167, pan 1608, static 1545, particles 1247 fps.
Since the nineteenth session the initial phase also writes its frame series (`results/frames/*-initial.csv`), and
`-Pbench.glowingSigns=true` gives the signs glowing text (inside the cube black and coloured groups, the face red, black,
lime and light blue; `dark_red` is no dye colour and would read as black) and adds yaw 270 shots 7.5 blocks farther
(`glow270x-7`, no face sign within the 16 block outline reach) and 4 closer (`glow270x4`); it changes the scene's numbers.
Default `dark_red` text color and no glow otherwise, so default numbers stay comparable.

### 6.2 Implementation (`src/gametest`)
- `BenchmarkClientGameTest`: sets options (Fancy preset, render distance 16, unlimited fps, no vsync) and runs the
  scene `bench.scene` (mixed, terrain, blockentities, entities, players: section 6.10). `MixedBenchmark` builds its world, then runs
  **initial (5 s) → pan (10 s, driven per frame) → static (5 s, reference at the particle spot) → particles (10 s)**.
  Scenes write JSON and TXT to `build/run/bench/results/<label>-<scene>-<time>.*`, the
  pan's per-frame series (frame ms, yaw, entities, block entities, GCs so far) to
  `results/frames/<label>-<scene>-<time>-pan.csv`, and screenshots to `build/run/bench/<label>/screenshots/`: in the
  mixed scene one at yaw 270 (the chest cube, `bench.extraShotYaws`, taken between pan and static) and one of the
  static view.
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
./gradlew benchRuthenium          # this mod on vanilla's Vulkan backend (label ruthenium)
./gradlew benchVanillaVulkan      # baselines: benchVanillaGl, benchVanillaVulkan, benchSodium (default backend = GL)
scripts/run-matrix.sh 3 benchRuthenium   # N runs each, logs in build/bench-logs, prints BUILD/NTSTATUS per run
python scripts/bench_summary.py build/run/bench/results   # medians per label and phase
```
`./gradlew benchRutheniumGl` runs this mod on vanilla's OpenGL backend (label `ruthenium-gl`).
Options: `-PbenchJfr` (JFR profile to `results/<label>.jfr`), `-PbenchWindow=WxH` (default 1920x1080),
`-PbenchJavaVendor=microsoft -Porg.gradle.java.installations.paths=<jdk>` (run on another JDK), and
`-Pbench.<name>=value` → `-Druthenium.bench.<name>` (`renderDistance`, `preset`, `panSeconds`, `initialSeconds`,
`staticSeconds`, `particleSeconds`, `scene`, `particleCommand`, `extraShotYaws`, `staticYaw`, `sectionProfile`,
`chestEvents`, `zoomShot`, `settingsScreen`), and `-Pvm.<name>=value` → `-Druthenium.<name>`, which overrides that
`Config` switch (`retainedChests=false`, `instancedModels=false`, `blockEntityCulling=false`, ...).
`-Pbench.settingsScreen=true` skips the benchmark: from the title screen it opens the settings screen through the
options screen's video button, screenshots each page (`settings-*`), toggles and applies a switch (checking
`Config` and the written file), hovers an option for its tooltip, searches with Ctrl+L and opens vanilla's screen
with Shift+P (`SettingsScreenCheck`), ~20 s. `zoomShot` adds a 30° FOV screenshot at yaw 270 (`zoom270`), where the enchanting table
books are large enough to compare.
`chestEvents` opens the lid of a chest in the crosshair and adds breaking progress to another one (client-side)
before the static phase, with `chestopen`/`chestclosed` screenshots; compare runs with `-Pvm.retainedChests=false`
to check the retained geometry's dynamic paths.
`-Pbench.staticYaw=270` runs static and particles facing the chest cube, for profiling the slow pan direction.
`-Pbench.label` does **not** change the label (the task sets it), so move such runs out of `results/` (e.g. to
`results/heavy270/`) before summarizing. `python scripts/frames_by_yaw.py <csv> [bin°]` buckets the pan by yaw
and lists frames much slower than their neighbours.

**Workflow per change:** archive the previous `ruthenium-*` results and frame CSVs
(`mkdir -p results/ruthenium-vN/frames && mv ruthenium-2026* ruthenium-vN/ && mv frames/ruthenium-* ruthenium-vN/frames/`),
run 3×, summarize, diff the static and yaw 270 screenshots, then commit with the numbers. Archives so far go up to
`vulkanmod-v18` (archives from before the rename keep the `vulkanmod-` prefix); the reference screenshots for parity are the v4 ones from `20261026-201821`. Runs with
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
- The old server scene was **CPU-bound** for vanilla: 296 armor stands, a wall of chests, slime/honey blocks, signs,
  and a building full of chests on the left (~14k block entities extracted per frame by vanilla). The mixed scene
  copies that mix (~13.8k block entities). The machine has an RTX 4060 Laptop
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

**Seventeenth session tools and traps:**
- Elimination runs through temporary system properties ranked the server scene's costs (retained draws skipped or doubled,
  enchanting table batch skipped); temporary counters in `RetainedBlockEntities.draw` counted batches, draws and quads (sign
  text separately). Patches and results are under `results/s17-*` and `results/s17-dev/`.
- **The parts table's "per tick" figures include calls made outside the tick**: `Level.getEntities(except, box, predicate)`
  read 53 calls per tick at ~4 us, but a stack walk in the probe showed 49.5 of them were `Minecraft.pick` in
  `renderFrame` (one per frame at ~1000 fps / 20 ticks). Check the caller before attributing a "per tick" part to the tick.
- Experiment runs that skip drawing something show in the game window (the user saw the chests vanish during a retained
  elimination run); say so in the notes when such runs are planned.
- Pan frame spikes by cause: the pan CSV has `gcs` and `ticks` columns; frames over a threshold that are neither GC nor tick
  frames isolate other stalls (here retained rebuilds after eviction). A temporary log of rebuilt sections per frame (time,
  block entities, dirty or new mesh) found the extras scene's rebuild storm.
- Tick changes below ~0.04 ms (the sprite pipeline repeats, ~0.03 ms in the probe) don't resolve in a 5 pair tick A/B on
  the server scene; its static tick spreads ~±0.05 ms run to run.

**Sixteenth session tools:**
- The entity cost probe's parts table (`-Pbench.entityCostParts=true`) has a per tick table, `parts of the client's entity
  tick`: base ticks, `move`, `recordMovement`, fluid interaction, `applyEffectsFromBlocks`, pushes, `Level.getEntities`
  searches by box (extra = results), boat status, experience orb collision checks, interpolated zero moves,
  `refreshDirtyAttributes` (extra = dirty attributes), attribute modifications on the render thread, the local player's
  `aiStep` and ambient sound handlers, and each texture atlas's animation tick (extra = sprites drawn). The wrappers add
  ~50-100 ns per call (items 670 -> 1000-1100 ns), so compare parts only between probe runs.
- `python scripts/tick_ab.py build/run/bench/results/<dir> [phase ...]`: per run of an A/B directory the tick probe's ms
  per tick and entities, the frames-with-a-tick split (avg and p99 of tick frames, avg of the others) and p95/p99 low,
  with medians. `run-ab.sh` and `run-ab-commits.sh` now copy each run's game log into the side's directory for it. Tick
  changes show here when fps can't: the static tick median moves by ~0.01-0.02 ms between identical sides.
- The light profiler splits each phase by frame kind (`Split by frame kind`: ms per frame of the frames with a client
  tick and of those without, per section). Two findings (both inferred, not proven): tick frames spend 0.11-0.2 ms in
  `scheduledExecutables`, most likely the Fabric client gametest's per tick handshake with the test thread (JFR shows
  its `postRunTasksHook` there; benchmark harness, not real play), and frames with a tick are also longer outside the
  tick (+0.45 ms static, uniformly ~12-15% in every section; +0.95 ms pan), likely because slow frames are more likely
  to contain a tick (length-biased sampling; in the pan the tick frames are mostly those facing the mobs) plus cache
  effects of the tick.
- `JfrTree.java --min PERCENT`: the tree's threshold (default 0.2% of the window).
- `-Pbench.entityGravity=true` (entity scene): dropped items and experience orbs fall and rest on the floor as in real
  play (by default they float with `NoGravity`, which skips most of their collision work). With `-Pbench.entityAi=true`
  the scene accepts 1200 entities instead of 1300 (mobs fight while the chunks arrive; it failed the old threshold).
- **Once-per-tick code is still warming up in a ~60 s benchmark**: in a 240 s static phase the local player's tick fell
  from ~107 to ~63-87 us and the parts probe's pan tick from 1.90 to 1.77 ms. Per entity code runs ~26k times per second
  and is compiled early; the player's tick, the ambient sound handlers and similar code run 20 times per second.
- JFR doesn't see native time: the texture atlas animation was 0.06% in JFR but 0.18-0.2 ms per tick in the light
  profiler (render passes in the Vulkan driver).

**Fifteenth session tools:**
- The entity cost probe has a table of parts nested in extraction and submission (`parts of extraction and
  submission`): culling, light, name tags and `shouldShowName`, shadows, item model resolves per display context, living
  render types, `RenderTypes.entityCutout`, the villager, armor and held item layers, name display submits, and
  attribute instances created on the render thread. Its mixins apply only with `-Pbench.entityCostParts=true` as well,
  so plain `entityCost` runs stay comparable with earlier sessions. **Their wrappers box arguments** (MixinExtras
  `Operation.call(Object...)`), adding ~50-100 ns per call, and they slow the pan from ~1085 to ~830 fps: compare parts
  only between probe runs with the same mixins, as per call before/after numbers.
- `-Pbench.staticView=k` (entity scene): the static phase at close-up position k (1 faces the villagers).
- `-Pbench.lightEvents=true` (entity scene): after the pan, glowstone above the mob rows is placed and removed eight
  times (light changes while entities are drawn; for checking kept light).
- **JFR misattributed twice more:** 1.25% of the static frame under `AttributeSupplier.createInstance`, which a counter
  showed is never called on the render thread; and 2.75% under `LivingEntityRenderer.extractNameTags` (1.47% self in
  `Object2ObjectOpenHashMap.get`), but skipping those attribute reads changed nothing in an A/B. Light (2.9% in JFR)
  measured 3.2% in the A/B. Rank with JFR, decide with A/Bs.
- A foreground Bash command running longer than ~3 minutes is moved to the background; wait with a loop of at most
  ~170 s per call, or with the background task's notification.

**Fourteenth session tools:**
- `java scripts/JfrTree.java <file.jfr> main [--from HH:MM:SS] [--to HH:MM:SS] [--under Class.method] [--depth N]`: the JFR
  execution samples of a thread through the JDK's consumer API (under a second, where `jfr_tree.py` on a 1 ms profile's
  JSON takes minutes): inclusive and top-frame shares, and with `--under` the callee tree below that method. Inlining
  still misattributes: `InstancedParts$PartBatch.<init>` showed 1.5% "self" under `findBatch` while a batch count
  logged every 1000 frames stayed at 890 (no batch was created).
- `python scripts/ab_table.py build/run/bench/results/<dir> [phase ...]`: each run of an A/B directory (fps, p99 low,
  render thread CPU ms per frame) in run order, with medians, so pairs can be read off directly.
- `scripts/run-ab-commits.sh` now resolves its commit arguments to hashes first: `HEAD` used to follow the checkouts,
  so both sides ran commit A.
- **A/B resolution:** with 5 pairs and `-Pbench.staticSeconds=10`, entity scene static medians move by ~1.5% between
  identical sides; gains below ~2% (e.g. 60 us per frame that the cost probe measured exactly) don't show. The machine's
  level also drifts within a matrix (a side's runs ranged 296-315 fps), so read pairs, not only medians.
- **A background Bash running `run-ab*.sh`, stopped with TaskStop, leaves the `sh` loop running:** it went on
  checking out commits and starting runs alongside the next matrix (session lock errors, mixed builds). Find it with
  `Get-CimInstance Win32_Process | ? CommandLine -match 'run-ab'` and kill the `sh.exe` by PID.
- `git reset --hard` to bin a WIP commit also discards uncommitted script edits; reset `--soft` and restore `src`.

**Thirteenth session tools:**
- `-Pbench.entityCost=true` (`EntityCostProbe`, mixins in `gametest/probemixin`, a separate mixin config applied only
  with the property, so other runs pay nothing): per phase, wall time per entity type of extraction, submission and
  the client tick (per tick), per feature renderer class of group prepares, per model class and pipeline of model
  prepares (with `setupAnim` and the part tree apart) and per display context of item prepares. One nanoTime pair per
  call (~2% fps in the entity scene); its rankings matched the A/B results this session.
- `-PbenchJfrPeriod=1ms` with `-PbenchJfr`: JFR execution samples every millisecond instead of the profile setting's
  10-20 ms. With `-Pbench.staticSeconds=20` the entity scene's client tick got ~300 samples. Parsing such a JSON takes
  minutes; run `jfr_tree.py` in the background.
- `scripts/run-ab.sh <pairs> <dir> "<common args>" "<A args>" "<B args>"`: interleaved ABBA pairs, results filed into
  `results/<dir>/{a,b}` (move other `ruthenium-*` results out of `results/` first; it takes all of them).
  `scripts/run-ab-commits.sh <pairs> <dir> <commitA> <commitB> <gradle args>` does the same for two commits checked
  out in turn in the main tree (copy it out first if commit A predates it).
- A/B a change without a switch through a temporary `static final` system property in a plain class (e.g.
  `-Pjvm.ruthenium.probe.x=true` on one side), removed before the commit.

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
  over the static phase into `build/run/bench/ruthenium/debug/profiling/*.zip`; `client/profiling.txt` has
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
coloured, bold, underlined and back side text, one glowing sign at x 4, black, so always outlined; retained since the
nineteenth session); with the
events it also changes one sign's text on the client (`besign`). It hides part of the field, so don't compare its fps
with runs without it. Crop the signs with `scripts/Crop.java` (`java scripts/Crop.java out.png x y w h
scale in1.png in2.png`, around x 740 y 600) to compare glyphs.

Since the seventeenth session: `-Pbench.blockEntityKinds=extras` fills the field with 3072 lecterns with books, 768 bells on
stone and 1536 copper golem statues (poses, oxidation levels, facings) instead; the chest row stays and the run needs 5000
block entities. Its events ring a bell and crack a lectern (`beevents`, while the bell swings) and reset (`bereset`, after
the swing). Both kinds' events now end with glowstone placed above the field and removed (`belight`, `belightreset`): a light
change in sections whose block entities don't change. The flat world's grass under the field turns to dirt by random
ticks for minutes, i.e. block changes next to the block entities (real play has these too: crops, redstone, water).

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
Mobs have `NoAI` unless `-Pbench.entityAi=true`. Since the thirteenth session the armored zombies wear trimmed
chestplates (gold bolt and diamond sentry in turn; no vanilla trim pattern is a decal) and every other one enchanted
leggings (glint, which animates even with `tick freeze`, so frozen shots of it differ by a few hundred pixels at
threshold 1). Phases: static and pan; results labelled `<label>-entities`.
Screenshots of two runs differ by ~1% even with the same settings (dropped items spin and bob, chickens flap): compare
on/off pairs against a same-setting pair. Results in `results/entities-dev/`.
`-Pbench.closeShots=N` adds eight close-ups after the benchmark (seven of the mob rows, `close7` of the dropped
items and experience orbs), with `tick freeze`: N screenshots per
position (`close<k>-<n>`), pixel-identical within a run when rendering is deterministic (checked: 0 differing pixels
at threshold 1). `-Pbench.closeToggle=<Config field>` flips that boolean switch for shots 1, 3, ... (by reflection, so
vanilla runs work without it), comparing both settings on the same frozen frame. Use short phases for these runs
(`-Pbench.staticSeconds=2 -Pbench.panSeconds=2`) and move their results out of `results/`. `-Pjvm.X=v` passes `-DX=v`
to the game; Mojang's debug flags work in release builds with `-Pjvm.MC_DEBUG_ENABLED=true` plus
`-Pjvm.MC_DEBUG_<FLAG>=true` (`MC_DEBUG_PRINT_PROPERTIES=true` lists them), e.g. `SHUFFLE_MODELS`, which shuffles the
submit order of the reorderable feature phases every frame.
Since the fifteenth session: `-Pbench.staticView=k` runs the static phase at close-up position k (1 faces the
villagers; the pan stays at the overview), and `-Pbench.lightEvents=true` places and removes glowstone above the mob
rows eight times after the pan (light changes while entities are drawn; not measured).
Since the sixteenth session: `-Pbench.entityGravity=true` lets the dropped items and experience orbs fall and rest on the
floor (real play) instead of floating with `NoGravity`; with `-Pbench.entityAi=true` the scene accepts 1200 entities
(fights during setup).

### 6.10 Players and holograms benchmark (nineteenth session)
`-Pbench.scene=players` on any bench task (`PlayerBenchmark`): a server lobby in a fresh flat world, render distance 12
(`playersRenderDistance`), clouds off, time 6000. The camera is a **flying creative player** at (0.5, -58, -7.5), yaw 0,
pitch 8 (a spectator would see the holograms' invisible armor stands as ghosts), so the first-person arm and the hotbar
are in every frame. `bench.playersKind`: `lobby` (default, both of the following), `players` or `holograms`.
- **Players** (`bench.playerCount`, 150; 15 per row, 1.5 x 2 blocks apart, facing the camera): `minecraft:mannequin`s. The
  client extracts a `ClientMannequin` with an `AvatarRenderer` from `mannequinRenderers` and submits its
  `AvatarRenderState` through `EntityRenderDispatcher.getRenderer(AvatarRenderState)`, i.e. the **same player renderers
  and layers as remote players** (`AvatarRenderer`: body with outer layers in `entity_translucent`, armor and trims,
  held items, arrows, Deadmau5 ears, cape, head items, elytra, shoulder parrots, spin attack, bee stingers; name tag and
  below-name line). Wide and slim models alternate over the 9 default skins, registered by the harness as
  `DynamicTexture`s (processed like a downloaded 64x64 skin: base layer opaque), which is how real players' skins are
  held (`bench.playerSkins=resource` uses the skin resources, `SimpleTexture`s). Every third player has a cape (a
  texture in the gametest resources), every seventh elytra, 3 of 5 armor (iron with trims, diamond with trims and
  enchanted leggings (glint), netherite helmet and boots), most hold items (swords, bows, tridents, blocks, ...), shields
  or torches in the off hand, some hide the hat/jacket or sleeves, every 15th crouches. Name tags with team prefixes and
  colours (`[VIP] `, `[MVP+] `, `[ADMIN] `), every other player a below-name line (the mannequin's description, e.g. `19 ❤`,
  standing in for a below-name score). Arrows (every 4th) and bee stingers (every 9th) are set on the server before each
  phase (they wear off). **Shoulder parrots**: `ClientMannequin` never has any, so the gametest mixin `ClientMannequinMixin`
  overwrites `getParrotVariantOnShoulder` to return variants the scene sets on the client entity (`BenchParrots`; 40 of
  150 players).
  **How representative:** the render path is the remote player's. Differences: mannequins stand still (no walk
  animation, no position interpolation; static is the best case for anything kept across frames), they tick as
  `ClientMannequin` (a living entity: `LivingEntity.tick`, no `Player.tick`), their name is a custom name (the display name
  is built by `Entity.getDisplayName`, a player's by `Player.getDisplayName`, which adds a click event), and the culling
  and renderer lookups differ only in which map holds the renderer. Client-side `RemotePlayer`s in the client level were
  not used.
- **Holograms** (`bench.hologramCount`, 120, `bench.hologramLines`, 4; 9 per row, 3 blocks apart, the lowest line 3 blocks
  above the floor, 0.25 between lines): in turn (`bench.hologramKind=mixed`) a stack of invisible marker armor stands with
  visible custom names (one per line, the classic plugin hologram), a stack of text displays (one per line) and one text
  display holding all lines; all billboard `center` with the default background, coloured and bold lines. `stand`,
  `text`, `multiline` use one kind for all. 480 lines; in the lobby 160 armor stands and 200 text displays.
- Phases: static (facing the crowd) and a 360 degree pan. Screenshot: the static view; `bench.closeShots=N` and
  `bench.closeToggle=<Config field>` take frozen close-ups of the first rows of players (`close0`) and holograms
  (`close1`) as in the entity scene. Results are labelled `<label>-players-<kind>`. The enchanted leggings' glint and the
  first-person arm differ between frozen shots of one frame (a few hundred to ~10k pixels at threshold 1): compare text
  and models, not pixel counts.
- The layer probe (`-Pbench.entityCostParts=true`) also times cape, elytra, parrot and stuck-in-body layers.

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
  vanilla's Vulkan backend; on its OpenGL backend (`benchRutheniumGl`, pipelines compile there too) ≤9 at
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

### Thirteenth session (2026-10-27)
Entity scene, the hot spots of section 8 item 1. Measured first with the entity cost probe (section 6.6), static view
(~6.1 ms render thread): model prepares 1.10 ms (part tree 0.83, `setupAnim` 0.12; armor and saddles on vanilla's
path), item prepares 0.86 (~1.7 us per item), extraction 0.92 (item frames ~1.2 us each), submission 0.75 (dropped
items 1.3 us each), Indigo block model prepares 0.34, client tick 1.74 ms per tick (dropped items 2.1 us, experience
orbs 5.7 us, boats ~9 us each). Block displays extracted in 1.5 us and minecarts in 1.8 us, ~45 us together.

Changes, each A/B tested in three interleaved pairs (median; `results/entities-dev/<dir>`):
- **Cached block model meshes submitted without copies** (`c579950`, `meshref`): static 165.4 -> 182.7 fps (p99 94.5
  -> 108.8, render thread 5.77 -> 5.35 ms, alloc 935 -> 536 MB/s), pan 762.7 -> 816.8. Item frame extraction 1.17 ->
  0.42 us, block displays 1.5 -> 0.67 us. Close-ups with the cache switch flipped: 0 differing pixels.
- **Instanced items** (`2552604`, `items`): static 181.9 -> 227.5 (p99 105.4 -> 128.3, render thread 5.31 -> 4.24 ms),
  pan 828.9 -> 947.1. Item prepares 0.86 -> 0.04 ms. 0 differing pixels at threshold 1 on Vulkan and OpenGL; a
  deliberately darkened shader changed 4.5-7.9%, so the flip between shots takes effect.
- **Instanced entity block models** (`aa8b15a`, `blockmeshes`): static 233.0 -> 252.0 (p99 132.8 -> 137.6, 4.20 ->
  3.83 ms), pan 947.6 -> 999.5. Indigo prepares 0.31 -> 0.02 ms. The first version took only `item_cutout`; item
  frames' frames come with `entity_solid_z_offset_forward`, which a temporary counter showed (all 256 rejected).
  0 differing pixels on both backends.
- **Instanced armor** (`1a88390`, `armor`): static 247.8 -> 256.5 (3.93 -> 3.78 ms), pan within noise. Zombie armor
  725 -> 436 ns per model, saddles 3.7 -> 1.1 us.
- **Zero moves skip the entity collision query** (`f34c4f3`, `collide`): JFR at 1 ms showed `getEntityCollisions` at
  ~38% of the entity ticks. Tick 2.015 -> 1.576 ms (entities 1.678 -> 1.244), frames with a tick 6.08 -> 5.51 ms,
  static p95/p99 172.4/131.4 -> 184.7/138.7.
- **Item bounding boxes kept** (`c5ebec2`, `bbox`): static 258.5 -> 270.5 (p99 130.4 -> 141.4, 3.78 -> 3.61 ms).
- **Part rotations kept** (`9ef370e`, `rotcache`): static 262.0 -> 265.0 (3.76 -> 3.66 ms), small but every pair >=.
- **Rejected: reusing part poses for layers** (`results/rejected-posereuse`). Villager clothing layers submit the same
  model with the same render state and pose 3 more times, each posing the whole model again. Recording each instanced
  submit's part poses (per render state) and replaying them for later submits cut villager prepares ~990 -> ~770 ns,
  but the recording (a map lookup, a pose copy and two list adds per part) made every other model ~150 ns slower:
  static 262.2 -> 254.1, and 260.5 -> 256.2 with the copies made cheaper. Shields share one render state across item
  frames, which chained ~25 records per state until chains were capped. Only worth it if recording can be limited to
  models that have such layers.

Session start (`a352e20`) vs end (`9ef370e`), checked out in turn in the same tree, two pairs
(`results/entities-dev/s13-session`; the start's scene has no armor trims): static 165.6 -> 268.5 fps (p95 122.4 ->
190.0, p99 98.3 -> 148.3, render thread 5.77 -> 3.58 ms), pan 762.5 -> 1015.0 (p99 151.8 -> 241.4), client tick
2.10 -> 1.55 ms. Server scene, four pairs (`results/s13-server`): static 1058.2 vs 1058.8, pan 997.3 vs 993.1,
particles 892.4 vs 896.3, initial 734.9 vs 720.5 (run ranges 703-793 vs 686-738; that phase is noisy).

### Fourteenth session (2026-10-28)
Entity scene, section 8 item 1, "model prepares". Measured first, static view (~3.4-3.9 ms render thread): entity
cost probe (`build/bench-logs/623-s14-probe0.log`) model prepares 1.07 ms per frame, part tree 0.68, `setupAnim` 0.12;
villager and zombie villager submits ~1.0 us (four per villager: body and three clothing layers, the same model and
render state), spider eyes 2.5 us, enderman eyes 1.9, horse markings 3.2 (these three on vanilla's vertex path).
JFR at 1 ms (`results/entities-dev/s14/jfr0.jfr`, `scripts/JfrTree.java`): `prepareModel` 30% of the render thread;
`ModelPart.compile` 11% (`findBatch` 3.2, `addInstance` 1.8, vanilla `Cube.compile` 5.4), `translateAndRotate` 5.7
(`Matrix4f.rotate` 4.5), `getVertexBuilder` 3.2, `pushPose` 2.3. Every number below was cross-checked with a second
method (A/B fps and render thread CPU, the probe, the light profiler); where they disagree it says so.

Changes, each A/B tested in five interleaved pairs with `-Pbench.staticSeconds=10` (median;
`results/entities-dev/<dir>`):
- **Transparent face culling** (`7e76c79`, `s14-transparent`, switch on vs off in the same build): static 280.3 ->
  297.6 fps (render thread 3.45 -> 3.22 ms; every on run above every off run), pan 1071.5 -> 1091.3. Probe: spider eyes
  2507 -> 1229 ns, enderman eyes 1927 -> 1159, horse markings 3210 -> 2479 (~84 us per frame); light profiler, one
  pair: world render 2.19 -> 2.11 ms, frame 3.42 -> 3.29 (the A/B's 0.23 ms is larger than both). Frozen close-ups
  with the switch flipped differ from same-setting pairs only by the animated glint, on Vulkan and OpenGL.
- **Layer pose replay with array batch lookup** (`3ad8542`, `s14-replaytable`, commits): static 298.1 -> 311.9 (render
  thread 3.26 -> 3.11 ms; every pair faster), pan 1100.2 -> 1093.9 (p99 265.1 -> 278.6). Probe: villager 1010 -> 598
  ns, model prepares 1.004 -> 0.868 ms. Alone, the replay (`s14-replay`, switch) measured 306.1 vs 305.2 fps (render
  thread 3.21 vs 3.16, 3 of 5 pairs) and -0.045 ms of world render in two light profiler pairs: replayed layers then
  spent most of their time walking each part's chain of ~28 batches (one per villager texture); with the array lookup
  the pair is clearly faster. Temporary check: 2.8M replays posed again, 23.4M parts, 0 mismatches. Close-ups as above.
- **No vertex builder for instanced submits** (`27b4e86`, `s14-lazy2`, commits): static 312.5 -> 324.2 (render thread
  3.08 -> 2.97 ms; every pair faster), pan 1114.6 -> 1124.6. Close-ups with a temporary switch: as above.
- **Rejected: array batch lookup alone** (before the replay; `s14-table`, `s14-table-long`, `s14-table-probe`). The
  probe showed villager prepares 997/968 -> 815/803 ns (-63 us per frame), but fps didn't: 3 pairs 279.1 vs 280.5
  (render thread 3.44 vs 3.40), 4 pairs of 10 s 283.7 vs 283.1 (3.42 vs 3.42). Committed later as part of the replay.
- **Rejected: no vertex builder for instanced submits, before the replay** (`s14-lazy`): 4 pairs 286.6 vs 287.7
  (render thread 3.35 vs 3.37, 1 of 4 pairs faster). The same change after the replay is the third commit above.
- **Rejected: texture alpha looked up only when a cube is written** (`s14-alphalazy`; JFR had `TransparentFaces.of`
  at ~1% of the render thread, render type changes between instanced submits): static 317.1 -> 322.5 (4 of 5 pairs,
  render thread 3.05 -> 2.99) but pan 1127.3 -> 1100.1 (2 of 5). Within noise.

Session start (`4150777`) vs end (`27b4e86`), checked out in turn in the same tree, three pairs. Entity scene
(`results/entities-dev/s14-session`): static 280.4 -> 310.0 fps (p95 198.9 -> 216.1, p99 148.2 -> 151.2, render
thread 3.44 -> 3.16 ms), pan 1052.0 -> 1112.9 (p99 248.7 -> 272.6); client tick unchanged (~1.55 ms); probe at the
end: model prepares 0.82 ms, part tree 0.38, villager 519 ns, spider eyes 726, enderman eyes 663, horse markings
1721. Server scene (`results/s14-server`): static 1079.9 vs 1080.2 (p99 447.2 vs 425.6), pan 1028.5 vs 1034.8,
particles 916.9 vs 911.3, initial 754.3 vs 750.7. Its static render thread CPU read 0.69 vs 0.75 ms there, so two
light profiler pairs followed (`-Pbench.lightProfile=5`): `prepareFeatures` 0.042/0.042 vs 0.041/0.041 ms, world
render 0.283/0.289 vs 0.281/0.281, whole-phase CPU 0.78/0.73 vs 0.75/0.70: no regression (the view waits ~0.18 ms
per frame on the GPU).

### Fifteenth session (2026-10-28)
Entity scene, section 8 item 1, "extraction and submission". Measured first at HEAD (`a98cb80`), static view (render
thread ~3.1-3.5 ms): entity cost probe (`build/bench-logs/812-s15-probe0.log`) extraction 0.570 ms per frame (item frames
~0.4 us, zombies/skeletons/endermen ~0.75-0.8, farm animals ~0.45-0.55), submission 0.496 (zombie villagers 1.25 us,
villagers 0.95, zombies 0.87, items/item frames ~0.3-0.38). JFR at 1 ms (`results/entities-dev/s15/jfr0.jfr`): entity
extraction loop 25% of the static render thread (culling 4.9%, light 2.9%, name tags 2.5%, renderer map lookups
~2%), submission 20% (villager clothing layers 3.1%: three texture identifiers built per submit with string concatenation
and path validation, then hashed afresh by `RenderTypes.entityCutout`'s memo; armor layers 1.7%; pose stack matrix math
~3%). In the pan the culling of all 1332 entities was ~12% of the render thread and the loop ~3.6%. A new probe table
(section 6.6) timed the parts; its numbers include the wrappers' boxing (below).

Changes, each A/B tested in five interleaved pairs with `-Pbench.staticSeconds=10`, switch off vs on in the same build
(median; `results/entities-dev/<dir>`):
- **Entity culling without allocations or renderer lookups** (`c959ffa`, `s15-cull`, switch `fastEntityCulling`, together
  with the renderer kept on the entity type): pan 1141.2 -> 1167.9 fps (every pair faster, p99 281.9 -> 288.3, render
  thread 0.85 -> 0.82 ms), static 327.9 vs 327.0. Probe: `isEntityVisible` 111 -> 92 ns per entity in the pan, 170 -> 143
  at the static view. The first version tested the box without the dragon head widening and then with it: no gain in the
  pan (111 vs 112 ns), as the second frustum test cost what the head slot read had; testing the widened box first decides
  most entities with one test. Check: 16.8M answers vs vanilla's with AI on, 0 mismatches, 0.3% undecided; 34M cached
  renderers equal to the map lookup.
- **Light kept per entity** (`14a6aad`, `s15-light`, switch `entityLightCache`): static 319.4 -> 329.6 (every pair faster,
  render thread 3.02 -> 2.92 ms), pan 1153.3 -> 1169.0 (3 of 5). Probe: `getPackedLightCoords` 137 -> 91 ns (with the
  wrapper); JFR 2.9% -> 1.2%. Check with `bench.lightEvents`: 29.4M kept values equal to fresh lookups; the same run with
  the invalidation disabled found 2.35M mismatches, so the check sees stale light. Close-ups with either switch flipped
  between shots of the same frame differ only like same-setting pairs (glint).
- Tools (`f773c7a`, `d7e9f42`): the probe's parts table, `bench.staticView`, `bench.lightEvents` (section 6.6, 6.9).
- **Rejected: name tag attribute instances kept on the entity.** JFR put 2% of the frame in `LivingEntity.getAttribute`
  (two attribute map lookups per living entity: name tag and below name distance), with `AttributeSupplier.createInstance`
  1.25% under it; a probe counted **0** `createInstance` calls on the render thread, so that attribution was impossible
  (inlined code again). Keeping the instances on the entity: probe `extractNameTags` 285 -> 300 ns (noise). Not
  committed.
- **Rejected: name tags without the attribute reads** (`s15-nametags`, patch `scripts/patches/s15-nametags.patch`): `shouldShowName`
  before the name tag distance and the below name distance only when the scoreboard shows a below name objective (or the
  entity class overrides `belowNameDisplay`); 5M name tags compared with vanilla's, 0 mismatches. JFR had 2.75% of the
  static frame under `extractNameTags`, 1.47% of it as self time in `Object2ObjectOpenHashMap.get`. A/B: static 328.0 vs
  328.2, pan 1180.3 vs 1167.9. So the attribute reads were not the cost JFR showed.
- **Rejected: villager clothing identifiers kept per layer** (`s15-villager`, patch `scripts/patches/s15-villager-ids.patch`):
  `VillagerProfessionLayer.getIdentifier` results kept per kind and registry key. Probe: layer submits 958 -> 425 ns
  (-46 us per frame); JFR 3.1% -> 1.4%. A/B with the static phase at the villager close-up (`bench.staticView=1`, static
  ~430 fps): 428.7 vs 431.6 (3 of 5 pairs), pan 1171.8 vs 1210.2 (4 of 5, but villagers are a small part of the pan).
  Within noise, so binned under the rules; it would matter most in villager-heavy places (trading halls).

Session start (`a98cb80`) vs end (`d7e9f42`), checked out in turn in the same tree. Entity scene (five pairs,
`results/entities-dev/s15-session`): static 321.7 -> 331.5 fps (4 of 5 pairs; p99 158.4 -> 157.7, render thread 3.01 ->
2.95 ms), pan 1131.8 -> 1195.9 (every pair faster, p99 272.3 -> 294.0, render thread 0.85 -> 0.81). Probe at the end
(without parts, `828-s15-probe-end-noparts.log`): extraction 0.570 -> 0.538 ms per frame, submission 0.496 -> 0.509
(unchanged). Server scene (`results/s15-server`): static 1080.9 vs 1086.3 (render thread 0.71 vs 0.72 ms), pan 1023.9
vs 1034.4, particles 915.1 vs 918.3, initial 742.8 vs 787.7 (that phase is noisy: runs 707-779 vs 733-809); four pairs,
no regression.

### Sixteenth session (2026-10-28)
Entity scene, section 8 item 1, "client tick". Measured first at HEAD (`6e1dc85`), static view, tick probe 1.60-1.73 ms per
tick (entities 1.26-1.40), every ~12th frame. Entity cost probe (`build/bench-logs/943-s16-probe0.log`): experience orbs
4.9 us each (315 us per tick), dropped items 0.7 us (178), the player 107 us, boats 8.3-8.8 us each (137), mobs 0.65-1.15
us (~0.5 ms together). JFR at 1 ms (`results/entities-dev/s16/jfr0.jfr`, tick ~3.1% of the render thread): entity searches
by box 0.60% (orb `noCollision` 0.37, the orbs' interpolated zero move 0.22, boat push searches 0.22), `pushEntities` ~0
(26.3's `ClientLevel.getPushableEntities` already tests only the local player), `Entity.commonTick` 0.32% self, the rest
spread over mob methods. The new per tick parts table (section 6.6; wrappers inflate it) confirmed it:
`Level.getEntities` 157 calls per tick at ~1.6 us, orb `noCollision` 2.2 us, boat `getStatus` 3.8 us. Light profiler
(depth 3-4): beyond entities the tick spends 0.19 ms in texture atlas animation and 0.19-0.2 ms in `animateTick`.

Lithium (CaffeineMC/lithium, LGPL-3.0) was read for ideas only, through its mixin option list; no code was copied. Client
relevant options and what became of them: `entity.collisions` / entity class groups (used: collidable section counts),
`entity.fast_retrieval` (built, binned), `entity.collisions.block_effects` (a related idea built, binned), `shapes.*` and
`entity.collisions.movement` (faster collision shapes and sweeps: not attempted, see section 8), `world.inline_*`,
`math.*`, `alloc.*`, `collections.attributes` (single digit nanosecond effects, below the resolution). Most of Lithium
(AI, hoppers, chunk ticking, block entity sleeping on the server) runs on the integrated server's thread.

Changes, each A/B tested in five interleaved pairs with `-Pbench.staticSeconds=10`, switch off vs on in the same build,
read with `scripts/tick_ab.py` (median; `results/entities-dev/<dir>`):
- **Collidable entity sections** (`f4c2517`, `s16-collide`, switch `fastEntityCollisions`; idea from Lithium): client
  tick 1.570 -> 1.410 ms (every pair; entities 1.236 -> 1.069), frames with a tick 4.95 -> 4.69 ms, p95 low 220.9 -> 225.6,
  p99 153.9 -> 159.5; pan tick 1.459 -> 1.304, p99 275.5 -> 287.2. Probe: orbs 4.9 -> 2.1-2.5 us, `getEntities` calls per
  tick 157 -> 28.
- **animateTick** (`f7cc5f9`, `s16-animate`, switch `fastAnimateTick`): tick 1.413 -> 1.341 ms (every pair), frames with a
  tick 4.79 -> 4.64, p95 229.3 -> 232.3, p99 156.9 -> 160.8; pan tick 1.301 -> 1.248. Light profiler pair: `animateTick`
  0.188 -> 0.121 ms per tick. An elimination run first (ambient lookup replaced by an empty list) had put the lookup at
  ~0.045 ms; JFR put the seed's compare-and-set at a third of `animateTick`.
- **Boat ticks** (`b24c07b`, `s16-boats`, switch `fastBoatTicks`): tick 1.328 -> 1.291 ms (every pair; entities 1.053 ->
  1.019), p95 230.6 -> 235.7; pan tick 1.207 -> 1.158. Probe pair: oak boats 8.5 -> 3.4 us, chest boats 8.0 -> 2.4 (16
  boats: 132 -> 46 us per tick; the A/B's smaller tick difference is within its resolution of the probe's).
- **Supporting block kept** (`0337357`, `s16-support`, switch `fastSupportingBlocks`), A/B with `-Pbench.entityGravity=true`
  (the default scene's floating items never land): tick 1.487 -> 1.429 ms (4 of 5 pairs), p95 223.6 -> 227.5; pan tick
  1.352 -> 1.293 (every pair), frames with a tick 3.20 -> 3.02. Probe pair: items 1178 -> 1065 ns, orbs 3402 -> 2787 ns.
  JFR of that scene (`s16/jfr-grav.jfr`): `Entity.move` 0.49% of the render thread, half of it `checkSupportingBlock`, a
  third `collide`'s block sweep.
  The first version kept the state in 13 `Entity` fields; the server scene's entity ticks (armor stands) then read 0.308 ->
  0.327 ms per tick in the session comparison (four pairs, inferred to be the larger objects), so it moved into an
  object created on first use (`272da2e`; probe: items 1047 ns, orbs 2430 ns).
- Tools (`eba9a88`, `2b24b07`, `67b10c3`, `7f4e28c`): section 6.6.
- **Rejected: direct entity section lookups** (Lithium's `entity.fast_retrieval` idea, own code; `s16-sections`, source
  `scripts/patches/s16-sections-EntitySectionStorageM.java.txt`): small boxes looked their sections up by key in vanilla's iteration order (x
  ascending, then z and y as unsigned 22/20 bit values) instead of walking the sorted set of all sections with that x;
  500k+ calls compared with vanilla's walk, 0 differences (181k with several sections, 55k crossing coordinate 0). Tick
  1.407 vs 1.405 ms, pan 1.291 vs 1.293: nothing to gain in this scene (few entity sections per x column); it would
  matter with entities spread over a large area.
- **Rejected: inside-block checks skipped at rest over air** (`s16-inside`, source `scripts/patches/s16-inside-EntityInsideBlocksM.java.txt`, related
  to Lithium's `entity.collisions.block_effects`): `applyEffectsFromBlocks` of entities that didn't move visits the blocks
  of their box and does nothing at air; when all of them were air the visit was skipped (exact for zero travel: vanilla
  visits `BlockPos.betweenClosed` of the box deflated by 1e-5 at the target). Check: 300k skips, 0 with a non-air block;
  a positive control (items resting in short grass, air test disabled) flagged 46k. Tick 1.398 vs 1.391, pan 1.282 vs
  1.259: within noise.
- **Not done (bench artifacts):** the experience orbs' `recordMovement` -> `AbstractInterpolationHandler.adjustInterpolationPosition`
  `noCollision` for moves by zero (~45 us per tick) only happens to `NoGravity` orbs, which real play doesn't have; the
  player's tick (~63-110 us) is mostly warm-up (section 6.6) and its parts (bubble column and biome ambient sound
  handlers 8-24 us each, `aiStep` 20-40) are each below the resolution.

Session start (`6e1dc85`) vs end (`7f4e28c`), checked out in turn in the same tree. Entity scene (five pairs,
`results/entities-dev/s16-session`): client tick 1.596 -> 1.261 ms per tick (entities 1.270 -> 1.003),
frames with a tick 4.95 -> 4.56 ms, static 321.5 -> 331.1 fps (p95 219.3 -> 241.5, p99
155.2 -> 165.8), pan 1159.7 -> 1193.6 (tick 1.465 -> 1.168, p99 282.4 -> 299.9).
Server scene, four pairs against `7f4e28c` (`results/s16-server`): client tick 1.231 -> 1.131 ms (static; pan 1.433 ->
1.317, particles 1.337 -> 1.239: animateTick), fps static 1084.8 vs 1084.5, pan 1029.4 vs 1031.9 (p99 379.5 -> 393.3),
particles 918.8 vs 918.9, initial 756.7 vs 771.2 (noisy). Its entity ticks (~300 armor stands) read 0.308 -> 0.327 ms,
which led to `272da2e`. Against that, three pairs (`results/s16-server-kept`): tick 1.250 -> 1.186 ms, entities 0.307 ->
0.319 (runs 0.298-0.307 vs 0.302-0.340; unresolved, ~40 ns per armor stand), static 1081.4 -> 1085.4 fps (p99 443.9 ->
463.8), pan 1030.7 -> 1035.2 (p99 368.4 -> 383.3), particles 921.2 vs 918.3. No fps regression; the scene is close to
GPU bound. The entity scene was compared against `7f4e28c`; `272da2e` only moved the kept state (probe above).

### Seventeenth session (2026-10-28)
Server scene, section 8 item 2, "per entity and block entity cost". The server was up the whole session. Measured first at
HEAD (`77b631c`; light profiler depth 4 and GPU timestamps, `build/bench-logs/1152-s17-base-prof.log`), static view: frame
0.944 ms, GPU frame 0.861 ms (solid terrain 0.412, retained block entities 0.156, instanced models 0.063, the rest ~0.23),
render thread waiting ~0.17 ms per frame in `awaitGpu`; block entity extraction 0.089 ms, entity extraction 0.063; tick
probe (no profiler) 1.11-1.19 ms per tick: block entities ~0.46 (5.6k enchanting tables), entities ~0.31, textures ~0.16,
animateTick ~0.15. Second methods used below: elimination runs (skipping a part), the entity cost probe's parts table,
temporary counters, GPU timestamps and frame time series.

- **Retained draws by elimination** (`results/s17-dev/exp1`): skipping the retained draws took the static view from 1084
  to 1224 fps (GPU frame 0.862 -> 0.725 ms), drawing them twice to 922 (retained 0.332 ms), so their GPU time is real and
  proportional to their work. Counters: 15 batches, 49 draws, **178.8k quads per frame, 86.8k of them sign text** (the signs
  inside the chest wall). Draw count is not the cost.
- **Enchanting tables by elimination** (`s17-dev/exp1`): with all of them skipped the block entity tick is 0.075-0.082 ms,
  so ~5.6k tables cost ~0.38 ms per tick (~70 ns each) on vanilla's path.

Changes, each A/B tested in five interleaved pairs unless noted (median; `results/<dir>`):
- **Enchanting tables ticked in one loop** (`4c51590`, `s17-tables2`, switch `batchTableTicks`, "Fast Enchanting Table
  Ticks"): `EnchantingTableBatch`. Client tick 1.151 -> 0.929 ms (every pair; block entities ~0.45 -> 0.24), frames with a
  tick 2.32 -> 2.04 ms, static p99 low 459 -> 505 fps (every pair), pan tick 1.409 -> 1.161. A first version that kept the
  wrapper and read the position through the block entity measured 1.156 -> 1.020 ms (`s17-tables`); the scattered
  wrapper and `BlockPos` objects were most of what was left (the batch now costs ~23 ns per table, elimination). Exact:
  3.1M ticks compared with vanilla's `bookAnimationTick` on a copy, 0 differing bits (both versions).
- **Retained quads drawn by plane** (`9c15ac0`, `s17-planes`, temporary switch): `SectionFaceGroups.sortByPlanes` and
  `visiblePlaneRuns`. Quads per frame 178.8k -> 131.6k (text 86.8k -> 65.5k); static 1063.1 -> 1094.8 fps (every pair; GPU
  frame 0.873 -> 0.837 ms, retained 0.156 -> 0.141), pan 999.1 -> 1008.0 (retained GPU 0.103 -> 0.077), particles 897.1 ->
  922.3. Pixel-identical in the block entity scene on Vulkan and OpenGL (only the sky band that differs between runs).
- **Closed books' tilt computed once** (`e02bd9f`, `s17-tilt`, facing the cube with `staticYaw=270`, temporary switch):
  static 832.5 -> 846.8 fps (every B run above every A run; render thread 1.03 -> 0.99 ms), particles at that yaw 684.8 ->
  715.5. 103.8M poses compared with `rotateZ`'s: 0 differences.
- **Lecterns, bells and copper golem statues retained** (`62992f5`, scene `6f73bd8`, `s17-extras`, three pairs): block
  entity scene with `-Pbench.blockEntityKinds=extras` (section 6.8), static 211.1 -> 1219.5 fps (render thread 4.58 -> 0.43
  ms), pan 1479.9 -> 2118.8. Screenshots on vs off: 29 pixels at threshold 1 (14 at 12) on Vulkan and OpenGL, the rounding
  difference the other retained types have.
- **Retained batches rebuilt only when their blocks or light change** (`0f9b780`, `s17-changes`, switch
  `retainedChangeChecks`, "Retained Change Checks"; glowstone events `ca6a80b`): found through a rebuild log in the extras
  scene, whose static p99 low stayed ~100 fps: the flat world's covered grass turning to dirt (random ticks) rebuilt the
  sections around each change, ~10 times a second, up to ~5 ms each. Extras scene: static 1472.9 -> 1625.3 fps, **p99 low
  280.2 -> 715.2** (frames without a tick p99 3.0-5.1 -> 1.18-1.20 ms, every pair), pan 2289.3 -> 2419.4. Exact: 800 kept
  batches rebuilt anyway by a temporary check, 0 CRC differences; screenshots with the switch flipped identical at
  threshold 1 in both block entity scenes, including the new glowstone events.
- **Retained batches kept 30 s behind the camera** (`aa618c3`, `s17-evict`, temporary switch): a 1200 frame eviction freed
  the chest cube's batches during the pan (~1.2 s at ~1000 fps), so turning back rebuilt them (frames of 7-21 ms). Pan
  frames over 7 ms that are neither GC nor tick frames, per run: 2/2/3/2/1 -> 0/0/1/0/1; fps unchanged (pan 1050.5 vs
  1058.4).
- **Rejected: pipeline set once per run of equal sprites in texture animation** (`s17-sprites`, patch `scripts/patches/s17-sprites.patch`). Vanilla's
  `SpriteContents.AnimationState.drawToAtlas` sets its pipeline before every sprite, which binds it again and makes the
  frontend set every uniform again. The parts probe put the blocks atlas at 105/104 -> 129/146 us per tick without/with the
  repeats (2 runs each), but the tick A/B (5 pairs) read static 0.953 vs 0.945 ms, pan 1.179 vs 1.143, not consistent across
  pairs: within noise, binned.
- **Rejected: front-to-back order within facing groups** (`s17-f2b-*`, five pairs each, patch `scripts/patches/s17-f2b.patch`): planes sorted
  farthest out first so the drawn suffix runs nearest to the eye first, with a binary search for the visible part; 0 of
  400k draws differed from a per-quad brute force, but no measurable change (server static 1096.5 vs 1095.3, GPU 0.835
  both; block entity scene 1731.6 vs 1735.1).
- **Rejected: block entity extraction through per cell index masks** (`s17-cells`, patch `scripts/patches/s17-cells.patch`): sections crossing the
  frustum visited only the list indices of cells not outside it (bit sets per mesh, list order kept; 260k sections compared
  with the old loop, 0 differences). Extraction probe: static 92 us per frame for 22 sections, 14k listed, 9.3k retained,
  4.7k visited (253 books, ~4.4k culled); the pan facing the cube 0.2-0.3 ms, mostly books. A/B: static 1102.7 vs 1105.4,
  pan 1030.8 vs 1017.0 (render thread 0.83 vs 0.85 ms): no gain, binned.
- **Measured, not pursued:** armor stand ticks (~0.23 ms per tick for 296 with the parts probe's wrappers, ~0.8 us each,
  spread over `aiStep` ~0.25, base ticks ~0.35, `commonTick` 0.09, fluid checks 0.08: no single part worth a change); the local
  player's tick (0.1-0.25 ms, `LocalPlayer.aiStep`, move, inside block checks and ambient sound handlers; once-per-tick code
  warming up, section 6.6); the parts table's 53 `Level.getEntities` calls "per tick" at ~4 us were 49.5 per-frame
  `Minecraft.pick` calls (a stack walk showed it), ~5 us per frame; glowing sign text (section 8); retained builds after
  joining (single sections of ~450-2300 block entities take 9-43 ms while the code is cold, and a section is built in one go).

Session start (`77b631c`) vs end (`aa618c3`), checked out in turn in the same tree, five pairs:
- **Server scene** (`results/s17-session-server`): static 1085.0 -> 1126.5 fps (every pair; p99 low 446.3 -> 507.2), pan
  1029.2 -> 1055.5 (p99 380.0 -> 436.8), particles 916.2 -> 941.2, initial 753.8 -> 818.8 (p99 126.2 -> 144.6); client tick
  1.236 -> 0.931 ms (pan 1.384 -> 1.133), frames with a tick 2.29 -> 2.02 ms. On vanilla's OpenGL backend (two pairs,
  `s17-session-server-gl`): static 866.2 -> 894.9, pan 844.0 -> 865.5.
- **Block entity scene** (default kinds, `s17-session-be`): static 1773.5 -> 1763.1 fps (**-0.6%, every pair**; render
  thread 0.27 ms on both sides, so GPU), pan 2602.1 vs 2600.7. A bisect (`s17-bisect-planes`, four pairs) put it on the
  plane sorting (`9c15ac0`): 1727 -> 1714 fps, GPU frame 0.542 -> 0.546 ms, retained 0.295 -> 0.299 ms, although fewer
  quads are drawn. Not the draw count (merging runs across gaps of hidden quads, `s17-dev/gap`: no better, worse with
  large gaps) and not front-to-back order within a group (`s17-f2b-*`, five pairs each: BE 1731.6 vs 1735.1, server
  1096.5 vs 1095.3, GPU equal; binned, `scripts/patches/s17-f2b.patch`); grouping planes into 4 block slabs kept in build order (`s17-dev/slab-*`)
  was within single-run noise in this scene and lost part of the server scene's gain (retained 0.138-0.141 exact planes,
  0.142-0.148 slabs, 0.155-0.157 with 16 block slabs, i.e. the old per group culling). Inferred cause: plane order spreads
  each block entity's quads through the buffer (less locality for the GPU). Kept: the server scene gains 15 us of GPU per
  frame (+3%), this scene loses ~4 us (-0.6%). The extras kind isn't comparable across these commits (it didn't exist at
  the start); its numbers are in the change list above.
- **Entity scene** (`s17-session-entities`): static 323.7 vs 327.4 fps, pan 1173.9 vs 1184.6, client tick 1.287 vs 1.285 ms:
  unchanged (none of the changes touch it).

### Nineteenth session (2026-10-28/29)
Two agents worked in parallel from `3c04a06`: part 1 (the open items of section 8) on `26.3`, part 2 (players and
holograms) on branch `worktree-agent-aa744c1ec3f1ed79f` in its own worktree. The branch was merged into `26.3` afterwards
(a `--no-ff` merge; conflicts only in the switch lists and these notes).

#### Part 1: the open items of section 8
The four open items of section 8 (transparent face cache misses, glowing sign text, retained builds after a join, resting
items and orbs). The machine was shared with another agent's benchmarks: every Gradle command ran under a shared GPU lock
(a `mkdir` lock in the session's scratchpad), whole matrices per lock. Results in `results/s19-*` and `results/s19-dev/`, logs
`build/bench-logs/s19-*.log`.

- **Transparent face masks per texture** (`a4d2751`, `s19-tf-*`): confirmed with temporary counters first: with
  `-Pvm.instancedParts=false` the entity scene missed `ModelPartCubeM`'s 4 entry cache 826 times per frame at ~1.06 us (~0.88 ms
  per frame, villager clothing cubes see ~28 textures); with defaults 0 misses, but `TransparentFaces.of` ran for all ~1306
  model submits per frame (~60 us with the timers) although most are instanced and write no cube. Each cube now keeps a hash
  table per `TextureAlpha` (id hashed, grows to 128 slots, starts over when full or when `TransparentFaces.clear` bumps the
  generation), and `ModelCulling` looks the alpha up when the first cube of a submit is written (1306 -> 103 lookups per frame).
  Same masks, so same output. Instanced parts off, culling off vs on (5 pairs): static 196.0 -> 221.6 fps (every pair; render
  thread 4.92 -> 4.37 ms), pan 954.9 -> 997.6. The culling is now a gain there too (it was ~8% slower). Defaults, commit vs
  commit: entity scene 324.7 vs 320.2, then 329.3 vs 337.6 with `staticSeconds=10` (pan 1131 vs 1151): within noise both
  times; mixed scene unchanged.
- **Glowing sign text retained** (`a4651b2`, `s19-glow-mixed`, switch `retainedGlowingText`, "Retained Glowing Sign Text"):
  `isRetainable` accepts outlined text (the 8 outline copies go to retained copies of vanilla's `text` / `text_grayscale`
  pipelines, `TEXT_NORMAL`, `TEXT_GRAYSCALE_NORMAL`; outline and text are separated by the polygon offset, so their order
  doesn't matter). Black glowing text is always outlined, so it is plain retained geometry. Other colours are outlined only
  while `AbstractSignRenderer.isOutlineVisible` (camera entity within 16 blocks of the block centre, or first person
  scoping): such signs are built with `drawOutline = false` into separate parts kept in sign order (`SIGN_PARTS`, quad counts
  per sign through a `BufferBuilder` accessor), and every frame a visible batch whose box is within 17 blocks of the camera
  entity (or while scoping) calls vanilla's `isOutlineVisible` (invoker) per such sign; those it outlines are left out of the
  members returned to the extraction (vanilla path) and of the draws (runs of consecutive signs). No invalidation is needed for
  scoping: the check runs every frame. The mixed scene got `-Pbench.glowingSigns=true` (section 6.1). Parity, retained on vs
  off: zoom shot 0 px at threshold 12; yaw 270 and the shots 7.5 blocks farther (no outlines) and 4 closer (outlined face
  signs on the vanilla path, up to 8 per pan frame) differ only in clouds; the block entity scene with signs and events (its
  black glowing sign now retained) 19 px at threshold 12 in every shot, as before. A first parity run passed for the wrong
  reason: the face signs' `dark_red` is no `DyeColor`, so they read as black (always outlined) and no sign took the vanilla
  path (extraction count 0 in every frame); the option now gives the face valid dye colours. A/B with `glowingSigns` (5
  pairs, switch off vs on): static 773.9 -> 1288.8 fps, pan 853.2 -> 1245.0, initial 498.3 -> 954.4, particles 657.7 ->
  1083.3, every pair (render thread static 1.27 -> 0.60 ms). Default mixed scene: unchanged (static 1514.2 vs 1525.7).
- **Rejected: retained builds spread over frames** (`s19-builds2`, `s19-budget`, patch `scripts/patches/s19-spread-builds.patch`).
  Measured first (temporary build timer, `s19-dev/p3-build`): in the mixed scene's initial phase three consecutive frames
  built 2056, 2048 and 2664 block entities in 95, 38 and 27 ms (frames 4-6 of the phase, 112/49/37 ms; the phase max of
  125 ms is frame 0, which builds nothing); later builds are warm (a 1536 chest section took 10.5 ms in the pan). The patch
  replaces the 2048 block entity per frame budget by a time budget (1 ms, checked after each block entity): one build at a
  time continues across frames with its vertices kept in the part sets, its block entities take the vanilla path until it
  is done, it starts over when its list or a block entity changes, and after block or light changes it is checked with
  `isUnchanged` when done. Exact: builds that spanned frames were rebuilt in one go in the same run and their part CRCs
  compared, 12 of 12 equal (with glowing signs too; CRCs don't compare across runs, glyph atlas positions depend on the
  order glyphs are first baked). A first version that restarted on every light change and when its section missed one
  frame never finished while chunks arrived. Result (5 pairs, initial phase, budget 1 ms vs unlimited in the same build):
  p99 low 302.0 -> 199.5 fps, frames over 10 ms 10 -> 19, over 5 ms ~25 -> ~55, time over 5 ms 457 -> 469 ms, max 138.8 ->
  125.6 (frame 0 dominates); pan and static unchanged. While a section is unbuilt its ~2000 chests take the cold vanilla
  path every frame, so one 30-90 ms frame becomes dozens of 5-20 ms frames. Binned; the initial spikes worth chasing are the
  first frames after the teleport (not builds; not investigated).
- **Resting block collisions kept** (`9f2c06d`, `s19-resting-noai`, switch `fastRestingCollisions`, "Keep Resting Collisions";
  idea from Lithium's collision work, own code): the entity cost probe got `Entity.collide` / `collideBoundingBox` parts per
  kind (items, orbs, other); with gravity and AI, `collideBoundingBox` was ~1.0 us per item move and ~0.7 us per orb move,
  ~164 us per tick of the probe's 2.07 ms. `RestingCollisions` keeps an entity's last `collideBoundingBox` answer (wrapped
  in `Entity.collide`, client levels, only without entity colliders) while the box and movement repeat, the world border is
  not close, and the swept region passes `SupportingBlocks.capture` / `unchanged` (no dynamic or context dependent shape,
  same sections and change stamps); the region is captured only on the second identical call, so moving entities only
  compare fields. Exact: a temporary check computed vanilla's answer on every hit (gravity, AI, explosions, light events):
  131k hits, 0 mismatches. A/B with `-Pbench.entityGravity=true -Pbench.staticSeconds=10` (5 pairs): static client tick
  1.429 -> 1.360 ms (every pair; entities 1.151 -> 1.087), pan tick 1.292 -> 1.246, fps unchanged within noise. With AI too
  (`s19-resting`, secondary) the scene differs run to run (creepers left 3-18, items 419-465, explosions crater the
  floor): static tick 1.821 vs 1.830 ms with runs from 1.69 to 1.91, no conclusion; don't use AI runs for tick A/Bs.
- Tools: the mixed scene writes the initial phase's frame series (`*-initial.csv`) and has `-Pbench.glowingSigns=true`
  (section 6.1); the entity cost probe's tick parts include `collide` and `collideBoundingBox` per kind.

Mod compatibility (each change has a switch; all exact for vanilla): `retainedGlowingText` calls vanilla's
`AbstractSignRenderer.isOutlineVisible` each frame for retained coloured glowing signs near the camera, so a mod changing
the outline rule there is followed, but one that changes it in `submitSignText` or the render state is not (as for other
retained sign text, mods drawing extra sign content through other submits make the sign non-retainable).
`fastRestingCollisions` wraps the `collideBoundingBox` call in `Entity.collide` (a mod overwriting `collide` makes the
mixin fail to apply; one redirecting the same call is wrapped), and has `fastSupportingBlocks`' assumptions: block changes
on the client go through `LevelChunk.setBlockState` or chunk data, and context dependent collision shapes override
`getCollisionShape` in the block class. `TransparentFaces`' per cube tables hold texture alphas until a reload (at most
128 per cube).

#### Part 2: Players and holograms
The question was whether other players and holograms were optimised yet. A new scene (`-Pbench.scene=players`, section
6.10; mannequins on the remote player render path, armor stand and text display holograms) measured it. Its own worktree
and branch (`worktree-agent-aa744c1ec3f1ed79f`), all runs in that tree; another agent benchmarked on the machine at the same
time, serialised by a shared lock. The user saw the screen turn off briefly during one run; no run has a frame over 32 ms,
and the only candidate outlier is noted with its A/B below.

**What was already optimised before this session** (code read, then checked with the probes):
- Players: the body (`PlayerModel`, `entity_translucent`, so the translucent phase, strictly ordered and sorted on upload)
  keeps vanilla's vertex path with `ModelPartCubeM`'s direct cube writes; no back face skip (the pipeline doesn't cull), no
  instancing (strictly ordered groups), no pose replay (no layer submits the body model again). `TransparentFaces` applies
  only to `SimpleTexture` skins (default skin resources), not to downloaded skins (`DynamicTexture`), and with many skins
  it is a loss (below). Everything a player wears or carries is already instanced: armor and trims and elytra
  (`armor_cutout_no_cull`), cape and shield/trident (`entity_solid`), parrots (`entity_cutout`), arrows and bee stingers
  (`entity_cutout_cull`), held items (instanced items); only the armor glint (`armor_cutout_no_cull_glint`) is not. Light is
  kept per entity (`EntityLight`). Culling of avatars stays vanilla's (`EntityRendererCache` skips players and mannequins,
  whose renderer comes from the skin's model). Name tags: nothing (fifteenth session: attribute reads binned).
- Holograms: armor stands are extracted into reused render states (`ReusedRenderStates`) and culled cheaply; their name
  tags and all text displays took vanilla's path except the direct glyph writes (`BakedSheetGlyphM`). Text displays with
  the default size 0 were never frustum culled (vanilla, see below).
- Ticks: the general entity tick work of the sixteenth session applies (typed armor stand pushes, collision sections, ...).

**Baselines** (lobby: 150 players, 160 armor stands and 200 text displays = 480 hologram lines, 510 entities drawn at the
static view; static / pan fps, render thread ms per frame at the static view):

| | static | p99 low | render thread | GPU frame | pan |
|---|---|---|---|---|---|
| vanilla Vulkan (3 runs) | 153.6 | 93.1 | 6.17 ms | 1.50 ms | 613.2 |
| Ruthenium at the session start (text changes off, 1 run) | 263.7 | 151.9 | 3.66 ms | 0.98 ms | 924.2 |
| Ruthenium at the end (15 runs) | 318.6 | 185.9 | 3.04 ms | 0.82 ms | 1435.4 |

Per kind (one run each, static; render thread minus the empty scene's 0.22 ms, divided by the count):

| | vanilla static / pan | Ruthenium static / pan | render thread per unit (vanilla -> Ruthenium) | GPU per unit |
|---|---|---|---|---|
| empty flat world | 2260.9 / 2176.6 | 2409.6 / 2354.6 | - | - |
| 150 players | 210.4 / 1264.3 | 473.2 / 1549.5 | 29.7 -> 12.2 us per player | 5.5 -> 1.5 us |
| 480 lines, mixed kinds | 687.0 / 1015.0 | 1080.2 / 1847.8 | 2.5 -> 1.4 us per line | ~0.04 us |
| 480 armor stand lines | 505.8 / 1511.5 | 937.6 / 1804.8 | 3.5 -> 1.7 us per line | 0.16 -> 0.06 us |
| 480 text display lines | 865.9 / 863.1 | 1291.3 / 1894.1 | 1.8 -> 1.1 us per line | ~0 |
| 120 text displays of 4 lines | 772.2 / 743.2 | 1327.9 / 1892.0 | 2.1 -> 1.0 us per line | ~0 |

The lobby is CPU bound (GPU 0.82 ms of a 3.1 ms frame). Vanilla's pan is no faster than its static view for text
displays: it processes all of them whatever the view direction. Client tick (tick probe, static): lobby 0.63 ms per tick
(entities 0.35, of which the local player ~0.15-0.2), every ~16th frame; per entity ~0.3 us for mannequins and armor
stands, ~0.1 us for text displays (stand holograms: entities 0.39 vs empty 0.20 ms per tick for 480 stands, vanilla 0.67).
Frames with a tick: 4.0 vs 3.1 ms (tick 0.63 ms plus the harness's handshake, section 6.6).

**Where the time goes** (lobby static, all changes on). Light profiler depth 6 (`build/bench-logs/s5-mod-light6.log`), per
frame of 3.2 ms: entity extraction 0.47 ms, entity submission (`submitFeatures`) 0.34, `prepareFeatures` 1.74 (group
prepares 1.14, `uploadSharedVertexBuffer` 0.51, sort 0.05), frame graph execution 0.20, GUI and hand ~0.06, tick 0.07 on
average. Upload probe (new upload table of `-Pbench.entityCost=true`, `s6-upload-*.log`): the lobby uploads ~3.2 MB of
vertices per frame in 3 uploads (world, hand, GUI) of ~154 draws; 147 draws are sorted on upload (the 145 player bodies,
`entity_translucent`, and see-through name tag text), 113 quads each on average: decoding their quad centroids 100 us and
sorting plus writing their indices 273 us of the upload's 507 us per frame (probe timings). With players only: 145 sorted
draws of 72 quads, **1.8 us per player body per frame** (sort and indices 1.4, centroids 0.37). Entity cost probe with
parts (`s5-probe-players.log`, `s5-probe-stand.log`; wrappers add ~50-100 ns per wrapped call):
- **Per player ~12 us** of render thread: culling 0.18 us (vanilla path), extraction 1.6 (name tag 0.55, held item models
  ~0.2, the rest spread over `AvatarRenderer`/`HumanoidMobRenderer` extraction: arm poses, flight and cape state, armor
  copies), submission 2.5 (layers ~1.4: held items 0.40, armor 0.37, arrows 0.22, parrots 0.14, stingers 0.11, elytra and
  cape 0.08 each; name display 0.22), **body prepare 2.8** (72 faces written, a render type switch per player for its own
  skin), armor 1.1 (instanced, ~0.6 per piece-set), glint 0.2, arrows/parrots/cape ~0.8, name tag glyphs 0.9, shadow 0.1,
  plus its share of the upload (the body's 72 quads are sorted every frame, `entity_translucent` sorts on upload) and of the
  draws (one per skin). GPU ~1.5 us. Tick ~0.3 us (mannequin; a remote player's `Player.tick` is not measured).
- **Per armor stand hologram line ~1.7 us**: extraction 0.94 (name tag 0.28, light 0.09, empty hand items 0.09, the reused
  armor stand state ~0.45), submission 0.29 (name display 0.14), culling 0.11, glyphs of its two name tag passes ~0.67.
  Tick ~0.4 us. **Per text display line ~1.0-1.1 us**: extraction 0.25, submission 0.32 (per display), glyphs and
  background. Text displays are the cheaper hologram, on the CPU and in the tick.

**Changes**, each A/B tested in five interleaved pairs with `-Pbench.staticSeconds=10`, switch off vs on in the same build
(median; `build/run/bench/results/<dir>` of the worktree):
- **Name tag text kept** (`279021d`, `p-nametag`, switch `nameTagTextCache`): lobby static 269.8 -> 318.6 fps (every pair;
  render thread 3.55 -> 3.04 ms, p99 low 155.7 -> 185.1), pan 1391.8 -> 1444.8 (every pair). Probe: name display submit 970
  -> 192 ns per named entity, name tag extraction 520 -> 584 ns (the equality test). Vanilla builds the display name every
  frame (`Entity.getDisplayName`: team formatting copy, hover event, insertion), so its own visual order cache never hits,
  and runs ICU bidi + Arabic shaping (`FormattedBidiReorder`) and two glyph layouts per name tag and frame.
- **Text display lines kept** (`279021d`, switch `textDisplayTextCache`): lobby (`p-textdisplay`) static 305.9 -> 317.7 (3 of 5
  pairs; 3.13 -> 3.04 ms, 4 of 5); text display holograms only (`p-textdisplay-text`, 480 lines) static 1160.6 -> 1293.8
  (every pair; 0.82 -> 0.74 ms, ~0.17 us per line), pan 1902.9 -> 1963.9 (every pair).
- Checks for both (temporary, `scripts/patches/s19-check-TextCacheCheck.java.txt`): 3.2M kept widths, 1.6M visual orders
  of kept vs fresh equal components and 5.4M prepared texts rendered into a recorder against fresh layouts: 0 mismatches.
  Frozen close-ups with either switch flipped: text identical.
- **Text displays culled by their text** (`955cd51`, `p-tdcull`, switch `textDisplayCulling`): lobby pan 1066.3 -> 1438.9 fps
  (every pair; render thread 0.88 -> 0.58 ms, p99 low 281.2 -> 300.2), static 317.5 -> 323.3 (3 of 5; some holograms are off
  screen at the sides). Vanilla never frustum culls a display entity whose width or height is 0 (`Display.noCulling`), the
  default and what hologram plugins use, so every text display within its view range (64 blocks x view range x entity
  distance scale) is extracted, submitted and its text written every frame. Check (temporary,
  `scripts/patches/s19-check-TextDisplayCullingCheckM.java.txt`): 462k drawn displays, 58.6M glyph and background vertices, all inside the bounding sphere
  (at most 0.51 of its radius). Close-ups with holograms cut by the screen edges, switch flipped: identical text.
- **Empty armor stand layers skipped** (`8b2cf0f`, `p-standlayers`, switch `emptyArmorStandLayers`): stand holograms only
  (480 lines) static 902.4 -> 949.2 (4 of 5 pairs; 1.05 -> 1.00 ms, 4 of 5), pan 1851.6 vs 1876.5. Pair 2's off run (959.8,
  above every other off run, one 31.5 ms frame) may be the run during which the screen went off; without that pair 900.9 ->
  954.8. Close-ups with the switch flipped: holograms identical.
- Close-up pixel counts: the first-person arm's outline and the enchanted leggings' glint differ between frozen shots of
  one frame with the same setting (up to ~10k pixels at threshold 1); every diff map of a switch flip showed only those.

**Measured, not changed:**
- **`TransparentFaces` costs players with resource skins 0.5 ms per frame** (`p-tf-resource`, 150 players with
  `bench.playerSkins=resource`, switch `transparentFaceCulling` off -> on): static 476.6 -> 382.1 fps (-20%, every pair; render
  thread 2.03 -> 2.53 ms, ~3.3 us per player), pan 1572.4 -> 1501.5. Each `PlayerModel` cube sees 9 skins per model, more than
  the 4 texture masks a cube keeps (`ModelPartCubeM.transparentPolygons`, section 8 "Decisions"), so every player's cubes
  rescan their faces' texels every frame. This is the cache miss noted in section 8; part 1 added a per texture
  mask cache (`a4d2751`, on `26.3`; merged, but not measured in this scene yet). Downloaded skins (`DynamicTexture`) are not tested by
  `TransparentFaces` at all, so real online players neither lose nor gain; extending it to them (their `NativeImage` is
  kept by the texture) would let empty outer-layer faces be skipped exactly once the cache holds many skins.
- The lobby's GPU frame is 0.82 ms (vanilla 1.50): the hologram text and backgrounds and the players are cheap on the GPU.
- Probe overheads: with parts the probe slows the lobby from ~320 to ~260 fps; numbers above are per call and rank work only.

**Not tried, with the reasons:**
- **The player body** (2.8 us per player, the largest single item, plus its per-frame quad sort and its own draw): it can't
  be instanced in place (strictly ordered translucent group, quads sorted on upload), and moving it to the solid phase is not
  exact (vanilla draws translucent text backgrounds before the translucent models phase, so a player behind a hologram
  background is hidden by the background's depth in vanilla). An exact path would keep each player's draw in its sorted
  place but take its vertices from a per-part GPU pose buffer, and only skip the quad sort for skins with no
  semi-transparent texels in drawn faces (then the order of opaque and discarded fragments doesn't matter). Large.
- **Retained or instanced billboard text** (name tags and center-billboard text displays: each frame's geometry is the same
  label-space quads under a new matrix): would remove most of the remaining glyph writes and upload, but the see-through
  name tag pass is sorted and needs vanilla's order, and per-label draws would cost more than the writes unless drawn with
  multi-draw indirect and per-draw instance offsets. Large.
- Avatar render state reuse (as `ReusedRenderStates` does for armor stands): not tried; the tenth session found no gain
  for other living entities, and players are where mods attach render state data (Fabric's render state data map), which a
  reused state would carry over.
- A display name shortcut (skip building the display name when team, custom name and team prefix/suffix/colour are the
  same objects): ~0.25 us per name tag, but a mod that injects into `getDisplayName` would show stale names.
- The player body's per-frame quad sort (1.8 us per player): skipping it for skins without semi-transparent texels is
  **not** exact (the arm's inner face touches the body's side, and such coplanar faces z-fight in vanilla's per-frame sort
  order). Reusing last frame's order when a sorted draw's decoded centroids are bit-identical (a still player under a still
  camera; bobbing is in the projection, not the positions) would be exact, at a hash and compare per sorted draw.

Session start (`011dec9`, the scene only) vs end (`8b2cf0f`), checked out in turn in the worktree:
- **Lobby** (five pairs, `sess-lobby`): static 271.6 -> 326.4 fps (every pair; render thread 3.57 -> 2.96 ms, p99 low 161.3
  -> 197.4), pan 959.8 -> 1438.5 (every pair; 0.99 -> 0.56 ms, p99 low 249.8 -> 301.0).
- **Entity scene** (three pairs, `sess-entities`; ~21 text displays and some named mobs): static 338.6 vs 339.4 (render
  thread 2.87 -> 2.81 ms), pan 1148.8 -> 1184.4 (every pair; 0.80 -> 0.76 ms).
- **Mixed scene** (three pairs, `sess-mixed`): static 1567.5 vs 1561.2, pan 1512.5 vs 1507.5, particles 1256.6 vs 1264.5,
  initial 1113.1 vs 1124.4: unchanged.

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
tolerance and a 0.05 block margin, because rotated vertices aren't exactly diagonal; the patch is
`scripts/patches/s05-diagonal-groups.patch` (made against `b63badc`).

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
is `scripts/patches/s07-terrain-occlusion.patch` (made against `dd183cb`); results and logs in
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

### Decisions and verification notes
On 2026-10-28 the user reviewed the items below and **kept all of them** (transparent face culling, supporting
blocks, plane culling and the mod compatibility trade-offs). Nothing from before the nineteenth session is pending (the
nineteenth session's switches are not reviewed yet). The notes stay as reference; the
`TransparentFaces` cache miss with instanced parts off was fixed in the nineteenth session (section 7).
- **`TransparentFaces` (7e76c79) is ~8% slower with instanced parts off** (independent verification after the
  fourteenth session, `results/verify-s14/`): entity scene, `-Pvm.instancedParts=false`, 3 pairs, every pair slower
  with the culling on: static 194.9 -> 179.7 fps (render thread 4.99 -> 5.37 ms), pan 987 -> 906. With defaults it
  is a real gain (5 pairs: static 298.5 -> 313.7, 3.26 -> 3.05 ms). Likely (inferred, not profiled) cause: each cube
  caches its face mask for only 4 textures (`ModelPartCubeM.transparentPolygons`), so models whose cubes see many
  textures per frame (villager clothing, ~28) rescan the faces' UV rectangles on every miss. This also hits the
  non-instanced fallbacks with defaults (translucent models with >4 textures, `MAX_BATCHES` overflow). Kept (user,
  2026-10-28). **Fixed (nineteenth session, `a4d2751`):** confirmed (826 misses per frame at ~1.06 us); per texture hash
  tables per cube, instanced parts off culling off -> on now 196.0 -> 221.6 fps.
- The server scene's static render thread CPU read 0.71 -> 0.74 ms in all 4 verification pairs (4150777 vs a9aafe1),
  fps unchanged (GPU bound); probably the per-submit `TransparentFaces.of` / `ModelPoseReplay.begin` checks across
  the 296 armor stands (not profiled).
- Mod compatibility of `ModelPoseReplay`: replayed layer submits bypass `setupAnim` and `ModelPart.render`, so a mod
  hooking those on a vanilla model class is skipped for layers (switch "Reuse Layer Poses").
- Sixteenth session, mod compatibility only (all four changes are exact for vanilla, each has a switch):
  `fastEntityCollisions` overrides `EntityGetter.getEntityCollisions` in `Level`, so a mod injecting into that default
  method (Lithium does) or into `Entity.canBeCollidedWith` (instead of overriding it in a class) is bypassed;
  `fastSupportingBlocks` relies on client block changes going through `LevelChunk.setBlockState` or chunk data, and on
  blocks with context dependent collision shapes overriding `getCollisionShape` (a mod changing shapes by mixin into
  `BlockBehaviour` would be missed); `fastBoatTicks` pushes only `minecraft.player` on the client, as vanilla's own
  `ClientLevel.getPushableEntities` assumes for living entities.
- Sixteenth session, verified independently (`results/verify/`): all four switches exact (~3.3M compared answers incl.
  nether biome patches, shulkers and a happy ghast, a survival player on boats, floor swaps through slab/ice/honey/soul
  sand/stairs, and the multiplayer server scene; 0 mismatches). Entity scene tick, 5 pairs off -> on:
  `fastEntityCollisions` 1.422 -> 1.281 ms, `fastAnimateTick` 1.330 -> 1.251, `fastBoatTicks` 1.324 -> 1.281 (16 boats),
  `fastSupportingBlocks` 1.433 -> 1.401 (4 of 5; pan 1.358 -> 1.286). `fastSupportingBlocks` is exact and measurable
  but the smallest gain for its size (~280 lines, an `Entity` field, chunk mixins); a mod writing
  `LevelChunkSection`s directly on the client would leave it stale. Kept (user, 2026-10-28). The server scene's tick gain is
  ~0.05 ms (not 0.1; spread +-0.1) and its armor stand tick slowdown was noise (no consistent sign over 3 matrices).
  `fastAnimateTick` loses vanilla's detection of `level.random` being used from two threads.
- Fifteenth session, verified independently (`results/verify-s15/`: 0 mismatches over 23-39M answers per run on
  Vulkan, GL and the multiplayer server scene, incl. chunk unload/reload and light packets; each switch +3% in its
  phase, also with instanced parts off). Mod compatibility only: `entityLightCache` would serve stale light to a
  dynamic-lights mod or a preview level whose light isn't backed by `ClientChunkCache`; `fastEntityCulling` bypasses
  mods hooking `EntityRenderDispatcher.shouldRender` / `EntityRenderer.shouldRender` for plain renderers. Leashed
  entities miss the light cache every frame (the leash reads light at the eye position).
- Seventeenth session, mod compatibility only (each exact for vanilla, each with a switch): `batchTableTicks` runs the
  idle branch of `EnchantingTableBlockEntity.bookAnimationTick` as a copy, so a mod injecting into that method is bypassed
  for tables without a player near; `retainedChangeChecks` assumes a retained block entity's look only changes with its
  block state, the light at it (and at a double chest's other half) or an event that marks it dirty (block events, data
  packets, sign text, breaking progress), so a mod that changes a retained vanilla block entity's fields on the client
  without one of these leaves its geometry stale until the next rebuild.
- Seventeenth session, a trade-off kept (user, 2026-10-28): drawing retained quads by plane (`9c15ac0`) gains ~3% in the server scene (GPU
  0.873 -> 0.837 ms) but costs the block entity scene's static view ~0.6% (GPU 0.542 -> 0.546 ms; section 7). Revisit if
  a scene that matters more looks like the block entity field from outside (no section containing the eye).
- Seventeenth session, verified independently (`results/v17-*`, `v17-vis/`): no correctness or visual issue in any
  commit (code read against vanilla; scripted bell rings, lectern book removal/placement, statue pose and oxidation
  changes, a sign edited by a server data packet and a double chest join all match retained off on Vulkan and OpenGL;
  the change checks are pixel-identical on OpenGL too). Every gain reproduced: tables tick 1.225 -> 0.978 ms, plane
  culling server static 1066.3 -> 1091.1 (GPU 0.866 -> 0.832 ms), book tilt 841.6 -> 856.3 facing the cube, extras
  scene 214.0 -> 1283.6 with retained lecterns/bells/statues, change checks extras p99 low 264 -> 720. Start -> end:
  server static 1087.8 -> 1126.1, block entity scene 1761.3 -> 1749.8 (-0.65%, 4 of 5 pairs), entity scene unchanged.
  The plane culling commit alone costs the block entity scene only -0.1% (+1-2 us GPU), so most of the -0.65% is not
  from it and is unattributed. Compare retained screenshots at threshold 12, not 1 (time-of-day drift dominates
  threshold 1; retained types show 15-19 px at t12 vs retained off).
- Nineteenth session, part 1: mod compatibility notes at the end of its entry in section 7 (not reviewed by the user yet).
- Nineteenth session, part 2 (players and holograms, not reviewed by the user yet), mod compatibility only; each switch is exact
  for vanilla and can be turned off: `nameTagTextCache` hands the renderer the component the entity showed last when the new
  one is `equals` to it, so a mod that mutates the name tag component in place after extraction would mutate the kept one,
  and a mod that relies on a new instance each frame sees the same one (mods that return different names each frame only
  miss the cache); `textDisplayTextCache` wraps text display lines in a `FormattedCharSequence` of its own, as sign lines
  already are; `textDisplayCulling` assumes everything a text display draws comes from `TextDisplayRenderer.submitInner`
  (a mod drawing more for a text display, outside its text, would lose it while the text is off screen; vanilla's own
  `shouldRender` and mods hooking it still run first); `emptyArmorStandLayers` bypasses mods that inject into vanilla's
  four armor stand layers to draw something for an armor stand with nothing equipped (mods that add a layer are respected).

1. **Entity scene hot spots** (section 6.9; static ~2.95 ms render thread after the fifteenth session, section 7).
   Done: block model mesh cache, hashed draw lookup, baked quads a quad at a time (10th), instanced parts (11th),
   instanced items, entity block models and armor, mesh submits without copies, item bounding boxes, part rotations,
   zero move collision queries (13th), transparent face culling, layer pose replay with array batch lookup, no vertex
   builder for instanced submits (14th), entity culling without allocations or renderer lookups and light kept
   per entity (15th). Block displays now extract in 0.64 us; minecarts in ~1.3 us, a third of it
   vanilla's rail lookup (`OldMinecartBehavior.getPos`, two block reads), ~20 us per frame for 16, not worth more.
   Measured leads, entity cost probe at the end of the fourteenth session (per frame unless noted):
   - **Model prepares ~0.82 ms** (1.07 at the session's start): the part trees of first submits are ~0.38 ms
     (~60 ns per part: `Matrix4f.rotate` ~3% of the render thread in JFR, instance writes ~3%, `pushPose`, the
     recursion and the children map's iterator ~1-3%); each is below the ~2% an A/B can resolve. Villager and zombie
     villager submits ~0.5 us (bodies ~1 us, replayed layers ~0.35). Armor pieces ~87 us (each slot is its own model,
     so no replay; their part poses equal the body's only by construction, not bit for bit). Horse markings ~43 us
     (vanilla translucent path, empty faces skipped), charged creeper swirl ~17 us (texture matrix), glint ~15 us,
     boat water masks ~13 us. The `ModelFeatureRenderer` group costs ~70 us beyond its prepares (build loop, instance
     copies at `endGroup`). Looking texture alphas up only when a cube is written (they are looked up per render type
     change, ~1% in JFR) measured within noise (section 7). Translucent and eyes types can't be instanced: their
     groups are strictly ordered and instancing reorders parts of different submits.
   - **Extraction and submission** (fifteenth session, section 7): done: entity culling without allocations or renderer
     lookups (the pan culls all ~1.3k loaded entities every frame), renderers kept on the entity type, light kept per
     entity. What is left is a long tail of vanilla per-entity work, each item at or below the ~2% an A/B resolves (JFR
     shares of the static render thread at the end of the session; JFR misattributed twice this session, see 6.6):
     name tags ~2.7% (but skipping their attribute reads gained nothing), villager clothing identifiers ~3% (keeping them
     cut the layer to 1.4% in JFR and -46 us in the probe, but the A/B was within noise; patch
     `scripts/patches/s15-villager-ids.patch`), item frames' frame block model resolve ~1.2% (geometry key per frame), item
     model resolves (~160 ns each, `ModelManager.getItemModel` hash lookups), armor layers ~2% (spread over render type
     memo lookups, `EquipmentClientInfo.getLayers` and submit records), the pose stack's matrix math in submission ~3%
     (`Matrix4f.rotateY`, `pushPose`), `LivingEntityRenderer.submit`'s `model.setupAnim` before layers ~1% (only layers
     reading the parent model's poses need it, e.g. held items and head items; skipping it per layer class would be
     typed), `SubmitNodeStorage.order` tree lookups ~0.4%, the culling's remaining distance test, section lookup and
     leash check. A different approach would be needed for more: e.g. skipping extraction of entities whose render
     state can't change (none is known to be safe generically; see the tenth session on state reuse).
   - **Client tick** (1.261 ms static after the sixteenth session, from 1.596; every ~12th frame at ~300 fps,
     so it sets the p95/p99). Done: collidable entity sections, animateTick, boat ticks, supporting blocks kept (16th).
     What is left, per tick (static view): entities ~1.0 ms, a long tail: ~600 mobs at ~0.6-1.0 us each (base tick and
     fluid interaction, `aiStep`, `commonTick`'s interpolation and first touch of each entity; no single part above ~3%),
     256 floating items ~0.17 ms and 64 `NoGravity` orbs ~0.13 ms (scene artifacts: zero moves still run `move`, inside
     block checks and, for orbs, an interpolation `noCollision`), the player ~0.06-0.11 ms (warm-up, section 6.6), 16 boats
     ~46 us. Texture atlas animation ~0.18-0.2 ms (blocks atlas 134 us: 56 animated sprites, ~42 drawn per tick into 5 mip
     levels; particles 31 us for one sprite; ~30 us per render pass, native driver time; only drawing visible sprites, as
     Sodium does, would not be exact), `animateTick` ~0.12 ms (1334 block reads and particle checks), player sync ~0.05.
     In real play (`-Pbench.entityGravity=true`) resting items and orbs also paid `collide`'s block sweep; kept since the
     nineteenth session (`RestingCollisions`, tick 1.429 -> 1.360 ms with gravity). What is left of their `move` is mostly
     `checkSupportingBlock` (kept) and the rest of vanilla's move bookkeeping. Dropped items from fights cost ~1.1 us each.
     Don't A/B ticks with `-Pbench.entityAi=true`: fights and explosions make every run a different scene (creepers left
     3-18, items 419-465 over five runs). The frames with a tick are also slower outside
     the tick, most likely the benchmark harness's handshake and length-biased sampling (section 6.6).
   - Players: measured in the players scene (nineteenth session, section 7 "Players and holograms"). Their name tags are
     done; their body stays on vanilla's translucent vertex path (the largest item per player, 2.8 us plus a per-frame quad
     sort and a draw of its own), see item 1b. Other translucent types stay vanilla.
1b. **Players and holograms, next** (section 7 "Players and holograms", in order of expected gain):
   - `TransparentFaces` with many skins: confirm part 1's per texture mask cache (`a4d2751`, merged) in the players scene with
     `-Pbench.playerSkins=resource` (here -0.5 ms per frame for 150 players without it), then consider testing downloaded
     skins too (`DynamicTexture.getPixels`; a skin's empty outer-layer faces would be skipped, exactly).
   - The shared vertex buffer upload is 0.51 ms of the lobby's 3.2 ms frame (light profiler depth 6), ~0.37 ms of it the
     sorted draws (upload table of `-Pbench.entityCost=true`, added this session); each player body's 72 quads are sorted
     every frame (1.8 us per player). Reusing last frame's order for sorted draws with bit-identical centroids is exact;
     skipping the sort for opaque skins is not (coplanar touching faces, section 7).
   - The player body as a per-player draw fed from a GPU pose buffer, in its sorted place (exact only with vanilla's quad
     order or for skins without semi-transparent texels). Large.
   - Retained/instanced billboard text for name tags (normal pass only; the see-through pass is sorted) and text displays.
   - Per player extraction (1.6 us) and layer submission (1.4 us) are a long tail of vanilla work.
2. **Per entity and block entity cost in the server scene** (twelfth and seventeenth session measurements in section 7).
   The server is gone; the mixed scene (section 6.1) has the same mix and is where this continues.
   The static view is GPU bound (GPU frame ~0.84 ms, render thread ~0.72 ms), so CPU work there shows in the render thread
   time and the p99 (tick frames); GPU work shows in the average. Done: sign text retained, closed books without
   extraction, cached table light, reused armor stand states (8th/9th), parked idle tickers, validated ticker states, typed
   armor stand pushes, retained faces by facing, packed terrain draws (12th), enchanting tables ticked in one loop,
   retained quads drawn by plane, the books' tilt computed once, lecterns/bells/copper golem statues retained, retained
   change checks and 30 s eviction (17th). Ruled out: cheaper entity culling, generic render state reuse (10th), sprite
   pipeline repeats, cell index masks and front-to-back plane order (17th, within noise). What is left, measured:
   - **GPU, retained 0.141 ms** for 131.6k quads per frame (static), half of them sign text of the signs inside the chest
     wall, mostly hidden behind chests; there is no exact way to skip occluded quads (chests are 14/16 wide, renderpearl
     has no occlusion queries). Instanced models 0.063 ms (armor stands are `entity_cutout`, drawn without culling).
   - **Client tick ~0.93 ms** (static): armor stands ~0.23-0.32 ms (296 at ~0.8 us, a long tail of base tick, `aiStep`,
     fluid checks and `commonTick`), the local player 0.1-0.25 ms (mostly once-per-tick code that is still warming up),
     the enchanting table batch ~0.13 ms (~23 ns per table, mostly loading each table), texture animation 0.10-0.15 ms
     (blocks atlas: ~42 sprites into 5 mip levels, a render pass each; native time), animateTick ~0.12, player sync ~0.04.
     Tick frames also carry the harness's handshake (section 6.6).
   - **Block entity extraction** ~0.09 ms per frame at the static view (hidden by the GPU wait); facing the chest cube
     0.2-0.3 ms, mostly closed books (~3.6k at ~60-70 ns after the tilt change: pose math and the instance write).
   - **Glowing sign text**: retained since the nineteenth session (`a4651b2`); coloured glowing signs within the outline
     reach take the vanilla path frame by frame.
   - **Retained builds after joining or teleporting**: a section is built in one go; in the mixed scene's initial phase
     three frames build 2-2.7k block entities each in 95/38/27 ms (cold). Spreading builds over frames with a time budget
     was built, verified exact and binned (nineteenth session, `scripts/patches/s19-spread-builds.patch`): the unbuilt
     chests' vanilla path made more slow frames than it saved. The phase's largest frames (~120 ms, frames 0-3 after the
     teleport) are not builds and were not investigated; that is the lead for join smoothness.
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
  `origin` = `6a0/Ruthenium` (renamed from `6a0/VulkanMod` on 2026-10-28), `upstream` = `xCollateral/VulkanMod`. Branch `26.3` is pushed to `origin` (since 2026-10-28) and tracks `origin/26.3`; nothing goes to `upstream`.
- **No Sodium code.** It is PolyForm Shield licensed.
- **Safety of the culling:** `BlockEntityCulling` only handles `minecraft:` block entity types. If more types
  render outside block ±1 (+2 up), widen the box rather than special-casing.
- **Thread-safety assumptions:** `ModelPartCubeM`, `BakedSheetGlyphM` and `QuadParticleRenderStateM` keep
  per-instance scratch, which matches vanilla (models and text are built on the render thread).
  `DistanceSorting` uses a `ThreadLocal` because sorting can run on worker threads. `SignTextCache`,
  `ModelCulling`, `ModelGeometryCache`, `ParallelUploadCopy` and `RetainedBlockEntities` are static, render-thread-only
  state (`ParallelUploadCopy`'s worker only copies chunks of the current job; `RetainedBlockEntities.markDirty`/`clear`
  may be called from other threads and only queue work). `EntityTextCache` keeps name tags in `Entity` and
  `MutableComponent` fields and only reads or writes them on the render thread (`RenderSystem.isOnRenderThread()`); a
  name tag built elsewhere (a GUI preview on another thread) takes vanilla's path.
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
