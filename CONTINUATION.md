# Ruthenium (formerly VulkanMod) → Minecraft 26.3 port: continuation notes

This is a hand-off document for a new session. It covers what was decided, what has been built, how to test it,
what the numbers are, and what to do next. Read it all before changing code.

Last updated: 2026-10-30 (twenty-eighth session: GPU time, the OpenGL gap, the initial phase; twenty-seventh; twenty-sixth; twenty-fifth; twenty-fourth; twenty-third; twenty-second; twenty-first; twentieth; before that 2026-10-28, seventeenth session; earlier: 2026-10-27, sixth session), with the GPU probe findings and the faster section compiles (biome blend,
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
Twentieth session (section 7, "Twentieth session"): part 1's per texture mask cache measured in the players scene: the -20%
with resource skins is gone (players static 477.3 -> 484.5 fps with the culling on, lobby 326.7 -> 331.6). Transparent face
culling of **downloaded skins** (`DynamicTexture`, alpha copied after each upload, masks kept per texture alpha for 100+ skins)
was built, checked exact (also across re-uploads) and **binned**: no gain (150 distinct skins static 482.1 vs 474.8, lobby 323.8
vs 325.1), because the one-texel margin that keeps the test exact leaves almost no outer layer face skippable once the base
layer is opaque (patch `scripts/patches/s20-downloaded-skins.patch`). The players scene now registers its skins through
vanilla's skin downloader and has `bench.skinVariants`, `bench.emptyOuterLayers` and `bench.skinEdit`. No `src/main` change.
Twenty-first session (section 7, "Twenty-first session"): **sorted draws start their quad sort from their last order**
(`SortOrderCache`, switch "Reuse Sort Order"; exact: the sort's order is unique, so the start only changes its speed). Players
static 482.7 -> 540.6 fps (render thread 1.99 -> 1.80 ms), lobby 328.0 -> 352.9 (2.96 -> 2.73), moving players 465.3 -> 511.1.
"Still" players are not bit-identical frame to frame (idle arm bob), so reuse keyed on identical centroids would have missed.
Twenty-second session (section 7, "Twenty-second session"): **player bodies are drawn from a part-space mesh and per-frame part
poses** (`PlayerPoseDraws`, switch "GPU Player Poses"; section 8's bigger job 1): each body keeps its draw's place in the strictly
ordered translucent group, the GPU poses a static mesh from a uniform slice per draw, and the faces are sorted from centroids
computed on the CPU with vanilla's float operations. Exact: 0 mismatches in positions, centroids, normals and index order over 5.86M
posed members; 0 pixels differ across the switch in frozen close-ups (Vulkan and OpenGL). Players static 523.6 -> 603.5 fps (render
thread 1.86 -> 1.60 ms), 150 skins 514.1 -> 605.0, moving players 507.7 -> 581.1, lobby 351.6 -> 397.3 (2.76 -> 2.45 ms), OpenGL
players 417.7 -> 465.0; mixed and entity scenes within noise.
Twenty-third session (section 7, "Twenty-third session"): **labels are drawn from label-space glyph meshes and per-frame poses**
(`RetainedLabels`, switch "Retained Labels"; section 8's bigger job 2): name tags, text display lines and other kept text are recorded
once into a shared vertex buffer whose vertices name their label's region in a texel buffer of poses; each glyph run takes vanilla's
draw (all phases, see-through included), unsorted draws are indirect multi-draws over arena ranges, sorted `text_polygon_offset` draws get
vanilla's order from CPU centroids. Exact: 0 mismatches over 24M labels, 0 pixels across the switch in frozen close-ups (Vulkan and
OpenGL; a new `bench.labelVariety` wall of special labels) except vanilla's obfuscated text. Lobby static 398.8 -> 479.1 fps (render
thread 2.43 -> 2.03 ms), moving players 382.9 -> 457.6, holograms only 1226.9 -> 1721.0, OpenGL lobby 329.5 -> 392.7; mixed unchanged,
entity scene ~1.5% lower (explained and fixed in the twenty-fourth). Also: the nineteenth session's claim that text backgrounds draw
before the translucent models was wrong (they draw after).
Twenty-fourth session (section 7, "Twenty-fourth session"): the entity scene's regression with Retained Labels was **a fixed ~20 us per
frame in `vkQueueSubmit2` that the NVIDIA Vulkan driver charges any submit using a texel buffer** (vanilla pays it too with clouds on), plus
the per-push `VkBufferView`; label poses are now an RGBA32F texture. Entity scene with the switch: pan lower in 5 of 5 pairs before, now
1198.5 -> 1203.4 (4 of 5 higher), static 339.9 -> 344.1; lobby static 390.0 -> 478.2, pan 1521.6 -> 1574.1. New tool: an in-launch A/B
of a switch (`bench.toggleField`, section 6.6), which resolves ~0.5% where 5 interleaved launches resolve ~1.5%.


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
- **Twentieth session** (section 7): transparent face culling is neutral to +1.5% with many resource skins now; extending it to
  downloaded skins was built, checked exact and binned (no gain, `scripts/patches/s20-downloaded-skins.patch`).
- **Twenty-first session** (section 7): sorted draws (player bodies, see-through name tags) start an insertion sort from their
  order in the last upload ("Reuse Sort Order", `reuseSortOrder`): players static +12%, lobby +7.6%, moving players +10%; 0
  mismatches against vanilla's sort over 4.97M draws.
- **Twenty-second session** (section 7): player bodies drawn from a GPU pose buffer ("GPU Player Poses", `playerPoseDraws`):
  a static part-space mesh per `PlayerModel`, per-frame part poses in a uniform slice per draw, each body's draw in vanilla's place
  and quad order (CPU centroids with vanilla's float operations). Players static +15%, 150 skins +18%, moving +14%, lobby +13%,
  OpenGL +11%; 0 mismatches over 5.86M members, 0 pixels across the switch. In parallel, the block entity scene's drop since the
  seventeenth session (~1770 -> ~1370 static) turned out to be the machine: the seventeenth session's build is as slow now, +0.17
  ms GPU per frame since a restart on 28/10 (section 7, nineteenth session, "After the merge"). GPU bound numbers compare only
  within a session.
- **Twenty-third session** (section 7): labels retained ("Retained Labels", `retainedLabels`): label-space glyph meshes in one vertex
  buffer, per-frame pose rows and light per label in a pose texture (a texel buffer until the twenty-fourth session), vanilla's draws and quad order (indirect multi-draws over arena
  ranges for unsorted draws, CPU-sorted indices for `text_polygon_offset`). Lobby static +20% (2.43 -> 2.03 ms), holograms only +40%,
  OpenGL lobby +19%; 0 mismatches over 24M labels, 0 pixels across the switch except obfuscated text.
- **Twenty-fourth session** (section 7): the entity scene's ~1.5% loss with Retained Labels was the texel buffer (a fixed ~20 us per
  frame in the NVIDIA driver's `vkQueueSubmit2`); poses now go through an RGBA32F texture. In-launch switch A/B (`bench.toggleField`).
- **Twenty-fifth session** (section 7): the mixed static view is **CPU bound** (render thread ~0.6 ms vs GPU 0.36 ms), and 44% of it was the
  armor stand chain: the base plate, which vanilla turns back by each stand's yaw, split the instanced armor stands into a group per yaw (65
  groups for 244 instances). The plate is now an instance of its own ("Separate Armor Stand Plates", `armorStandPlates`): mixed static
  1599 -> 1800 fps (+12.5%, every pair), render thread 0.54 -> 0.42 ms. Retained builds sort their plane keys with a radix sort and gather
  straight into the upload (exact, no switch): warm 2048 block entity builds ~6.2 -> ~4.65 ms. Every Gradle command runs under the GPU lock
  (`scripts/gpulock.py`, section 6.3). Part 2: clouds read their faces from an R8_SINT texture instead of a texel buffer ("Cloud Face
  Texture", `cloudFaceTexture`): `vkQueueSubmit2` -21.7 us per frame, mixed static 1786.7 -> 1847.6 fps (+3.4%, every pair), OpenGL +1.8%;
  instanced groups share one instance ring, one prepare per render type and their bindings (exact): ~0.8 us per group, -4 us at the default 7
  groups; full size screenshots copy rows ("Fast Screenshots", `fastScreenshots`): the new world's icon readback 24-33 -> 11-15 ms.
- **Twenty-sixth session** (section 7):
  - **OpenGL:** the plate change gains +30.7% mixed static there (Vulkan +12.5%), and the radix sort cuts warm builds by 24%. Neither
    regresses.
  - **Stale instanced groups are evicted** (`InstancedModels`, no switch). With plates off, after the yaw sync, static goes 1526 -> 1804
    fps; plates on is unchanged. This also explains the plate change's "seven unexplained pixels": those stands weren't instanced.
  - **Candidate 6:** a front cache before the two-argument render type memos ("Render Type Lookup Cache") and empty hands skipping the
    crossbow charge time's enchantment iteration ("Skip Empty Crossbow Charge Time"). Both are exact. Each saves ~3 us of render thread and
    9-12% of its allocation. The mixed static fps doesn't move, because that view now waits on the GPU.
- **Twenty-seventh session** (section 7): **render-thread allocation** cut by 12 commits, 6 of them with new switches. Static view
  KB/frame: entities 2156 -> 1045, players 1296 -> 747, mixed 105 -> 63. Players +7.6% fps on Vulkan and +7.4% on OpenGL; entities +2%.
  The changes: pooled submit poses, no Fabric scratch mesh for cached block models, model part children from an array, kept display
  names, and more. Render states are now the largest source left.
- **Twenty-eighth session** (section 7):
  - **Mixed scene GPU time:** session start vs end, Vulkan static 1857 -> 2290 fps (+23.4%), pan +11.1%, particles +18.9%; OpenGL static
    +4.7%, pan +6.2%. Three new switches: "Skip Black Vignette" (the GUI pass goes when the vignette is black), "Cull Closed Model Insides"
    (instanced armor stands drawn with back face culling), "Retained Cell Culling" (retained chests drawn per 8³ cell, cells outside the
    view skipped). The block entity scene's static view lost 2.1% (dense sections fully in view).
  - **The frame gap** is GPU-side present work plus submit latency (vanilla queues one frame). The bench starts each launch in a slower
    present path because `options.txt` started with vsync on; since 2026-10-30 they start with vsync off, so Vulkan numbers are 13-24% higher than earlier ones (section 6.4).
  - **OpenGL gap:** ~85% of its extra ~0.2 ms is vanilla's GL backend and the driver. "Skip Redundant Frame Clear" skips two clears
    that a later clear overwrites: OpenGL static +2.5%, pan +4.8%; Vulkan +1.6%, +1.4%. `7193e1d` gains +17.9% on OpenGL with plates off.
  - The mixed `initial` phase's ~2% drop was noise.
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
| `mixin/render/vertex/StagedVertexBufferSortM.java` + `render/vertex/SortOrderCache.java` + `render/vertex/SortedQuadIndices.java` | **Sort order reuse** (twenty-first session, switch `reuseSortOrder`, "Reuse Sort Order"). A `@WrapOperation` on `writeSortedIndexBuffer` in `StagedVertexBuffer.uploadDrawsToBuffers` sorts each sorted draw whose `VertexSorting` is a `DistanceSorting.Sorter` through a per buffer `SortOrderCache`: the draw is matched to an entry by quad count and the half-block cell of its first centroid, and `Sorter.sortFrom` insertion sorts from the entry's order (in place), falling back to the full sort after 4n+64 moves; no entry: full sort, new entry. Entries unused for 4 uploads are dropped (the level's and the hand's features share one buffer). Exact because every sort orders by the same strict total order (key, then quad index), whatever the start. Other `VertexSorting`s (mods) take the vanilla call. `SortedQuadIndices` holds the index writer `MeshDataSortStateM` used to keep |
| `render/entity/PlayerPoseDraws.java` + `mixin/render/feature/{RenderTypeFeatureRendererM,RenderTypeFeatureRendererGroupM}.java` + `mixin/render/vertex/StagedVertexBufferDrawM.java` + `ModelPartM`/`ModelPartCubeM`/`ModelM`/`ModelFeatureRendererM` + `interfaces/{PosedDrawGroup,PosedDrawHolder,PosedPart,PoseLayoutHolder,TransparentCube}` + `assets/ruthenium/shaders/core/player_pose.vsh` | **Player bodies drawn from poses** (twenty-second session, switch `playerPoseDraws`, "GPU Player Poses"). A submit of the exact `PlayerModel` class with vanilla's `entity_translucent` pipeline (no UV mapping, no decal) in a strictly ordered group of the world's features (the translucent models phase), without OIT, is not written: `ModelFeatureRendererM.prepareModel` hands it to `PlayerPoseDraws.prepare`, which runs vanilla's `setupAnim` and part tree into a sink, and `ModelPart.compile` passes each written part's pose to `capturePart` instead of writing its cubes. Each `PlayerModel` instance (wide, slim, per renderer) gets a layout: one slot per part with cubes (max 12) and a static mesh of its cubes in part space (position, UV, normal as floats, member and slot as `RG16_SINT`), in 8 copies, one per member of a draw. Per member (1184 B of a std140 uniform block): light and overlay as the entity format's shorts, color, and per slot the pose rows and the normal matrix rows (w: normalize or not). The member joins the group's draws exactly as `getVertexBuilder` would place its vertices (`PosedDrawGroup`: a new staged draw via `getOrAddDraw`, or the group's last draw when the render type repeats, i.e. consecutive bodies of one skin share a draw as in vanilla); the staged draw stays empty (skipped on upload) and carries the posed draw (`StagedVertexBufferDrawM`), which `RenderTypeFeatureRendererM` draws in its place (a redirect of `executeGroup`'s `getExecuteInfo`): a copy of `entity_translucent` with `player_pose.vsh`, the draw's prepared transforms and textures, its uniform slice (`PlayerPoses`, offset aligned to `minUniformOffsetAlignment`, the full 9472 B block bound) and 16-bit indices from a ring buffer. **Order:** each face's centroid is computed on the CPU from the same JOML `transformPosition` of its vertices 0 and 2 and the same `(a + b) / 2.0F` as `MeshData.decodeQuadCentroids`, faces are numbered in vanilla's write order (part tree, cubes, polygons, minus the faces `TransparentFaces` skips, through `ModelPartCubeM`'s per texture masks), and the draw's own `VertexSorting` sorts them (`SortOrderCache` start orders with `reuseSortOrder`), after the world's features are prepared (`upload`). **GPU arithmetic:** positions in JOML's operation order with `precise` (as `instanced_part.vsh`); normals as vanilla writes them: `Matrix3f.transform` order, `normalize` only for untrusted normals, truncated to signed bytes and decoded through a table of `max(c / 127.0F, -1.0F)`. A vanilla write that joins a posed draw (same render type, consecutive) writes the posed members first with vanilla's path (`materialize`, from a redirect of the staged builder fetch in `Group.getVertexBuilder`); captures fail (vanilla path) for parts that aren't the layout's, a part written twice or any vertex written to the sink (mods), and a model whose captures fail 64 times in a row takes vanilla's path until the next clear. Max 64 layouts; cleared on resource reload and when toggled. Render thread only |
| `render/text/RetainedLabels.java` + `SignTextCache.KeptText` + `mixin/render/text/TextFeatureRendererM.java` (visit redirect) + `RenderTypeFeatureRendererM`/`RenderTypeFeatureRendererGroupM`/`StagedVertexBufferDrawM` + `mixin/render/vertex/StagedVertexBufferAccessor.java` + `interfaces/LabelDrawHolder` + `assets/ruthenium/shaders/core/label_text.vsh` + access widener (`TextFeatureRenderer$GlyphRenderer`) | **Retained labels** (twenty-third session, switch `retainedLabels`, "Retained Labels"; section 8's bigger job 2). Text submits without outline whose prepared text `SignTextCache` keeps (name tags, text display lines, signs that aren't retained; never obfuscated text) are not written: from a kept text's second frame its quads are recorded once in label space (the coordinates handed to `addVertex(pose, x, y, z)`, UV, color; only `BakedSheetGlyph` glyph and effect instances, at most 2048 quads) into an arena vertex buffer (28 B per vertex, each tagged with the label's region; at most 16384 regions, a full arena is replaced next frame). Each run of glyphs with one render type picks the group's draw exactly as `Group.getVertexBuilder` would (`joiningDraw`/`getOrAddDraw`, so merged, new and strictly ordered draws are vanilla's) and joins that draw's `LabelDraw` if the draw holds no vertices; else it is written as vanilla writes it, and a vanilla write joining a label draw writes its labels first (`materialize`). Per label per frame: its pose rows and light (64 B per region) into an RGBA32F pose texture (256 x 256 texels, four per region, row r holding regions 64r to 64r + 63; the rows in use written with `writeToTexture` each frame, read with `texelFetch`; ordered after the last frame's label draws by the full barrier that ends every Vulkan render pass). Not a texel buffer: on the NVIDIA Vulkan driver any texel buffer in a submit costs ~20 us in `vkQueueSubmit2` (twenty-fourth session). Upload: unsorted draws as indirect multi-draws over ranges of the arena with the sequential quad index buffer (consecutive labels placed together are one range; gated like vanilla's terrain MDI), else 16/32-bit indices; sorted draws (`text_polygon_offset`, `text_grayscale_see_through`) get indices in the order of their own sorting over CPU centroids (JOML's `transformPosition` of vertices 0 and 2, `/ 2.0F`), kept from last frame when every input is bit-identical. Drawn in the staged draw's place with copies of the six world text pipelines and `label_text.vsh` (`precise` JOML-order position). Exact: 0 mismatches in positions, runs, UV, color, light and sort order over 24M labels; 0 pixels differ across the switch in frozen close-ups on Vulkan and OpenGL except vanilla's obfuscated text (random glyphs, also with an unrelated switch). Not with OIT. Cleared on resource reload and when toggled |
| `mixin/render/vertex/MeshDataSortStateM.java` | `@Overwrite` `writeSortedIndexBuffer`/`buildSortedIndexBuffer`: sorted quad indices written with `MemoryUtil` puts instead of an `IntConsumer` per index. Sign text (`text_polygon_offset`) is sorted on upload and needs 32-bit indices with ~500 signs |
| `render/text/SignTextCache.java` + `mixin/render/text/{SignTextM,AbstractSignRendererM,TextFeatureRendererM,FontSetM}.java` | `SignText.getRenderMessages` wraps each line (only when recomputed) in a `FormattedCharSequence` that carries its width and `Font.PreparedText` (plain/shadowed/outline, keyed by x/y/colours). Obfuscated lines are not cached; `FontSet.resetTextures` bumps a generation. A first version with a global identity map was slower than no cache. `AbstractSignRendererM` also skips the text of a sign side whose plane faces away from the eye by ≥ 0.55 blocks (`ModelCulling.isSignTextHidden`): polygon-offset text pipelines (and their OIT sets) cull back faces, checked at runtime. Glyph quads wind like (0,0),(0,1),(1,1), so the side's facing is `column1 × column0` of the pose |
| `render/entity/ModelCulling.java` + `mixin/render/feature/LevelRendererM.java` | Flags the world's `FeatureRenderDispatcher.prepareFrame`, `LevelRenderer.submitBlockEntities` (for sign text) and the current model's render type. When the pipeline culls (`RenderPipeline.isCull()`), `ModelPartCubeM` skips faces the GPU would cull: `(v1-v0)x(v2-v0)` of the transformed corners points away from every eye position within 0.5 blocks of the origin (camera-relative positions; bobbing moves the eye ~0.1). Degenerate faces are kept. `skipDuplicateQuads()` is true for render types without blending or upload sorting (drawing a quad twice is then idempotent) |
| `render/entity/ModelGeometryCache.java` + `mixin/render/feature/ModelFeatureRendererM.java` | `@Overwrite prepareModel`. Models whose state is `Float` 0 (closed chests, …) are captured once per pose rotation + normal matrix, overlay, tint and `UvMapping` (translation removed, through a recording `VertexConsumer` on the vanilla path), then written with the translation added, the submit's light and precomputed face planes for the back-face test. **Model space entries** (`modelSpaceKey`: currently closed `BookModel.State`, openness 0, where the page flips move nothing) are captured with the identity pose and transformed per write: back faces are tested in model space (eye moved into model space when the pose's linear part is a similarity, else the transformed corners are tested), only corners of written quads are transformed, exact duplicate quads are skipped when allowed. Vertices are written as 4 longs + 1 int. Only rounding differs from vanilla; a temporary check against vanilla's model rendering of 214k book submits found no extra quads and only culled or duplicate missing ones. Max 16 entries per model; models whose capture fails are remembered |
| `mixin/render/vertex/StagedVertexBufferM.java` + `render/vertex/ParallelUploadCopy.java` | Redirects the per-slice `ByteBuffer.put` in `StagedVertexBuffer.uploadDrawsToBuffers` to record the copies; a worker thread and the render thread copy them in 1 MB chunks, starting when the first sorted draw is decoded (sorting overlaps the copy) and joined before the staging buffer is unmapped. Each upload is its own `Job`, so a late worker can't take chunks of the next upload. 1 worker: 3.3 → 1.9 ms for 34 MB; more workers don't help (memory bandwidth) |
| `render/blockentity/RetainedBlockEntities.java` + `ModelSubmitCapture.java` + `mixin/render/blockentity/{ClientLevelM,BlockEntityRenderDispatcherM,BlockEntityDataPacketM,RenderPipelinesM}.java` + `mixin/world/{ShulkerBoxBlockEntityM,DecoratedPotBlockEntityM}.java` + `assets/ruthenium/shaders/core/retained_entity.vsh` | Block entities at rest kept on the GPU. Candidates (exact vanilla classes and renderers): chests (lid idle incl. the other half of a double chest), shulker boxes (`CLOSED`, progress 0 at partial tick 0 and 1), decorated pots (no wobble, or game time - start > duration) and skulls (`getAnimation` constant, dragon/piglin heads not `POWERED`; heads with an owner profile never, their skin loads asynchronously). Others of these types are "moving": left out and re-checked every frame. Per render section (keyed by `SectionPos`), the real renderer runs into `ModelSubmitCapture` (a `SubmitNodeCollector` that records model submits; pots submit their 7 parts as `Model.Simple`; anything else, e.g. crumbling, rejects the block entity); a block entity is kept only if every model has no outline and a render type that maps to a retained pipeline (`entity_solid`/`_z_offset_forward`, `entity_cutout_cull`, `entity_cutout`/`_z_offset`) with `DEFAULT_TEXTURING`. Models are written with all faces, through `ModelGeometryCache` for `Float` 0 states, else with vanilla's `setupAnim` + `renderToBuffer`, relative to the section origin, into one vertex buffer per render type. `LevelExtractorM` skips members; batches are drawn after `featureFrame.executeSolid` in `LevelRendererM` with `PIPELINES` (vanilla entity pipelines + a vertex shader adding `DynamicTransforms.ModelOffset`; registered via `RenderPipelines.optionalPipelines`, so a compile failure just turns it off), the render type's textures (`prepare()` once per type per frame) and its layering transform applied to the model view (the skulls' z offset; `RenderType.state` and `RenderSetup`'s transforms are widened in the access widener). Transforms are written after `prepareFrame` with fresh vectors (`DynamicGpuDataStorageMapped` dedups against the last record by `equals`), and the sequential index buffer is grown there too (not inside a pass). Rebuilt when the section's mesh changes, when the section or a neighbour hits `LevelExtractor.setSectionDirty(IIIZ)` (block and light changes; a double chest's light can come from the neighbour), on chest, shulker box and pot `triggerEvent`, block entity data packets (pot sherds, skull profiles; they can arrive after the block), `ClientLevel.destroyBlockProgress`, and when a moving block entity comes to rest. Sections partly within the view distance (64) use the vanilla path that frame. Since the twelfth session, buffers of pipelines that cull back faces (chests, shulker boxes and pots: `entity_cutout_cull`; sign text: the polygon offset text pipelines) are sorted into `SectionFaceGroups` at build time and each frame draws only the runs of groups that can face the eye (switch `modelBackFaceCulling`; GPU 0.197 -> 0.155 ms at the server scene's static view, which was vertex bound: the same at 960x540). Budget 2048 block entities rebuilt per frame; batches unused for 1200 frames are freed. Cleared on level change, `allChanged` and resource reload. **Signs** (eighth session; `SignBlockEntity`/`HangingSignBlockEntity` with `StandingSignRenderer`/`HangingSignRenderer`, no glowing or obfuscated text on either side): `ModelSubmitCapture` records text submits; a text is retained only if vanilla would put it in the **solid** phase (`SubmitNodeCollection.canRenderAsSolid`: colour alpha 255, no background) with `POLYGON_OFFSET` and no outline, which is why the draw order can change. Its glyphs are prepared through `SignTextCache.prepareText` (as `TextFeatureRendererM` does) and written with `TextRenderable.render` into a part per render type (`renderType.format()`, POSITION_TEX_LIGHTMAP_COLOR), drawn with `TEXT`/`TEXT_GRAYSCALE`: copies of vanilla's `TEXT_POLYGON_OFFSET`/`TEXT_GRAYSCALE_POLYGON_OFFSET` (same state, built from a `Snippet` like `CompressedTerrain`) with `retained_text.vsh`. Extra rebuild triggers: `SignBlockEntity.setText` on the client (`SignBlockEntityM`; the edit screen calls it while typing), `FontSet.resetTextures` (`FontSetM`, clears everything) and a change of `Minecraft.isTextFilteringEnabled` (checked in `beginFrame`, clears) |
| `render/blockentity/InstancedModels.java` + `assets/ruthenium/shaders/core/instanced_entity.vsh` | GPU instancing of models that only differ by pose. Two entry points: (1) the extraction (`LevelExtractorM`) calls `tryAdd(dispatcher, state, …)` after `tryExtractRenderState` for `EnchantTableRenderer`: the real renderer submits into `ModelSubmitCapture`, and a single model with a `modelSpaceKey` (closed book) becomes an instance instead of a submitted render state; (2) `ModelFeatureRendererM.prepareModel` calls `tryAdd(submit)` for models in `POSED_MODELS` (`ArmorStandModel`): after `setupAnim`, a snapshot of all parts' transforms/visibility keys the group (exact for pure `ModelPart` trees; max 32 groups per model) and the mesh is `model.root().render` with the identity pose and CPU culling off. An instance is 3 pose rows + light (52 B, `INSTANCE_FORMAT`, step rate 1). Poses must be orthogonal (rotations, or the `scale(-1,-1,1)` mirror of living entities, where vanilla's normal matrix equals the pose). Instanced copies of `entity_solid`, `entity_cutout` (per-face lighting, no cull) and `entity_cutout_cull` (`PIPELINES`). Per group (model, key, render type, UV mapping, overlay, tint): a model-space mesh from `ModelGeometryCache.writeModelSpace` (identity pose, all faces, duplicates skipped) and a `MappableRingBuffer` for the instances (rotated at the next prepare, so its fence covers the frame that used it). Drawn after `RetainedBlockEntities` with `PIPELINE` (vanilla `entity_solid` + an instanced vertex shader). Cleared with the retained chests. **Closed book fast path** (eighth session, `tryAddClosedBook`, called by `LevelExtractorM` before `tryExtractRenderState`): once one closed book went through the capture path (which records its group and renderer), vanilla `EnchantingTableBlockEntity`s with `open == oOpen == 0` and no breaking progress skip the render state and capture: the pose is built with the same JOML calls in the same order as the extraction + `EnchantTableRenderer.submit` (`translate` x3, `rotateY(-yRot)`, `rotateZ(80°)`; bit-identical, checked on 16.7M books), plus `shouldRender`'s distance test. The light and the `isValid` block state check are cached on the block entity (`EnchantingTableBlockEntityM`, `interfaces/CachedBlockEntityLight`) under a generation stamp that every `setSectionDirty` (and `clear`) bumps (`invalidateLight`); 69M cache hits matched fresh lookups. **Base plates** (twenty-fifth session, switch `armorStandPlates`): the plate, which `setupAnim` turns back by the stand's yaw, is hidden and unturned while the body's snapshot and mesh are taken and drawn from a per-model plate group (`PLATE_KEY`: the plate alone at its scale; instance = submit pose x root and plate `translateAndRotate` without the scale), so stands posed alike share their groups whatever their yaw; read through `ArmorStandModelAccessor`; groups are dropped when the switch changes |
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
| `render/entity/TransparentFaces.java` + `ModelCulling.textureAlpha` + `ModelPartCubeM` | **Transparent face culling** (fourteenth session, switch `transparentFaceCulling`, "Transparent Face Culling"). Model cube faces whose texels all draw nothing are not written by `ModelPartCubeM`'s direct path: alpha below the pipeline's `ALPHA_CUTOUT` (discarded before any color is applied; 8 bit alpha a counts if a < ceil(255 c - 0.01)), or alpha 0 in a `BlendFunction.TRANSLUCENT` pipeline without depth writes (the fragment's alpha stays 0, so it blends to the destination exactly). Only render types of vanilla's `core/entity` vertex and fragment shaders without `APPLY_TEXTURE_MATRIX`, `GLINT` or `DISSOLVE`, `TextureTransform.DEFAULT_TEXTURING`, only `Sampler0`, and a texture whose class is exactly `SimpleTexture` with one mip level and the resource's size (read once with `TextureContents.load`); never with improved transparency (OIT) or a UV mapping (atlas sprites). A face's UV rectangle is widened by one texel per side and every texel is tested at its repeated and its clamped position (nearest and linear filtering). Per render type a `TextureAlpha` (`IdentityHashMap`, last type cached), revalidated against the texture's `GpuTexture` and the texture manager's entry; per cube the masks of its last 4 texture alphas (since the nineteenth session, `a4d2751`, a hash table of up to 64 texture alphas per cube, 128 slots; it starts over when full, so one model drawn with more than 64 textures per frame rescans: the twentieth session's binned downloaded skin culling hit that). Cleared on resource reload and when toggled. Spider and enderman eyes keep only the eye faces; horse markings lose their empty faces. Access widener: `RenderSetup.textures`, `RenderSetup$TextureBinding.location` |
| `render/entity/ModelPoseReplay.java` + `mixin/render/entity/{EntityRenderStateM,model/ModelM,model/PoseAccessor}.java` + `interfaces/{PoseRecordHolder,LayeredModel,ReplayablePart}` | **Layer pose replay** (fourteenth session, switch `reuseLayerPoses`, "Reuse Layer Poses"). Layers that submit a model again with the same render state and pose (villager and zombie villager clothing, spider and enderman eyes, horse markings, collars, armor trims and glint) skip `setupAnim` and the part tree. While the world's features are prepared, every model submit stamps its `EntityRenderState` with a pooled `Record` (two slots per state; frame, state and model identity); a second submit of the same model and state marks the `Model` as layered. From then on the first submit of a layered model of a `net.minecraft.` class records each written part and a copy of its pose (`ModelPartM.compile`), and a later submit of that model and state whose submit pose matches bit for bit (matrix, normal matrix, JOML properties, `trustedNormals`) calls each recorded part's `compile` with the recorded pose (instanced or vanilla path). Exact because vanilla models pose their parts from the state alone (`Model.setupAnim` starts with `resetPose`; checked: every model's `setupAnim` calls it except the copper golem statue's, which isn't an entity state); a temporary check posed 2.8M replayed submits again: 0 mismatches in 23.4M parts |
| `render/entity/LazyVertexBuilder.java` + `ModelFeatureRendererM` | Model submits whose parts become instances get a lazy vertex builder instead of the group's (a render type switch: `RenderType.prepare`, the draw lookup and a new `BufferBuilder`); it fetches the builder (wrapped by the UV mapping) only if a part is written after all, and `ModelPartM.compile` resolves it before writing cubes, so the direct writers see the `BufferBuilder`. Such submits add no empty draws. No switch (same image) |
| `render/entity/EntityRendererCache.java` + `mixin/render/entity/{EntityTypeM,EntityRenderDispatcherM}.java` + `LevelExtractorEntityM.ruthenium$fastCulling` + `interfaces/{EntityRendererHolder,EntityRendererMap}` | **Renderer kept on the entity type and cheaper entity culling** (fifteenth session). `EntityRenderDispatcher.getRenderer(Entity)` and `getRenderer(EntityRenderState)` look non-player renderers up in the dispatcher's `ImmutableMap` (culling, extraction, submission: up to three times per entity and frame); a `@WrapOperation` on that `Map.get` returns the renderer kept on the `EntityType` together with the map it came from (only while the dispatcher holds that map; only for `ImmutableMap`s). Each entry also classifies the renderer's culling: plain if no class below `EntityRenderer` (or `LivingEntityRenderer` for living ones) declares `shouldRender`, `affectedByCulling` or `getBoundingBoxForCulling` (reflection, once per type). A `@WrapOperation` on `isEntityVisible`'s `EntityRenderDispatcher.shouldRender` call answers for plain renderers (switch `fastEntityCulling`, "Fast Entity Culling"; players and mannequins excluded): `entity.shouldRender` (distance), then the interpolated box (`getInterpolatedBoundingBox`, virtual) through the frustum accessor without the inflated box allocations. Living renderers widen the box by another half block for a dragon head; instead of reading the head slot, the box with the widening is tested first (inside the frustum: visible either way; outside: hidden either way) and only when it crosses the edge the box without (visible: visible either way, else vanilla's path). Leash holders and non-finite boxes take vanilla's path. The frustum test is monotone in the box and the widened coordinates are computed in vanilla's `inflate` order. Checked: 16.8M answers equal vanilla's (AI on), 0.3% undecided |
| `render/entity/EntityLight.java` + `mixin/render/entity/{EntityRendererLightM,EntityLightHolderM,LayerLightSectionStorageM}.java` + `interfaces/CachedEntityLight` | **Entity light kept across frames** (fifteenth session, switch `entityLightCache`, "Keep Entity Light"). `EntityRenderer.getSkyLightLevel`/`getBlockLightLevel` read `Level.getBrightness` at the light probe position for every entity and frame; a `@WrapOperation` on those two calls returns the values kept on the entity (`Entity` fields: level, packed position, generation, block and sky light + 1) while level, position and `EntityLight.generation()` are unchanged. Readers see light through each `LayerLightSectionStorage`'s visible map, which only changes when `swapSectionMap` publishes a new copy (writes copy each section into the updating map first); the client's storages (`chunkSource instanceof ClientChunkCache`) bump the generation at `swapSectionMap` HEAD when sections changed. Renderer overrides (fire, glow frames, displays) still run around the kept values. Render thread only. Checked with `bench.lightEvents`: 29.4M kept values equal fresh lookups (2.35M differ with the invalidation disabled) |
| `world/CollidableEntities.java` + `mixin/world/{EntitySectionM,LevelEntityCollisionsM,LevelEntityGetterAdapterAccessor}.java` + `interfaces/CollidableEntityCount` | **Entity collision searches skip sections without collidable entities** (sixteenth session, switch `fastEntityCollisions`, "Fast Entity Collisions"; idea from Lithium's entity class groups, own code). `getEntityCollisions` (every moving entity's `collide`, every `noCollision`: experience orbs twice per tick, interpolated movement, player pose checks) only accepts entities whose class overrides `canBeCollidedWith(Entity)` (boats, shulkers, happy ghasts, modded ones; checked once per class by reflection). Each `EntitySection` counts those (`add`/`remove` injects, always on), and `Level` overrides the `EntityGetter` default method with vanilla's code whose search skips sections with a count of 0, through the section storage's own `forEachAccessibleNonEmptySection` and `EntitySection.getEntities` (same entities, same order), plus vanilla's dragon part loop. Only for a null source or a source using `Entity.canCollideWith` (boats and minecarts override it to collide with pushable entities: vanilla search). Checked: 800k searches on the client and the integrated server with mob AI, 0 differences. Mod compatibility: a mod injecting into `EntityGetter.getEntityCollisions` or `Entity.canBeCollidedWith` (instead of overriding it) is bypassed (Lithium touches the former) |
| `world/AnimateTick.java` + `mixin/world/{ClientLevelAnimateTickM,LegacyRandomSourceAccessor,EnvironmentAttributeSystemAccessor,EnvironmentValueSamplerAccessor}.java` | **Faster animateTick** (sixteenth session, switch `fastAnimateTick`, "Fast Block Animation Tick"). `ClientLevel.doAnimateTick` runs for 1334 random positions per tick. Its `nextInt` calls on the level's `LegacyRandomSource` (exact class) step the seed with `getPlain`/`setPlain` instead of a compare-and-set per call (same sequence; the CAS only detects use from two threads). Its ambient particle lookup (`environmentAttributes().getValue(AMBIENT_PARTICLES, pos)`: a fuzzy-zoomed biome lookup) is answered with the attribute's no-biome value (computed once per `animateTick`) when the biome palettes of the chunk sections holding the zoom's 2x2x2 quart corners (the only biomes it can pick; y clamped as `ChunkAccess.getNoiseBiome` does) contain no biome whose attribute map has an `AMBIENT_PARTICLES` entry (vanilla: four nether biomes), cached per section per call. Only while the attribute's sole positional layer is the biome lambda of `EnvironmentAttributeSystem` (checked through accessors on the sampler's layers); unloaded chunks take vanilla's lookup. Checked: 24M random results and seeds, 3.8M ambient values in the entity and terrain scenes, and the biome vanilla's zoom chose was always in a checked palette |
| `mixin/world/AbstractBoatM.java` | **Boat ticks** (sixteenth session, switch `fastBoatTicks`, "Fast Boat Ticks"). In `getGroundFriction`, full cube collision shapes (`== Shapes.block()`) skip the moved shape and the `joinIsNotEmpty` with the bottom slice: separated by more than 1e-6 on an axis = false (vanilla's own early out uses 1e-7), overlapping by more than 1e-6 on all axes = true, anything else vanilla's join (redirects on `VoxelShape.move` and `Shapes.joinIsNotEmpty` there). The push search in `tick` (`getEntities(this, box, EntitySelector.pushableBy(this))`, which on the client only accepts local players) asks `ClientLevel.getPushableEntities` on the client, as vanilla's own living entity push does. Checked: 200k cube joins and 6.1k pushes equal to vanilla's |
| `world/SupportingBlocks.java` + `mixin/world/{EntitySupportingBlockM,BlockChangeTrackingM,LevelChunkSectionM}.java` + `interfaces/SectionChangeStamp` | **Supporting block kept for resting entities** (sixteenth session, switch `fastSupportingBlocks`, "Keep Supporting Blocks"). `Entity.checkSupportingBlock`'s `findSupportingBlock` (a `BlockCollisions` sweep of a thin slab under the entity: orbs every tick, items every fourth, entities moved by server packets) is answered from the last result on the client while the slab, the entity's position and the chunk sections of the swept region (box +-1 block, at most 8 sections, the same objects) have the same change stamp. Each `LevelChunkSection` has a stamp, bumped by `LevelChunk.setBlockState` on client levels and for every section by `LevelChunk.replaceWithPacketData` (chunk data is read into the existing sections). Results are kept only when every block of the region has a collision shape that is neither dynamic (`Block.hasDynamicShape`) nor context dependent (no class below `BlockBehaviour` declares `getCollisionShape(BlockState, BlockGetter, BlockPos, CollisionContext)`), so the entity's collision context can't change it. A first version with one level-wide stamp hit ~65% (the integrated server sends ~4.5 block updates per second: grass and dirt random ticks); per section 99.5% at rest. The kept state lives in an object created on an entity's first search (`SupportingBlocks.Kept`; 13 fields on every `Entity` made the server scene's armor stand ticks ~0.02 ms slower per tick). Checked: 220k kept answers equal to a fresh search (gravity scene, with and without AI) |
| `world/EnchantingTableBatch.java` + `LevelM`, `RebindableTickingBlockEntityWrapperM`, `BoundTickingBlockEntityM` (`ParkableTicker.ruthenium$tryBatch`, `BlockEntityTicker.ruthenium$getValidState`) | **Enchanting tables ticked in one loop** (seventeenth session, switch `batchTableTicks`, "Fast Enchanting Table Ticks"; needs parking and `cacheTickerStates`). On the client, `LevelM`'s loop parks the wrapper of a vanilla `EnchantingTableBlockEntity` whose bound ticker has a validated state of the exact `EnchantingTableBlock` class into the level's batch (arrays of wrappers, tables and packed positions, in joining order). After the list, the batch runs vanilla's guards (removed: the wrapper leaves; `shouldTickBlocksAt`, `hasLevel`, the world border) and, unless a player passing `EntitySelector.NO_SPECTATORS` (positions read once per tick) is within the 3 blocks `bookAnimationTick` searches, a statement for statement copy of its branch without a player; otherwise vanilla's `bookAnimationTick`. A wrapper leaves on rebind or unpark (`remove(slot)`; slots move as the batch compacts). Order among tables only matters for their shared unseeded random source, used only with a player near. Mod compatibility: mixins into `bookAnimationTick` don't run for tables without a player near |
| `SectionFaceGroups.sortByPlanes`/`visiblePlaneRuns` (used by `RetainedBlockEntities`) | Retained batches are sorted by facing and, within a facing group, by plane (nearest to the faced side first; stable, so equal plane quads keep their order); each frame draws the prefix of each group whose planes (with the 0.25 margin) the eye can be in front of, merged into at most 7 runs. Quads within a 1e-5 tilt of an axis and 1e-4 blocks of a plane count as in it (rotated models), with their backmost coordinate as the plane. Terrain keeps `sort` and its packed per-group draws. Since the twenty-fifth session the keys are sorted with a stable LSD radix sort of bits 20..55 (the same order as `Arrays.sort`: keys arrive in quad order) and the quads are gathered straight into the buffer that is uploaded (`sortByPlanes(mesh, target)`) |
| `RetainedBlockEntities` (seventeenth session) | Also retains lecterns, bells (not while `shaking`; `BellBlockEntityM` marks the section dirty on `triggerEvent`) and copper golem statues. **Change checks** (switch `retainedChangeChecks`): `LevelExtractor.setSectionDirty` now calls `markBlocksChanged`; a batch remembers its block entity objects and block states and its members' light (`LightCoordsUtil.getLightCoords` at the block entity and, for double chests, at the other half), and a block or light change, or a new compile of the section, rebuilds it only if any of these differ (`isUnchanged`). Neighbours are only checked when they hold a double chest whose other half is in another section (`linked`). Block entity events still force rebuilds (`markDirty`, with neighbours). Batches are freed after 1200 frames *and* 30 s unused, or 1200 frames when more than 64 blocks beyond their view distance. Mod compatibility: a mod changing what a retained block entity renders without a block entity event, block state or light change (e.g. a field set directly on the client) leaves the batch stale until the next rebuild |
| `InstancedModels.tryAddClosedBook` | The closed book's constant 80 degree tilt: sine and cosine computed once with JOML's functions and applied with `rotateTowardsXY`, which `rotateZ` calls on a matrix that isn't a pure translation (pure translations keep `rotateZ`); bit-identical |
| `render/text/EntityTextCache.java` + `mixin/render/text/{EntityRendererNameTagM,EntityNameTagHolderM,MutableComponentM,SubmitNodeCollectionNameTagM}.java` + `interfaces/{KeptNameTags,NameTagLayout}` | **Name tag text kept** (nineteenth session, switch `nameTagTextCache`, "Keep Name Tag Text"). `EntityRenderer.extractNameTags` builds the name tag (`getNameTag` -> `getDisplayName`: team formatting, hover event, insertion) and the below-name line anew every frame; a `@WrapOperation` on both replaces a result that is `equals` (deep) to the component the entity showed last (`Entity` fields) by that kept component. `SubmitNodeCollection.submitNameTag` then measures it (`Font.width`) and orders it (`getVisualOrderText`: ICU bidi and Arabic shaping); the width is kept on the `MutableComponent` (per font, language and `SignTextCache` generation), vanilla's own visual order cache applies to the kept instance, and the visual order is wrapped once in a `SignTextCache` line (`NAME_TAG`), so `TextFeatureRendererM` reuses the prepared glyphs of both submits. Equal components lay out the same; obfuscated text is still laid out every time |
| `render/text/SignTextCache.java` (nineteenth session) | Lines keep up to 4 prepared variants (kind, x, y, colour, background, includeEmpty) instead of one per kind; lines know their source (`SIGN` always on, `NAME_TAG`, `TEXT_DISPLAY` follow their switches) |
| `mixin/render/text/TextDisplayRendererM.java` | **Text display lines kept** (nineteenth session, switch `textDisplayTextCache`, "Keep Text Display Text"): `TextDisplayRenderer.splitLines` (called only when the display's text or line width changes, `TextDisplay.cacheDisplay`) wraps each line in a `SignTextCache` line (`@ModifyArg` on `CachedLine.<init>`) |
| `render/entity/TextDisplayCulling.java` + `mixin/render/entity/TextDisplayAccessor.java` + `LevelExtractorEntityM.ruthenium$fastCulling` | **Text displays culled by their text** (nineteenth session, switch `textDisplayCulling`, "Cull Text Displays"). Vanilla culls displays only with a width and height set (`Display.noCulling`); hologram plugins leave them at 0, so every text display in range was extracted, submitted and its text written every frame. After vanilla's test passes, a `TextDisplay` without passenger offset, name tag, shadow or glow, with a constant transformation and split text (`clientDisplayCache`), is tested as a sphere: the text rectangle (widest line x line count, +1 px background, +8 px glyph margin, 1 px depth) x 1/40, times the transformation (`|t| + ||A||_F |v|`), any billboard rotation; box widened by 0.5 + 2% of the distance (bobbing). Off while the camera entity is hurt/dying or under nausea/portal |
| `mixin/render/entity/LivingEntityRendererLayersM.java` | **Empty armor stand layers skipped** (nineteenth session, switch `emptyArmorStandLayers`): `LivingEntityRenderer.submit`'s `shouldRenderLayers` answers false for vanilla's `ArmorStandRenderer` with exactly its four vanilla layers when the state's equipment, hands, head item and worn head are all empty (the layers draw nothing; the model's own submit poses the model again when prepared) |
| `render/sky/CloudFaceTexture.java` + `mixin/render/sky/CloudRendererM.java` + `assets/ruthenium/shaders/core/clouds.vsh` | **Cloud faces in a texture** (twenty-fifth session, part 2, switch `cloudFaceTexture`, "Cloud Face Texture"): `CloudRenderer.buildMesh` writes into a scratch buffer (`@ModifyArg`), which is copied into vanilla's mapped texel buffer (so the switch can change at any frame) and uploaded with `writeToTexture` into an R8_SINT 2D texture (byte i at (i % 1024, i / 1024)) when the mesh rebuilds; `render(RenderPass, RenderPipeline)` draws with copies of `CLOUDS`, `FLAT_CLOUDS` and the OIT sets whose `CloudInfo` layout binds the texture (`COMBINED_IMAGE_SAMPLER`), and `ruthenium:core/clouds` reads it with `texelFetch(isampler2D)`. Turning the switch on forces a mesh rebuild so the texture is current |
| `render/ScreenshotReadback.java` + `mixin/render/ScreenshotM.java` | **Fast screenshots** (twenty-fifth session, part 2, switch `fastScreenshots`, "Fast Screenshots"): full size `Screenshot.takeScreenshot` of 4 byte formats copies the read back frame two pixels per long (alpha ORed, rows flipped) instead of vanilla's per pixel `getInt`/`setPixelABGR`; downscaled shots keep vanilla's path |

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
**Every Gradle command, compiles included, runs under the machine-wide GPU lock** (`scripts/gpulock.py`, since `681f1f2`), so only one
Minecraft instance runs at a time, across worktrees and agents:
```bash
python scripts/gpulock.py ./gradlew --console=plain benchRuthenium -Pbench.scene=mixed
python scripts/gpulock.py sh scripts/run-ab.sh 3 mixed-dev/x "benchRuthenium" "-Pvm.x=true" ""
```
It holds an OS file lock in `%LOCALAPPDATA%/ruthenium-gpu.lock` (released by the OS however the holder exits; holder details in
`ruthenium-gpu.lock.info`), waits until no Minecraft JVM is running, then runs the command with `RUTHENIUM_GPU_LOCK` set. Bench tasks refuse
to start without it; `-PbenchNoLock` overrides (don't use it while anyone else may run the game). Never start two Gradle commands at once,
and don't run Gradle in the background while running another; the idle Gradle daemon is fine. The plain commands below are shown without
the wrapper for brevity.
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
- **Vulkan results before the vsync change (the commit after `9d389c5`, 2026-10-30) are 13-24% low, so don't compare across it.** Bench tasks now write
  `enableVsync:false` into the run directory's `options.txt` before launch. Before that, the first swapchain was FIFO and Vulkan stayed in a
  slower present path for the whole run (twenty-eighth session). `-PbenchStartVsync` starts with vsync on as before, for comparisons with
  older results. OpenGL is unaffected.
- **Run everything through `scripts/gpulock.py`** (section 6.3). A second game instance (another worktree, another agent, a leftover
  `KnotClient`) changes GPU bound numbers and can collide on the world's `session.lock`; the lock also waits for leftover game JVMs.
- **Move the results of plain runs out of `results/` before `run-ab.sh`**: it moves every `ruthenium-*` result it finds into the side it
  just ran (twenty-fifth session: three close-up runs landed in an A/B's side a and had to be moved out by timestamp).
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

**Twenty-fifth session tools:**
- The mixed scene has `bench.toggleField` (`ToggleAb` after the particle phase, once the particles are gone; resets to the static yaw,
  pans from yaw 0) and frozen close-ups of the armor stands (`bench.closeShots`, `bench.closeToggle`, which there takes comma-separated
  fields, e.g. `instancedModels,instancedParts` for vanilla's path; clouds off; 13 extra stands summoned for them). Neither changes the
  measured phases.
- `scripts/patches/s25-diagnostics.patch` (against `681f1f2`): `-Pbench.slowFrameMs=N` logs every frame slower than N ms with its profiler
  tree (`SlowFrameProbe`), per-packet timing, destroy queue contents, retained build timers per phase, `InstancedModels` counters,
  `-Pjvm.ruthenium.s25.rebuildEvery=N` (warm retained builds). `scripts/patches/s25-radix-check.patch`: the radix sort's check mode.
- **The mixed static view is CPU bound** (unlike the old server scene): CPU savings show in its fps.
- Part 2: `bench.cloudShots=N` (mixed scene, after the phases, ticks frozen): N shots each of 8 cloud views (`cloud<k>-<n>`: fancy from
  below, inside the layer and above, fast from below and above, improved transparency fancy below and inside, fast below), toggled by
  `bench.closeToggle` like the close-ups. `scripts/patches/s25-screenshot-check.patch` (against `5307cfd`):
  `-Pjvm.ruthenium.screenshotCheck=true` runs vanilla's screenshot loop on the same read back pixels and compares, and the patch times every
  readback callback on both paths. **`ToggleAb` in the mixed scene with `armorStandPlates=false` is only valid for ~60 s after the scene
  is built** (about 3 static rounds): the server's rotation sync then re-sends the stands' yaws byte-quantized, and the instanced groups
  hit their cap (section 7, twenty-fifth session, part 2).

**Twenty-fourth session tools:**
- **In-launch A/B of a switch** (`ToggleAb`): `-Pbench.toggleField=<Config field>` (entity and players scenes) adds, after the scene's
  phases, `bench.toggleRounds` rounds (default 8) of the static view and of the pan (`bench.togglePhases=static,pan`), each round both
  settings in ABBA order: the field is set by reflection (no `clear()`), `bench.toggleWarmup` seconds (1) discarded, then
  `bench.toggleSeconds` (3) of the static view or a full pan of `bench.togglePanSeconds` (5). Logs each round (`static-on-3`, ...; also in
  the JSON) and a summary line `Toggle A/B static <field>: ... off/on per round [...] (median, mean, k of n higher)`. Both settings run on
  code compiled for both, so it measures the switch's direct cost, not JIT shape. Resolution: rounds spread ~+-1% (static) and the first
  one or two rounds drift (warm-up); a 2.4% effect showed in 10 of 10 rounds, a ~0.5% one in 7 of 10. With `-Pbench.lightProfile=9` each
  round's profiler tree is logged; a small script that parses the `Light profile of <phase>-<on|off>-<n>` blocks and prints the mean
  ms/frame difference per section path, largest first, found the cause in one launch (`profdiff.py`, kept as
  `scripts/profdiff.py`). Use 2 s scene phases (`-Pbench.staticSeconds=2 -Pbench.panSeconds=2`); ~1m45s per launch with static rounds only.
- **The light profiler splits `submit` further** (`VulkanCommandEncoderProfileMixin`): `awaitGpu`, `endCommandBuffer`, `queueSubmit`
  (`VulkanQueue$Submission.close`, i.e. `vkQueueSubmit2`), `destroyQueue`, `poolReset`, `transientEnd`, `transientBegin`.
- `-Pbench.entityClouds=true`: fancy clouds in the entity scene (off by default; screenshots of different runs then differ).
- **Driver cost to know:** on the NVIDIA Vulkan driver (RTX 4060 Laptop) a submit that uses any texel buffer costs ~20 us more in
  `vkQueueSubmit2` (0.042 -> 0.062 ms per frame), the same for 1 or 5 draws per pipeline and with device-local buffers; vanilla's clouds
  pay it. Prefer textures (`texelFetch`) or uniform buffers for per-frame data read by shaders.

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
- Since the twentieth session the downloaded skins go through vanilla's `SkinTextureDownloader.downloadAndRegisterSkin` from local
  copies in `bench-skins/` of the run directory (`processLegacySkin`, then the downloader's `DynamicTexture`; nothing is
  downloaded). `bench.skinVariants=N` (default 18 = the 9 defaults, wide and slim; player i gets variant i % N, model wide for
  even variants) adds tinted copies whose six outer layer regions are each kept, cleared, faded below the cutout (alpha 20),
  semi-transparent (alpha 100) or checkered, and every tenth of them (wide) is a legacy 64x32 skin (variants 18, 28, 38, ...;
  38 is in the third row of `close0`). `bench.emptyOuterLayers=true` clears every outer layer. `bench.skinEdit=true` edits the
  skins of players 7 and 8 after the close-ups (in place, and through `setPixels`; both uploaded) and takes the close-ups again
  (`close<k>-<n>-<run>-edited`).
- Since the twenty-second session: `bench.playersHurt=true` sets every third player's hurt time during the close-ups (the red
  overlay of a hurt flash; frozen ticks keep it), and `bench.playersGlass=true` places, while ticks are frozen for the close-ups,
  two stained glass walls through the first row in front of `close0` and water around its players at x 4.5 to 7.5 (no fluid spread
  or suffocation while frozen; removed afterwards). Frozen shots of one setting differ by the glint and the arm; to find what a
  switch changes, count pixels that are equal in all shots of each setting but differ across them (`java scripts/StableDiff.java
  2 map.png shot0 shot1 ...`, shots 0, 2, ... one setting and 1, 3, ... the other; with `closeShots=8` the glint left none).
- Since the twenty-third session: `bench.labelVariety=true` adds a wall of special labels at x 34-47 (shadowed, see-through, half
  opacity, red and no background, underlined / struck / italic, obfuscated, CJK and accented, wrapped and aligned, fixed and scaled
  text displays, stacked and overlapping displays and hologram stands, a named pig, stained glass in front) and a third close-up
  (`close2`) of it; it changes the scene's numbers. Obfuscated text differs stably between the two settings of any `closeToggle`
  (its random glyphs depend on the frames drawn between shots): ignore the "Obf" label's pixels.

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
  exact (the reason first given here, that vanilla draws translucent text backgrounds before the translucent models phase, is
  wrong: the twenty-third session found the nameTags and texts phases drawn *after* the translucent models; moving the bodies would
  still change their order against the other translucent models and sorted geometry). An exact path would keep each player's draw in its sorted
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

#### After the merge (`33bff56`)
Checks on the merged tree in the main worktree (`results/s19-merge/`; another agent's runs serialised by the GPU lock):
- Every scene passes and its screenshots look right: players lobby (3 runs, `staticSeconds=10`) static 328.8 fps, pan
  1530.4 (part 2's final numbers in its worktree: 326.4 / 1438.5); the four part 2 switches off (2 runs) 275.7 / 991.1, so
  they apply on the merged tree; mixed (3 runs) static 1622.9, pan 1661.8, particles 1319.6 (part 1: ~1514-1526 static);
  entities static 333.0, pan 1221.7; block entities static 1387.2, pan 1868.3.
- Pre-merge `26.3` (`5a90842`) vs the merge, 2 pairs each: block entity scene static 1372.1 vs 1368.8, pan 1864.3 vs 1861.5;
  mixed static 1593.5 vs 1661.6, particles 1275.6 vs 1336.7, pan 1667.3 vs 1665.5 (both pairs higher with the merge; most
  likely `emptyArmorStandLayers` on the scene's 296 empty armor stands, not verified). The mixed scene ran higher today than
  in either part's runs (1525 / 1561 static) on both builds, so compare it only within one session.
- Open: the block entity scene runs at ~1360-1390 static / ~1860 pan on `3c04a06`, `5a90842` and the merge, against
  1750-1775 / ~2600 at the end of the seventeenth session (same blocks, same chat line in the shot, render thread 0.23 vs
  0.25 ms, so not CPU). Part 1's parity runs (`s19-dev/par-be-on`, with signs) were already low. Not from the merge; not
  bisected (eighteenth session commits or the machine).
- **Resolved (twenty-second session): the machine, not the code.** The seventeenth session's last commit (`8dee171`, built as
  it was: `benchVulkanMod`, fast-ip-ping loaded, `vulkanmod` run directory) runs as slow as `26.3` HEAD (`111db82`) now, both
  built and run in turn in one worktree, 3 ABBA pairs (`results/s22-be-old-new`, warm-up pair in `s22-be-warm`): static
  1371.9 vs 1369.2 fps, pan 1851.1 vs 1851.7 (the seventeenth session: 1763.1 / 2600.7 with the same arguments). The GPU frame
  probe (`s22-be-gpuframe`) reads static 0.72 ms (0.543 on 28/10, `v17-planes-be`) and pan 0.52 (0.351): **+0.17 ms per frame
  in both views**, while the probed main pass parts are unchanged (solid terrain 0.124 vs 0.132, retained 0.32 vs 0.30
  static, 0.115 vs 0.113 pan), so it is a fixed per-frame GPU cost outside them (the probe's interval also contains the wait
  on the swapchain acquire, which the blit's command buffer waits on with `ALL_COMMANDS`; GPU work or presentation, not
  told apart). The last fast run was 28/10 16:19, the first slow one 22:34; in between the machine was restarted by the
  user (21:21, no Windows update then) and the repository moved to `D:\dev\Ruthenium`. Same driver (617.42) before and
  after; now 2490-2505 MHz at 95-97% GPU utilization, ~85 W of 125 W, AC, Balanced + "Best performance" overlay (no clocks
  were recorded before, and the Lenovo thermal / GPU overclock / MUX state can't be read without admin). The 29/10 05:39
  update (KB5127936) came after the slowdown. Details: `results/s22-be-old-new/environment.txt`. Consequence: block entity
  scene numbers from before 28/10 21:21 (and any other GPU bound view's) don't compare with later ones; compare within a
  session. Which setting changed with the restart is unknown (candidates: the presentation path, e.g. independent flip vs
  composed, the Lenovo GPU mode or overclock, an NVIDIA app setting).
- Measured in the twentieth session (section 7, "Twentieth session"): the -20% is gone (players 477.3 -> 484.5 fps static).

### Twentieth session (2026-10-29)
Section 8 item 1b's first bullet: transparent face culling with many skins. One agent on `26.3` (main tree), every Gradle
command under the shared GPU lock. Results in `results/s20-*` and `results/s20-dev/`, logs `build/bench-logs/s20-*.log`.
**Net code change: none in `src/main`** (both changes were binned, `scripts/patches/s20-downloaded-skins.patch`); the
players scene got new skin options.

**1. Part 1's per texture mask cache (`a4d2751`) in the players scene** (`s20-tf-resource-*`, `-Pbench.playerSkins=resource
-Pbench.staticSeconds=10`, switch `transparentFaceCulling` off vs on, 5 pairs each, tree `e20b43e`). The -20% of the nineteenth
session (476.6 -> 382.1 static, measured without `a4d2751`) is gone; the culling is now neutral to slightly positive:

| resource skins | static | p99 low | render thread | pan | pan p99 | pan render thread |
|---|---|---|---|---|---|---|
| players kind, off | 477.3 | 300.1 | 2.03 ms | 1633.9 | 431.3 | 0.51 ms |
| players kind, on | 484.5 (3 of 5 pairs) | 291.4 | 2.01 ms | 1645.1 | 430.2 | 0.50 ms |
| lobby, off | 326.7 | 197.4 | 2.97 ms | 1519.7 | 305.5 | 0.56 ms |
| lobby, on | 331.6 (3 of 5) | 200.3 | 2.94 ms | 1516.8 | 309.4 | 0.55 ms |

Temporary counters (single runs): 1726 cube tests per frame for 150 players (~11.5 per player), 0 mask rescans, 878 faces
skipped per frame (of ~10.8k body faces).

**2. Downloaded skins (`DynamicTexture`) tested too: built, exact, measured, binned.**
- *What the GPU samples* (26.3 sources): `SkinTextureDownloader.downloadAndRegisterSkin` reads the PNG (or its local copy),
  runs `processLegacySkin` for skins (64x32 converted to 64x64: top half copied, bottom half cleared, arms and legs mirrored
  into the new slots, `doNotchTransparencyHack` on the hat if it is fully opaque, then `setNoAlpha` on the head, body/arms/legs
  base regions: (0,0)-(32,16), (0,16)-(64,32), (16,48)-(48,64)), then `registerTextureInManager` makes a `DynamicTexture`
  whose constructor uploads `pixels` (RGBA8, 1 mip, repeat nearest sampler) and registers it. Capes and elytra take the same
  path without `processLegacySkin`. Vanilla never touches a skin texture again (`SkinManager`'s texture caches keep every
  skin of the session; nothing releases them).
- *Changes to the pixels* (`DynamicTexture`): `getPixels()` hands out the `NativeImage` (editable in place), `setPixels`
  replaces it (closing the old one), `upload()` writes `pixels` to the GPU texture. The GPU only changes at `upload()` (or a
  direct `writeToTexture` on `getTexture()`, which nothing in vanilla does for skins). So the design was: a `@ModifyArg` on the
  `TextureManager.register` call in `SkinTextureDownloader.lambda$registerTextureInManager$0` (`require = 0`) marks the texture
  and copies its alpha (its constructor just uploaded the processed pixels); a `DynamicTextureM` `upload()` RETURN inject copies
  the alpha again for marked textures (a new array each time); `TransparentFaces.load` uses that copy for marked
  `DynamicTexture`s, and `TextureAlpha.isCurrent` (now also checked when the render type repeats) compares the array's identity,
  so a re-upload replaces the masks. Edits in place or `setPixels` without an upload change nothing on the GPU and nothing in
  the copy. Exact for anything going through `upload()`; a mod writing the GPU texture directly would leave stale masks
  (compatibility trade-off, moot now). Memory: 4 KB per skin (the alpha copy; vanilla keeps 16 KB of pixels plus the GPU
  texture per skin).
- *The per cube cache of `a4d2751` doesn't cope with many skins*: a cube's table holds 64 texture alphas (128 slots, half full)
  and starts over when full, and every `PlayerModel` cube of a model type sees every skin of that type each frame. With 150
  distinct skins (75 per model) the counters showed 1500 of 1726 cube tests per frame rescanning texels, static 356.0 fps
  against 490.2 with 18 skins (single runs). Replaced in the attempt by **masks kept per texture alpha**: each `TextureAlpha`
  keeps an open addressing table of cube ids (`long`, from a counter, so never reused; 16 slots, grows to 2048, starts over
  past 1024 cubes) and masks; the cube keeps only its id. 0 rescans with 150 skins, static 473.5.
- *Bench support* (kept, `4798bd6`, `970dcc5`; section 6.10): the scene's downloaded skins now go through vanilla's downloader
  from local copies, `bench.skinVariants=N` adds tinted variants with cleared, faded (alpha 20, below the 0.1 cutout),
  semi-transparent (alpha 100) or checkered outer layer regions and legacy 64x32 skins, `bench.emptyOuterLayers=true` clears
  every outer layer, `bench.skinEdit=true` edits and re-uploads two skins after the close-ups and takes them again.
- *Parity* (`s20-parity`, 150 variants incl. the legacy variant 38 in the third row, `closeShots=4`,
  `closeToggle=transparentFaceCulling`, `skinEdit=true`): on vs off differs by 9481 / 3434 px at threshold 1 (players /
  holograms close-ups), same-setting pairs by 8788-11073 / 3013-3405, and the diff maps show the same places (enchanted
  leggings' glint, glinting items, the first-person arm's outline). After the edit and re-upload (which changed 58k px at
  threshold 12: the formerly empty pants and sleeves of players 7 and 8 now drawn) on vs off still match (7549 vs 9126-9274 px
  same-setting), so the masks followed the upload.
- *Result*: no gain. 5 pairs each, switch off vs on, `staticSeconds=10`, players kind unless noted:

| downloaded skins | static off -> on | p99 low | render thread | pan off -> on | pan p99 | pan render thread |
|---|---|---|---|---|---|---|
| 18 (defaults), `s20-dyn-players` | 486.1 -> 479.4 (2 of 5 lower) | 303.6 -> 295.9 | 1.99 -> 2.01 ms | 1651.3 -> 1641.7 | 445.7 -> 433.8 | 0.47 -> 0.49 ms |
| 150 variants, `s20-dyn-players150` | 482.1 -> 474.8 (4 of 5 lower) | 289.7 -> 278.9 | 1.99 -> 2.02 ms | 1652.3 -> 1644.0 | 435.1 -> 432.5 | 0.48 -> 0.48 ms |
| 150, empty outer layers, `s20-dyn-players150-empty` | 489.1 -> 489.6 (4 of 5 higher) | 284.3 -> 298.3 | 1.98 -> 1.99 ms | 1643.3 -> 1652.2 | 443.5 -> 446.1 | 0.49 -> 0.48 ms |
| lobby, 18, `s20-dyn-lobby` | 323.8 -> 325.1 | 184.7 -> 190.2 | 2.99 -> 2.99 ms | 1522.8 -> 1521.2 | 303.7 -> 302.9 | 0.55 -> 0.54 ms |

  Elimination (`s20-dev/elim-*`, 150 variants, 4 pairs, a temporary property that looked the masks up but skipped nothing):
  off 477.7 (2.02 ms) vs lookups only 475.8 (2.05 ms); lookups only 477.0 (2.03) vs on 475.8 (2.03). The lookups cost at most
  ~0.03 ms for 150 players and the skipped faces save about as much.
- *Why so little is skipped*: faces skipped per frame were 744 (18 defaults), 550 (150 variants) and only **1485 with every
  outer layer empty** (~10 per player, where an empty outer layer has 36 faces). `TransparentFaces` widens a face's UV
  rectangle by one texel and also tests the repeated position, and the skin layout puts every outer layer region next to the
  opaque base layer (rows 16, 32 and 48; the hat beside the head; the top rows wrap to row 63, the left leg's base), so almost
  no outer layer face has a fully transparent one-texel border. The margin is needed for exactness even with nearest
  sampling: a pixel centre exactly on a face's edge samples the next texel (u = b/w is texel b). With the base layer forced
  opaque on download, a downloaded skin can only lose a handful of faces. Resource skins are no different in kind (878 per
  frame for the defaults).
- *The per texture alpha tables alone* (commit A/B, `970dcc5` vs a WIP commit with only the table change, `s20-c3-*`):
  entities with `-Pvm.instancedParts=false` (5 pairs) static 230.0 -> 226.4 (render thread 4.24 -> 4.32 ms, 3 of 5 lower), pan
  1118.2 -> 1101.6 (4 of 5 lower); entities defaults (3 pairs) 337.1 vs 347.9 (2.86 vs 2.81 ms), pan 1237.8 vs 1229.3; lobby with
  resource skins (3 pairs) 329.8 vs 329.7, pan 1522.4 vs 1521.2; mixed (3 pairs) static 1607.5 vs 1646.6, pan 1669.3 vs 1676.5,
  initial 1148.4 vs 1140.2, particles 1316.3 vs 1298.0. A gain only past 64 textures per cube, which no vanilla case reaches
  without the downloaded skins, and slightly slower in the villager clothing case. Binned with the skin part; both are in
  `scripts/patches/s20-downloaded-skins.patch` (applies to `970dcc5`/HEAD with `git apply`). Worth revisiting only together
  with a resource pack or mod that gives one model >64 textures (random mob textures), or if the margin rule changes.
- Not verified: real downloaded skins from Mojang's servers (the bench's local copies take the same code path after the
  download), capes and elytra through the downloader (their render types are instanced or not tested anyway), the OpenGL
  backend, and mods that write skin textures directly.

For the next agent (item 1b, reuse last frame's quad sort order for still player bodies): `TransparentFaces` is unchanged from
`a4d2751`, so a player body draws all 72 faces minus the few the culling skips (~5 for the default skins, none of which
change from frame to frame for a still player); the skipped set depends only on the skin and the cube, so the decoded
centroids of a still player are bit-identical frame to frame with the culling on or off. Use `-Pbench.playersKind=players`
(150 players, `staticSeconds=10`); with `skinVariants=150` every player has its own skin (and draw) as on a real server. The
upload table of `-Pbench.entityCost=true` times the sorted draws (section 7, "Players and holograms").

### Twenty-first session (2026-10-29)
Section 8 item 1b's second bullet: the per-frame quad sort of sorted draws (player bodies, see-through name tag text). One agent
on `26.3` (main tree), every Gradle command under the shared GPU lock. Results in `results/s21-*` and `results/s21-dev/`, logs
`build/bench-logs/s21-*.log`. Commit `e0ee013`.

**What the 26.3 sort reads** (decompiled): `RenderTypeFeatureRenderer$Group.getOrAddDraw` gives a draw a sorting when
`renderType.sortOnUpload()` and improved transparency (OIT) is off: `RenderSystem.getProjectionType().vertexSorting()`, i.e. the
constants `VertexSorting.DISTANCE_TO_ORIGIN` (`byDistance(new Vector3f(0,0,0))`, `origin::distanceSquared`) or `ORTHOGRAPHIC_Z`
(`-z`). `StagedVertexBuffer.uploadDrawsToBuffers` decodes each sorted draw's quad centroids (`decodeSortingPoints`: the mean of
vertices 0 and 2 of each quad, from the written vertex positions), builds `MeshData.SortState(centroids,
IndexType.least(vertexCount))` and calls `writeSortedIndexBuffer(buffer, sorting)`: `sorting.sort(centroids)` (vanilla: keys
from the distance function, stable `IntArrays.mergeSort` by `Floats.compare` descending; here `DistanceSorting`, identical
output) and 6 indices per quad relative to the draw's base vertex. So the inputs are the centroid floats, the sorting's function
and the index type (from the quad count). World vertex positions are camera-relative and unrotated: the view rotation is in the
model-view matrix (`LevelRenderer`: `modelViewStack.mul(viewRotationMatrix)`) and view bobbing in the projection
(`GameRenderer.renderLevel`: `projectionMatrix.mul(bobStack)`), so a turning camera at a fixed spot changes no world centroid.
The first-person hand's bobbing is in its pose stack (positions), but that is the hand's own upload.

**Finding: "still" players are not bit-identical.** `HumanoidModel.setupAnim` always adds `AnimationUtils.bobModelPart` to both
arms (`ageInTicks` = tick count + partial tick, so it changes every frame) unless the arm holds a spyglass. The temporary check
counted bit-identical centroids against the matched draw's last frame: **1.6%** of the matched draws (29k of 1.86M; not broken
down by draw, most likely not player bodies, which are ~99% of the draws). A reuse keyed on identical centroids would almost never hit.

**Design kept: start the sort from last frame's order.** The sorted permutation is unique: every sort here (vanilla's stable
merge sort, `DistanceSorting`'s packed-long and radix sorts) orders by the strict total order "distance key as `Float.compare`
descending, then quad index ascending", so *any* algorithm sorting by that order from *any* start returns the same indices. So
`DistanceSorting.Sorter.sortFrom` computes this frame's keys, packs (key, index) longs in the start order and insertion sorts
them, giving up after 4n+64 moves for the normal sort with the same keys. The cache (`SortOrderCache`, one per
`StagedVertexBuffer`) only supplies a start, so finding the "same" draw can be a guess: quad count plus the half-block cell of the
first centroid. A wrong match (another draw with as many quads in the same cell) or a far-off start costs time, never
correctness; a start is always a previous sort's output of the same length (a permutation). No entry: full sort, new entry.
Entries unused for 4 uploads are dropped (the level's and the hand's features evidently share the dispatcher's buffer, two uploads
per frame; the first version dropped after one upload and matched only 0.01% of draws). Draws whose `VertexSorting` is not a
`DistanceSorting.Sorter` (a mod's) take the vanilla call. Memory: one `int[n]` per live sorted draw.

**Exactness check** (temporary, `scripts/patches/s21-sort-check.patch`, `-Pjvm.ruthenium.probe.sortCheck=true`): every sorted
draw also sorted with a copy of vanilla's algorithm (keys, `IntArrays.mergeSort`, `Floats.compare`) and compared. Players kind
1.86M draws (133M quads), moving players 1.87M (133M quads; started by a matched entry 99.9%, 1 fallback), lobby 1.24M (140M quads,
incl. see-through name tag draws of ~3k quads): **0 mismatches**. Frozen close-ups (`s21-parity`, lobby, `closeShots=4`,
`closeToggle=reuseSortOrder`): on vs off 8339 / 7151 px (players) and 2314 / 4418 (holograms) at threshold 1, same-setting pairs
9598 / 10043 and 3310 / 4649; the diff maps show only the glint and the first-person arm's outline.

**A/B** (5 interleaved pairs, `-Pbench.staticSeconds=10`, switch off vs on in one build, medians; `s21-<dir>`):

| | static off -> on | p99 low | render thread | pan off -> on | pan p99 | pan render thread |
|---|---|---|---|---|---|---|
| players kind (`s21-players`) | 482.7 -> 540.6 (every pair) | 303.4 -> 314.6 | 1.99 -> 1.80 ms | 1645.7 -> 1689.8 (every pair) | 441.9 -> 484.9 | 0.48 -> 0.47 |
| players, 150 skins (`s21-players150`) | 474.8 -> 522.3 (every pair) | 292.3 -> 324.4 | 2.02 -> 1.85 | 1639.9 -> 1672.2 | 429.3 -> 471.3 | 0.49 -> 0.49 |
| lobby (`s21-lobby`) | 328.0 -> 352.9 (every pair) | 186.5 -> 206.5 | 2.96 -> 2.73 | 1516.5 -> 1546.4 | 307.1 -> 324.4 | 0.55 -> 0.55 |
| moving players (`s21-move`) | 465.3 -> 511.1 (every pair) | 278.4 -> 327.1 | 2.09 -> 1.91 | 1632.5 -> 1658.7 | 416.5 -> 454.5 | 0.48 -> 0.48 |
| entities, 3 pairs (`s21-entities`) | 345.6 vs 348.0 | 167.2 vs 172.9 | 2.80 vs 2.78 | 1231.5 vs 1243.2 | 305.2 vs 312.6 | 0.74 |
| mixed, 3 pairs (`s21-mixed`) | 1628.9 vs 1638.8 | 795.9 vs 791.8 | 0.55 vs 0.54 | 1670.8 vs 1666.2 | 719.7 vs 724.5 | 0.42 / 0.43 |

Mixed: initial 1155.3 vs 1119.9, particles 1340.6 vs 1336.5 (pairs mixed both ways; the scene has no sorted entity draws). The
first `s21-players/a` runs were five earlier check runs swept in by `run-ab.sh`; they were moved to `s21-dev/check/`.
`bench.playersMove=true` (new, `PlayerBenchmark`): from the static phase on a server tick listener turns every mannequin 4 degrees
per tick (body and head) and walks it round a 0.3 block circle, so every body changes every frame (walk animation too); the log
line `Players moving: first mannequin ...` shows it on the client.

**Upload table** (`-Pbench.entityCost=true`, one run each, `s21-upload`): players kind, sort and write indices **212.5 -> 45.6
us/frame** (1465 -> 314 ns per draw of 71.5 quads), decode 53 us unchanged, `uploadDrawsToBuffers` 347 -> 179 us; lobby sort
280.1 -> 71.0 us (1905 -> 483 ns per draw of 113 quads), upload 522 -> 304 us. The A/B's render thread gain (0.19 ms for 150
players, 0.23 ms in the lobby) is about what the table shows (0.17 / 0.21 ms), plus less allocation (no packed `long[]` and
result `int[]` per sort).

**Miss path** (temporary property, 3 pairs each, players kind, off vs forced, `s21-dev/miss-*`): every lookup missing (the key
salted per upload: full sort, new entry, map insert and eviction) static 499.2 vs 484.4 fps, render thread 1.95 vs 2.01 ms (2 of
3 pairs slower); every start reversed (worst insertion start: 4n+64 moves, then the full sort) 485.8 vs 470.3, 2.02 vs 2.05 ms.
So a scene where nothing matches pays up to ~0.2-0.4 us per sorted draw (~3% in this 150 player scene, at the noise floor), where
a match saves ~1.2 us. Neither happens in the scenes measured: moving players, the pan (turning camera) and the lobby's name tags
matched 99.9%. A camera moving fast through a crowd changes the first centroid's half-block cell every few frames (flying at 10
blocks/s and 500 fps: every ~25 frames), so it mostly matches too; not measured.

Not verified: the OpenGL backend; OIT (no sorted draws then, so nothing to do); mods that change `getOrAddDraw`'s sorting;
the hand's upload separately (counted in the totals); a real remote player's movement interpolation (mannequins moved by the
server stand in for it).

### Twenty-second session (2026-10-29)
Section 8 "Bigger jobs" job 1: player bodies drawn from a GPU pose buffer. One agent on `26.3` (main tree), every Gradle command
under the shared GPU lock (another agent benchmarked a block entity scene at the same time). Results in `results/s22-pose-*` and
`results/s22-pose-dev/`, logs `build/bench-logs/s22-*.log`. Commit `e52b45d`.

**The "find out first" questions** (26.3 sources):
- *Can a vertex shader read a per-draw buffer?* renderpearl binds three kinds of uniforms (`UniformType`): combined image samplers,
  uniform buffers and texel buffers; no storage buffers. **Uniform buffer slices work per draw on both backends**:
  `RenderPass.setUniform(name, GpuBufferSlice)` takes any slice whose offset is a multiple of `minUniformOffsetAlignment` (vanilla's
  `DynamicTransforms` is a per-draw slice; GL binds it with `glBindBufferRange`, Vulkan pushes it as a push descriptor with the
  slice's offset and range). **Texel buffers** (`samplerBuffer`; vanilla's clouds read one with `gl_VertexIndex` and no vertex
  buffer, on both backends) take only a whole buffer (`FrontendRenderPass`: "Uniform texel buffers do not support a slice of a
  buffer"), and on Vulkan every descriptor push with a texel buffer bound creates a `VkBufferView` and queues its destruction
  (`VulkanRenderPass.pushDescriptors`), i.e. one driver call per draw here. (Twenty-fourth session: on the NVIDIA driver any texel buffer in a submit also costs a fixed ~20 us in `vkQueueSubmit2`.) So: one uniform slice per draw, no per-part sub-draws.
- *How is the sorted draw's index buffer produced?* `StagedVertexBuffer.uploadDrawsToBuffers` decodes each sorted draw's centroids
  from its written vertex slices (`decodeSortingPoints` -> `MeshData.decodeQuadCentroids`: per quad `(v0 + v2) / 2.0F`), builds
  `MeshData.SortState` and writes the indices into the staging buffer (`writeSortedIndexBuffer`, wrapped by `StagedVertexBufferSortM`);
  nothing takes external centroids or an order. Not needed: the posed draw keeps an *empty* staged draw (skipped by the upload) in
  its place in the group and is drawn in that place with its own index buffer, sorted by the staged draw's own `VertexSorting`
  (through a `SortOrderCache` of its own) from centroids computed on the CPU with the same float operations. No vanilla code path is
  copied beyond that arithmetic.

**Design as built** (`PlayerPoseDraws`, switch `playerPoseDraws`, "GPU Player Poses"; section 4 row):
- *Which submits:* the exact `PlayerModel` class, vanilla's `entity_translucent` pipeline, no UV mapping or decal, a strictly ordered
  group of the world's features (the translucent models phase), no OIT. Everything else is vanilla's path, among it invisible
  players seen by teammates (`entity_translucent_cull`), the first-person arm (hand features), GUI paperdolls, glint and other models.
- *Mesh:* per `PlayerModel` instance (wide and slim are different instances, per renderer) a layout: one slot per part with cubes
  (12; at most 12 allowed), the cubes' faces in part space (vertex position, UV and polygon normal as floats, member and slot as
  `RG16_SINT`), 8 copies (one per member of a draw), 36 B per vertex, built once.
- *Per frame:* `ModelFeatureRendererM.prepareModel` hands an accepted submit to `PlayerPoseDraws.prepare`: vanilla's `setupAnim`
  and part tree run into a sink, and `ModelPart.compile` passes each written part's pose to `capturePart` (`visible`, `skipDraw`,
  the outer layer toggles and spectators are vanilla's tree's business). Per member a 1184 B std140 block: light and overlay as the
  entity format's shorts, color, and per slot three pose rows and three normal matrix rows (w: whether vanilla normalizes). Per
  face of the written parts, minus the faces `TransparentFaces` skips (`ModelPartCubeM`'s per texture masks, exposed through
  `TransparentCube`), the centroid: the corners transformed with JOML's `transformPosition` (8 per cube), then `(a + b) / 2.0F` of
  vertices 0 and 2. Faces are numbered in vanilla's write order.
- *Draws:* the member is placed where `Group.getVertexBuilder` would put its vertices (`PosedDrawGroup`): a new staged draw from
  `getOrAddDraw` (its prepared transforms and textures are the draw's), or the group's last draw when the render type repeats
  (consecutive bodies with one skin share a draw and one sort, as in vanilla; up to 8). After the world's features are prepared
  (`upload`), each posed draw's faces are sorted by the draw's `VertexSorting` (`SortOrderCache.sort` with `reuseSortOrder`) and 16
  bit indices `member * vertices + face * 4 + (0 1 2 2 3 0)` are written; blocks and indices go up through two ring buffers.
  `RenderTypeFeatureRendererM` redirects `executeGroup`'s `getExecuteInfo`: a posed draw is drawn there (a copy of `entity_translucent`
  with `player_pose.vsh`, the block's slice at its full declared 9472 B, `drawIndexed`), in the empty staged draw's place.
- *Shader arithmetic:* positions as `instanced_part.vsh` (`precise`, JOML's order). Normals as vanilla writes them: `Matrix3f.transform`
  order, `Vector3f.normalize` only when the pose's normals aren't trusted (players' are: the `(-1, -1, 1)` mirror keeps them), truncated
  to signed bytes (`(int) (clamp(c) * 127)`) and decoded through a table of the bits of `max(c / 127.0F, -1.0F)`. A first version
  normalized in full precision (as instanced parts do): ~200-500 px per close-up differed by 2-4 levels (vanilla lights with the
  shorter decoded vector); the emulation removed them.
- *Fallbacks:* a capture fails (the submit is written by vanilla's path; `setupAnim` runs again) when a written part isn't the
  layout's or is written twice, or anything writes a vertex to the sink (a mod drawing in `ModelPart.render`); a model whose
  captures fail 64 times in a row takes vanilla's path until the next clear. A vanilla write that joins a posed draw (same render
  type, consecutive: a player head with the same skin, a failed capture, a ninth member) first writes the posed members with
  vanilla's path (`materialize`, from a redirect of the staged builder fetch in `Group.getVertexBuilder`), so the draw is vanilla's. Max 64 layouts. Cleared on
  resource reload and when toggled.

**Exactness check** (temporary, `scripts/patches/s22-pose-check.patch`, `-Pjvm.ruthenium.probe.poseCheck=true`): every posed member
also written through the switch-off path (`ModelPartCubeM`'s direct writes into a `BufferBuilder`) and through vanilla's own
`Cube.compile` arithmetic (a wrapper that isn't a `BufferBuilder`, all faces); the shader's position and normal arithmetic emulated in
Java floats from the uploaded rows and the mesh; centroids compared with those decoded from the direct write; each draw's order
compared with vanilla's sort (keys, stable `IntArrays.mergeSort` by `Floats.compare`, descending) of the decoded centroids:

| run | members posed | faces | draws (several members) | mismatches |
|---|---|---|---|---|
| players kind | 902,532 | 62.3M | 863,134 (39,398) | 0 |
| moving players | 1,015,242 | 70.1M | 974,637 (40,545) | 0 |
| 150 skins | 1,010,378 | 69.8M | 1,010,378 (0) | 0 |
| lobby | 797,683 | 55.1M | 764,207 (33,476) | 0 |
| lobby, hurt players in the close-ups | 687,192 | 47.4M | 659,312 (27,880) | 0 |
| players, hurt, glass and water, normal emulation | 761,285 | 52.6M | 729,283 (32,002) | 0 (also 0 normal byte mismatches) |
| forced failures (every 7th capture), polished code | 683,440 (130,035 failed) | 47.3M | 429 materialized | 0 |

"Mismatches" covers positions against the direct write and against vanilla's arithmetic (bit for bit; no signed zero cases), face
counts, centroids (bit for bit), UVs, light/overlay/color, quantized normal bytes (after the emulation) and index order. The
first-version normal error was at most 1/127 (quantization). Forced failures exercise the fallback and materialization paths
(every failed join materialized its draw).

**Parity** (frozen close-ups, `closeShots=4`, `closeToggle=playerPoseDraws`: shots 0 and 2 on, 1 and 3 off; new bench options
`playersHurt` (every third player red during the close-ups) and `playersGlass` (stained glass through and water around first row
players, placed while ticks are frozen)). Same-setting pairs differ by the glint and the first-person arm (thousands of px); a pixel
counts as caused by the switch only if it is equal within each setting and differs across them (`scripts/StableDiff.java` at 2
levels, `ImageDiff`'s threshold 1):

| close-up | on vs off | same-setting pairs | stable across the switch |
|---|---|---|---|
| Vulkan players (hurt, glass, water) close0 / close1 | 3383 / 4992 | 2212-6899 / 1605-5203 | 297 / 8, all inside or next to glint |
| Vulkan lobby (hurt; holograms, name tags over bodies) | 7822 / 4398 | 2925-9023 / 3629-5610 | 0 / 0 |
| OpenGL players (hurt, glass, water) | 7725 / 2418 | 1303-10350 / 4616-5796 | 0 / 0 |
| OpenGL lobby (hurt) | 8425 / 190 | 3279-9327 / 152-205 | 404 / 0, inside or next to glint |

Final code, 8 shots per position (`s22-pose-dev/parity8-*`, `scripts/StableDiff.java`: equal in all four shots of each setting, different
across): Vulkan players (hurt, glass, water) and OpenGL lobby (hurt): **0 pixels** in both close-ups, at a difference of 2 levels and
also of 1 level. The first-version normals (full precision) showed as ~200-500 stable pixels of 2-4 levels per close-up.

**A/B** (5 interleaved pairs unless noted, `-Pbench.staticSeconds=10`, switch off vs on in one build; static / pan fps, p99 low,
render thread ms per frame; medians):

| | static off -> on | p99 low | render thread | pan off -> on | pan p99 | pan render thread |
|---|---|---|---|---|---|---|
| players kind (`s22-pose-players`) | 523.6 -> 603.5 (every pair) | 311.2 -> 371.6 | 1.86 -> 1.60 ms | 1631.3 -> 1682.2 (every pair) | 439.7 -> 494.2 | 0.48 -> 0.47 |
| players kind, first hooks (`s22-pose-players-v1`) | 522.4 -> 609.2 (every pair) | 302.8 -> 355.4 | 1.85 -> 1.58 | 1625.2 -> 1676.3 (every pair) | 424.9 -> 491.9 | 0.48 -> 0.47 |
| players, 150 skins* (`s22-pose-players150`) | 514.1 -> 605.0 (every pair) | 308.7 -> 343.3 | 1.88 -> 1.61 | 1622.2 -> 1681.1 (every pair) | 428.0 -> 483.2 | 0.49 -> 0.47 |
| moving players* (`s22-pose-move`) | 507.7 -> 581.1 (every pair) | 324.6 -> 347.9 | 1.92 -> 1.67 | 1608.5 -> 1668.9 (every pair) | 416.1 -> 466.5 | 0.49 -> 0.48 |
| lobby* (`s22-pose-lobby`) | 351.6 -> 397.3 (every pair) | 193.7 -> 225.6 | 2.76 -> 2.45 | 1485.7 -> 1514.5 (every pair) | 314.2 -> 338.7 | 0.57 -> 0.54 |
| OpenGL, players kind* (`s22-pose-gl-players`) | 417.7 -> 465.0 (every pair) | 247.2 -> 270.1 | 2.31 -> 2.08 | 1417.8 -> 1461.2 (4 of 5) | 379.3 -> 396.2 | 0.68 -> 0.66 |
| mixed, 3 pairs (`s22-pose-mixed`) | 1569.4 vs 1598.0 | 705.5 vs 724.1 | 0.54 vs 0.55 | 1621.3 vs 1609.3 | 612.2 vs 560.8 | 0.42 vs 0.44 |
| mixed, first hooks (`s22-pose-mixed-v1`) | 1582.0 vs 1586.0 | 743.4 vs 734.5 | 0.55 vs 0.56 | 1627.8 vs 1609.1 (3 of 3 lower) | 617.2 vs 588.7 | 0.43 vs 0.44 |
| entities, 3 pairs (`s22-pose-entities`) | 348.1 vs 340.3 (3 of 3 lower) | 169.8 vs 169.3 | 2.77 vs 2.84 | 1215.0 vs 1192.1 (3 of 3 lower) | 309.9 vs 305.2 | 0.74 vs 0.76 |
| entities, first hooks (`s22-pose-entities-v1`) | 340.8 vs 342.5 | 169.3 vs 170.1 | 2.87 vs 2.84 | 1204.8 vs 1194.2 (2 of 3 lower) | 302.8 vs 298.7 | 0.76 vs 0.76 |
| entities, commit A/B `111db82` vs `e52b45d` (`s22-pose-entities-commits`) | 345.0 vs 334.6 (2 of 3 lower) | 170.6 vs 165.1 | 2.82 vs 2.90 | 1202.3 vs 1197.9 (2 of 3 lower) | 311.6 vs 293.6 | 0.73 vs 0.76 |

\* measured with the first variant of the two group hooks (an `@Inject` at `Group.getVertexBuilder` HEAD and an `@WrapOperation` around
`executeGroup`'s `getExecuteInfo`); both became `@Redirect`s (no per-call callback or wrapper object) after the mixed and entity scenes'
pans read lower in all pairs, and the players kind, mixed and entity matrices were run again with them (rows without "first hooks").
Within an off/on A/B the scenes without players differ only in `accepts()`'s checks per model submit (now pipeline first); every other
hook runs on both sides. The commit A/B covers all of them. The entity cost probe on both commits (one run each, `s22-pose-dev/probe-
entities-commits`) put the differences in run-to-run drift: extraction and submission, which the change doesn't touch, were 6.5% and
4.7% slower in the new commit's run (0.567 -> 0.604, 0.468 -> 0.490 ms per frame), model prepares, where the new checks are, 1.4%
(0.736 -> 0.746; part trees 0.334 -> 0.340). Not resolved further: the entity scene's 3 pair resolution is ~1.5-3% (section 6.6).

Per player (players kind, render thread): 1.86 -> 1.60 ms for 150 players is ~1.7 us per player of the body's ~3.5 us (vertex writes
2.8 us, centroid decoding 0.35, sort 0.3); upload bytes per body go from ~10.4 KB of vertices to 1.2 KB of poses (indices unchanged).

**Mod compatibility** (exact for vanilla; switch "GPU Player Poses"): mods that act on the player body's written vertices (a
`VertexConsumer` wrapper on the group's builder, mixins into `Cube.compile` / `ModelPartCubeM` or `BufferBuilder`) don't see them;
mods that pose parts (`setupAnim`, `ModelPart` fields, `translateAndRotate`) or hide them work, as do mods drawing in
`ModelPart.render` (detected through the sink: vanilla's path) and mods with their own model class or render type (not accepted);
a mod hooking `PreparedRenderType.drawFromBuffer` or `RenderTypeFeatureRenderer.executeGroup`'s draws doesn't see posed draws;
shader packs that replace vanilla's pipelines (Iris-like) would not replace `player_pose`'s copy of `entity_translucent` (the same
holds for the instanced and retained pipelines).

Not verified: invisible players seen translucent by teammates or spectators (`entity_translucent_cull`: not accepted, vanilla's path
by the pipeline check; the scene can't make a mannequin translucent to a creative camera); real `RemotePlayer`s (mannequins take
the same renderer); OIT (not accepted); the `normalize` branch of the normal emulation (players' poses have trusted normals: 0
normalized faces in the check) and the color conversion for non-white tints (bodies are white; GPU `UNORM8` decode assumed exact like
Java's `c / 255.0F`); hardware other than the RTX 4060 Laptop (Vulkan and OpenGL); mods.

### Twenty-third session (2026-10-29)
Section 8 "Bigger jobs" job 2: retained billboard text. One agent on `26.3` (main tree). Results in `results/s23-labels-*` and
`results/s23-dev/`, logs `build/bench-logs/s23-*.log`. Commit `4890a5d`.

**The "find out first" questions** (answered by research before the session, re-checked in the 26.3 sources):
- *Indirect multi-draw:* Vulkan always (`multiDrawIndirect`, `drawIndirectFirstInstance` are required features); OpenGL only with
  `GL_ARB_multi_draw_indirect` (GL 3.3 is the minimum), base instance only with `GL_ARB_base_instance`, and `gl_DrawID` /
  `gl_BaseInstance` only with `GL_ARB_shader_draw_parameters`. `RenderPass.drawIndexedIndirect(slice, count)` takes 5-int commands from
  a `USAGE_INDIRECT_PARAMETERS` buffer; the frontend checks `drawIndirect` / `multiDrawIndirect` but not `maxDrawIndirectDrawCount`.
  There are also direct multi-draws (`multiDrawIndexed`: Vulkan `VK_EXT_multi_draw` "interleaved", OpenGL `glMultiDrawElementsBaseVertex`
  "separate", each on one backend only). **Texel buffers** (`samplerBuffer`, a whole buffer per binding, as the clouds use) work on both.
- *Render types, phases, order:* 26.3 has no `text_background` / `text_intensity` types; backgrounds and effects use the white glyph
  through the label's own glyph type. `text` (name tags: the non-sneaking NORMAL submit is opaque white and emissive, so the **solid**
  phase; sneaking name tags the **nameTags** phase), `text_polygon_offset` (text displays and signs; lines at opacity 255 in the solid
  phase, their `StandaloneBackground` and translucent lines in the **texts** phase; **sorted on upload**), `text_see_through` (the
  non-sneaking SEE_THROUGH submit and see-through displays; the strictly ordered **see-through** phase, sorted by distance;
  `text_grayscale_see_through` is also sorted on upload). Solid, nameTags and texts are `SimpleFeatureRenderPhase`s (one group per
  feature type in submission order, may reorder: Ruthenium's `mergeDraws` merges equal types there); translucent order: shadows,
  translucent models, **nameTags, texts**, custom geometry (`FeatureRenderDispatcher.executeTranslucent`), then outline, then see-through.
  **Correction:** the nineteenth session's note that "vanilla draws translucent text backgrounds before the translucent models phase"
  (section 7 "Players and holograms", section 8 job 1) is wrong: backgrounds (texts phase) and translucent name tags (nameTags phase) draw
  *after* the translucent models. A player body behind a hologram background is drawn first and the background blends over it. The
  twenty-second session's `PlayerPoseDraws` doesn't depend on that claim (it keeps each body's draw in vanilla's place).

**Measured first** (elimination, `s23-dev/elim`, 2 pairs, lobby static, a temporary property that skipped `renderText` for kept lines
in the world pass): 400.1 -> 532.2 fps, render thread 2.43 -> 1.82 ms. Labels cost **~0.61 ms per frame** in the lobby (~940 label
submits per frame at the static view: glyph writes, their upload and sort, and their GPU work), more than the ~0.3-0.4 ms estimate.

**Design as built** (`RetainedLabels`, switch `retainedLabels`, "Retained Labels"; section 4 row). Not the brief's indirect multi-draw
of instance records: a per-vertex **region** index into a **texel buffer of poses**, so that a draw sorted on upload keeps vanilla's
quad order across labels (with per-label sub-draws a sorted draw would need a sub-draw per run of consecutive quads of one label, and
labels at similar distances interleave), and every vanilla draw stays one draw:
- *Mesh:* a kept prepared text (`SignTextCache` lines: name tags, text display lines, signs that aren't retained) is wrapped in
  `SignTextCache.KeptText`; from its second frame it gets a mesh: its glyph and effect quads recorded through a `VertexConsumer` that
  takes the label-space coordinates handed to `addVertex(pose, x, y, z)` (only `BakedSheetGlyph`'s glyph and effect instances; anything
  else, or more than 2048 quads, is vanilla's path), with their UV and color. Runs of renderables with one render type are computed per
  display mode. Meshes are appended to one GPU vertex buffer (the arena; 28 B per vertex: position, UV, color, region), up to 16384
  regions (four RGBA32F texels each, within the minimum texel buffer size); a full arena is replaced at the next frame and meshes are
  placed again when next drawn.
- *Per frame:* `TextFeatureRendererM` redirects the `PreparedText.visit` of a text submit without outline. Each run picks the group's
  draw exactly as `Group.getVertexBuilder` would (`joiningDraw` or `getOrAddDraw`), and is added to that draw's `LabelDraw` (attached to
  the staged draw like `PlayerPoseDraws`' posed draws) if the draw has no vertices; otherwise it is written as vanilla writes it. A
  vanilla write joining a label draw writes its labels first (`materialize`). The label's region gets its pose rows and light (64 B).
- *Upload* (after the world's features are prepared): new meshes go up with one `writeToBuffer`; unsorted draws become **indirect
  commands over ranges of the arena** (consecutive labels placed one after the other are one range; the sequential quad index buffer,
  base vertex per command, no instance data, so `nonZeroFirstInstance` isn't needed; gated like vanilla's terrain: `drawIndirect`,
  `multiDrawIndirect`, no known MDI issues, split by `maxDrawIndirectDrawCount`), or 16/32-bit indices where MDI isn't available; sorted
  draws get indices in the order of the draw's own sorting over centroids computed on the CPU with JOML's `transformPosition` of
  vertices 0 and 2 and `/ 2.0F` (inlined in JOML's generic operation order, checked against JOML at startup), through a
  `SortOrderCache` with `reuseSortOrder`. **A sorted draw whose inputs are bit for bit last frame's** (same meshes, places, quad ranges,
  pose elements and JOML properties, same sorting) keeps last frame's order (the centroids would be the same floats).
- *GPU:* copies of vanilla's six world text pipelines with `label_text.vsh` (vanilla's `text.vsh` with the position computed from the
  region's rows with `precise` in JOML's order, the light from the texel buffer).

Per frame in the lobby's static view (temporary counters): 940 labels in 3 draws (solid `text`, solid `text_polygon_offset` sorted with
6078 quads, see-through), 15318 quads; upload 175 us with per-quad 32-bit indices for every draw, 96 us with MDI ranges (2-33 ranges for
the two unsorted draws) and 16-bit indices, 39 us with the sorted order kept (sort 70 -> 13 us); submit ~0.1 us per label.

**Exactness check** (temporary, `scripts/patches/s23-label-check.patch`, `-Pjvm.ruthenium.probe.labelCheck=true`): every retained label
also visited with vanilla's renderables, pose and light into a recorder, compared with its mesh: vertex count, runs (render type and
quads per vanilla draw selection), positions (JOML's transform of the mesh's coordinates and the shader's arithmetic in Java on the
uploaded rows, bit for bit), UV, color, light; every sorted draw's order against a fresh full sort of JOML-computed centroids:

| run | labels | vertices | sorted draws | sorted quads | mismatches |
|---|---|---|---|---|---|
| lobby + label variety, close-ups with the switch toggled | 9,777,436 | 637.3M | 13,177 | 61.0M | 0 |
| moving players, label variety, `mergeDraws` and `reuseSortOrder` off | 7,918,336 | 516.1M | 331,246 | 50.8M | 0 |
| lobby, with the kept sort order | 6,223,560 | 405.9M | 8,760 | 40.5M | 0 |

24.0M labels in all.

**Parity** (frozen close-ups, `closeShots=8`, `closeToggle=retainedLabels`, with the new `bench.labelVariety=true`: a wall of shadowed,
see-through, half-opacity, red-background, background-less, underlined / struck / italic, obfuscated, CJK and accented, wrapped and aligned,
fixed and scaled text displays, stacked and overlapping displays and name tags, a named mob, stained glass in front; `close2`), and
`playersHurt`, `scripts/StableDiff.java`:

| close-up | Vulkan stable px (1 / 2 levels) | OpenGL stable px (1 / 2 levels) |
|---|---|---|
| close0 (players, name tags over bodies) | 0 / 0 | 0 / 0 |
| close1 (holograms) | 0 / 0 | 0 / 2 (2 levels, next to glint, unstable at 1 level) |
| close2 (label variety) | 5 / 5 (obfuscated text) | 0 / 0 |

The obfuscated text is vanilla's path in both settings; its random glyphs differ stably between the two settings' shots because the
toggle changes how many frames are drawn between shots: a run toggling an unrelated switch (`fastBoatTicks`) shows 14 such pixels there.
With `mergeDraws` off (vanilla's draw order) and 4 shots: only glint-noise pixels and the obfuscated text.
Fallback writes and materialized draws (the patch's `labelFallback`: every other mesh written as vanilla does, so most draws mix and
materialize; Vulkan, 8 shots): 0 stable pixels except the obfuscated text; its check: 5.6M labels, 0 mismatches.

**A/B** (`-Pbench.staticSeconds=10`, switch off vs on in one build, interleaved; static / pan fps, p99 low, render thread ms per frame;
medians):

| | static off -> on | p99 low | render thread | pan off -> on | pan p99 | pan render thread |
|---|---|---|---|---|---|---|
| lobby, final code, 5 pairs (`s23-labels-final-lobby`) | 398.8 -> 479.1 (every pair) | 225.0 -> 259.4 | 2.43 -> 2.03 ms | 1573.4 -> 1596.7 (4 of 5) | 350.2 -> 398.0 | 0.52 -> 0.53 |
| lobby, first code*, 5 pairs (`s23-labels-lobby`) | 404.2 -> 477.1 (every pair) | 226.9 -> 265.5 | 2.42 -> 2.03 | 1571.8 -> 1590.8 (4 of 5) | 357.5 -> 390.1 | 0.53 -> 0.53 |
| lobby, moving players*, 3 pairs (`s23-labels-move`) | 382.9 -> 457.6 (every pair) | 218.2 -> 265.2 | 2.52 -> 2.14 | 1554.3 -> 1570.7 | 330.7 -> 374.8 | 0.53 -> 0.54 |
| holograms only*, 3 pairs (`s23-labels-holograms`) | 1226.9 -> 1721.0 (every pair) | 658.3 -> 786.6 | 0.79 -> 0.55 | 2043.9 -> 2150.8 (every pair) | 873.9 -> 932.7 | 0.39 -> 0.36 |
| OpenGL lobby, final code, 3 pairs (`s23-labels-final-gl`) | 329.5 -> 392.7 (every pair) | 193.6 -> 219.1 | 2.95 -> 2.47 | 1316.6 vs 1281.9 (3 of 3 lower) | 297.0 -> 324.1 | 0.74 vs 0.74 |
| OpenGL lobby*, 3 pairs (`s23-labels-gl`; one on run 50 fps, see below) | 326.6 -> 395.0 | 195.9 -> 210.5 | 2.97 -> 2.46 | 1300.6 -> 1317.3 | 293.9 -> 325.6 | 0.74 -> 0.74 |
| OpenGL lobby*, recheck, 2 pairs (`s23-dev/gl-recheck`) | 329.3 -> 391.7 | 187.5 -> 221.8 | 2.95 -> 2.46 | 1305.7 vs 1304.4 | 297.0 -> 320.4 | 0.74 vs 0.74 |
| kept sort order off -> on*, 3 pairs (`s23-labels-sortreuse`) | 462.6 -> 473.4 (2 of 3) | 252.2 vs 251.9 | 2.08 -> 2.06 | 1581.1 vs 1583.7 | 391.3 vs 391.1 | 0.53 vs 0.53 |
| entities, final code, 3 pairs (`s23-labels-final-entities`) | 344.6 vs 339.7 (2 of 3 lower) | 166.1 vs 167.9 | 2.84 vs 2.86 | 1231.6 vs 1211.2 (3 of 3 lower) | 317.5 vs 300.5 | 0.75 vs 0.77 |
| entities*, 2 pairs (`s23-labels-entities`) | 348.7 vs 331.8 (2 of 2 lower) | 173.6 vs 162.3 | 2.78 vs 2.93 | 1241.1 vs 1217.4 (2 of 2 lower) | 309.1 vs 301.5 | 0.72 vs 0.74 |
| mixed*, 2 pairs (`s23-labels-mixed`) | 1623.7 vs 1647.6 | 789.9 vs 804.0 | 0.54 vs 0.54 | 1666.3 vs 1679.9 | 723.9 vs 737.4 | 0.43 vs 0.43 |

\* before the final fix of the arena growth and the fallback writes (below); the label path itself is the same. Earlier development
A/B (`s23-dev/ab-v1`, 3 pairs, per-quad 32-bit indices for every draw, no MDI, no kept order): lobby static 392.6 -> 451.5.

**Found by the OpenGL matrix:** one on run ran its static view at 50 fps (render thread 18.9 ms per frame, slow in proportion to the
labels in view in its pan). Cause, reconstructed (not reproduced as such): the first arena was sized from too few live labels (64k
vertices against the lobby's ~61k plus labels seen elsewhere), filled up, and the reset rule ("only if this frame's labels fit into half
the arena") never let it grow, so every frame the overflowing labels were written through the generic `VertexConsumer` path, and each
such write into a label draw materialized the whole merged draw (36k vertices) through it as well. Fixed in the commit: a full arena is
replaced by one sized for twice the labels wanted that frame (placed or not) whenever that fits the 4M vertex cap, and the fallback
writes go through `ExtendedVertexBuilder` as `BakedSheetGlyphM` writes (the same bytes). Checked with a temporary 4096 vertex first arena
and the per-quad index path on OpenGL: one frame of fallback writes (548 runs), then a 131072 vertex arena holding all 940 labels; close-ups
of that run, 8 shots: 0 stable pixels except the obfuscated text.

**The entity scene** reads ~1.5% lower with the switch on in 5 of 5 pan pairs (3 of 5 static). Counters there: ~75 labels in 3 draws per
frame at the static view, upload 5-8 us and label submits ~8 us per frame (with the timers' own cost), no fallback runs, no materialized
draws, and almost no labels in the pan, so the label path costs less than the ~45 us of glyph writes it replaces. Not explained further
(the entity scene's resolution is ~1.5-3% with 3 pairs, section 6.6); with both switch sides sharing every other hook, only the label
path, its per-frame ring rotations (three `MappableRingBuffer`s once used) and its draws differ. **Explained in the twenty-fourth session:**
the texel buffer of poses (a fixed ~20 us per frame in the NVIDIA driver's `vkQueueSubmit2`); fixed by a pose texture.

**Mod compatibility** (exact for vanilla; switch "Retained Labels"): mods that act on the written text vertices (a `VertexConsumer`
wrapper on the group's builder, mixins into `BakedSheetGlyph.render`, `BufferBuilder` or the upload's sort of those draws) don't see the
vertices of retained labels; mods that replace glyphs with their own `TextRenderable`s, outline text or draw obfuscated text keep vanilla's
path; a mod hooking `PreparedRenderType.drawFromBuffer` or `executeGroup`'s draws doesn't see label draws; shader packs that swap vanilla's
text pipelines would not swap the `label_*` copies (as with the instanced, retained and posed pipelines).

Not verified: OIT (not accepted: vanilla's path), hardware without indirect multi-draw (the per-quad index path ran on OpenGL with
`noIndirect`: 0 stable pixels), TrueType fonts (the recorder takes any `BakedSheetGlyph`), hardware other than
the RTX 4060 Laptop, mods.

### Twenty-fourth session (2026-10-29)
The twenty-third session's open issue: the entity scene ran ~1.5% slower with `retainedLabels` on (5 of 5 pan pairs, 3 of 5 static), although
counters put the label path at ~15 us per frame, less than the glyph writes it replaces. One agent on `26.3` (main tree). Results in
`results/s24-dev/`, `results/s24-labels-entities`, `results/s24-labels-lobby`, `results/s24-final-toggle-*`, logs `build/bench-logs/s24-*.log`.
Commit `48af349`.

**Method.** An in-launch A/B (`bench.toggleField=retainedLabels`, new, section 6.6) first: 10 rounds, entity scene, off/on fps per round
median **1.0242 static (10 of 10 off faster), 1.0261 pan (10 of 10)**, render thread 2.79 -> 2.85 ms. So the cost is real, direct (both
settings on the same compiled code, not JIT shape) and on the render thread's CPU: frames without a tick 2.77 -> 2.83 ms. The light profiler
at depth 9 per round (on minus off, mean of the later rounds): `frame/submit` **+23-25 us** (0.052 -> 0.076 ms), of which the new
`queueSubmit` split (`vkQueueSubmit2`) **+20 us** (0.042 -> 0.062); `executeFrameGraph` +12 us (label draws recorded; `renderSolidFeatures/Text`
+6 us), label upload +4 us, `present` +5 us; against `prepare/Text` -14 us (the glyph writes saved). Eliminations, each one launch of 8 rounds
(temporary properties in `RetainedLabels.draw`/`upload` and the entity scene, removed):

| variant (on side) | off/on static median | `queueSubmit` on - off |
|---|---|---|
| as committed in `4890a5d` | 1.017-1.024 | +20 us |
| label draws skipped (labels invisible) | 0.991 (on faster) | ~0 |
| state bound, no draw call (no descriptor push) | 0.999 | ~0 |
| no indirect draws (16/32-bit indices) | 1.019 | +20 us (not MDI) |
| each draw issued 5 times, with a new push (and `VkBufferView`) each | 1.012 | +20 us (does not scale) |
| each draw issued 5 times, one push | 1.013 | +22 us |
| poses written into a device-local buffer (`writeToBuffer`) | 1.023 | +21 us (not mapped memory) |
| poses, indices and commands all device-local | 1.017 | +21 us |
| **clouds on** (vanilla's clouds read a texel buffer) | **1.002** | **~0**: vanilla's `queueSubmit` is 0.063 ms on both sides |
| **poses in an RGBA32F texture** (the fix) | 0.991 | +0.2 us |

**Cause:** on this NVIDIA Vulkan driver, a submit whose command buffer uses any texel buffer costs a fixed ~20 us more in `vkQueueSubmit2`,
whatever the number of draws or buffer views (vanilla's clouds pay it whenever clouds are on, and then the switch's own cost is ~0); plus
~5 us per frame of recording, most likely the `VkBufferView` that `VulkanRenderPass.pushDescriptors` creates for every push with a texel
buffer bound (`renderSolidFeatures/Text` +6.3 us before, +0.7 us after). The lobby's gain hid it (its pan gain was +1.5% then, +3.4% now).
Not checked: AMD/Intel drivers. The OpenGL lobby's pan was lower with the switch on in 3 of 3 pairs in the twenty-third session and is
higher in 4 of 4 in-launch rounds now, which suggests a texel buffer cost on OpenGL too (not measured separately).

**Fix** (`48af349`): the pose rows and light go into an RGBA32F texture, 256 x 256 (four texels per region; region r at texels 4r..4r+3,
texel t at (t & 255, t >> 8)), created once; each frame `writeToTexture` uploads the rows holding regions 0..maxRegion (whole rows of
`poseBytes`; other regions' texels are never read that frame), and `label_text.vsh` reads them with `texelFetch(sampler2D, ivec2, 0)` (nearest,
clamp). Same bytes, same shader arithmetic, so the positions stay bit-identical. A single texture is safe: the copy is recorded outside
render passes and every Vulkan render pass ends with an ALL_COMMANDS memory barrier (`submitRenderPass`), so last frame's label draws are
ordered before it; OpenGL synchronizes `glTexSubImage2D` itself. The pose `MappableRingBuffer` is gone (the index and command rings stay).

**Exactness:** frozen close-ups, `closeToggle=retainedLabels`, 8 shots, `StableDiff` at 1 and 2 levels: players lobby with `labelVariety` and
`playersHurt`: close0 and close1 0 stable pixels on Vulkan and OpenGL, close2 only the obfuscated "Obf" label (2 px Vulkan, 9 px OpenGL,
checked on crops; random glyphs, see the twenty-third session); entity scene close0-7 0 stable pixels (Vulkan; with 4 shots one 2-level
pixel on a dropped item's edge in close7, gone with 8). The CPU-side data (pose rows, light, indices, sort orders) is unchanged, so
`scripts/patches/s23-label-check.patch` was not re-run; the label path was active on both backends (the `prepare/Text` savings in the
profiler, the OpenGL lobby gain).

**Numbers** (switch off -> on; fps; render thread ms per frame):

| | static | pan |
|---|---|---|
| entities, in-launch, 10 rounds, before (`s24-dev/toggle1`) | off/on 1.0242 (10 of 10 off faster) | 1.0261 (10 of 10) |
| entities, in-launch, 10 rounds, after (`s24-final-toggle-entities`) | 346.0 vs 345.1, off/on 1.0062 (7 of 10), 2.82 vs 2.82 ms | 1184.2 -> 1186.3, 1.0004 (5 of 10) |
| entities, 5 interleaved pairs, `staticSeconds=10` (`s24-labels-entities`) | 339.9 -> 344.1 (3 of 5 higher), p99 158.0 -> 168.6, 2.85 -> 2.80 | 1198.5 -> 1203.4 (4 of 5), p99 301.1 -> 303.7 |
| lobby, 3 pairs (`s24-labels-lobby`) | 390.0 -> 478.2 (every pair), p99 228.8 -> 257.8, 2.49 -> 2.00 | 1521.6 -> 1574.1 (every pair), 0.54 -> 0.53 |
| OpenGL lobby, in-launch, 4 rounds (`s24-final-toggle-gl-lobby`) | 317.7 -> 400.1 (every round), 3.02 -> 2.44 | 1271.2 -> 1302.8 (every round) |

Still open: a ~0.3-0.6% static residual in the entity scene's in-launch A/B (7 of 10 rounds, about 1-2 standard errors; frames without a
tick 2.796 vs 2.809 ms): the label upload (one `writeToTexture` and two ring maps every frame, ~4-5 us) and the label submits roughly cancel
the ~15 us of glyph writes saved there. Skipping the pose upload when the rows are bit-identical to last frame's would only help static
views; not done. The submit split and `ToggleAb` stay in the harness.

### Twenty-fifth session (2026-10-29)
The mixed scene, two agents on `26.3` (main tree), one after the other, every Gradle command under the GPU lock (`scripts/gpulock.py`,
section 6.3). First an investigation (nothing committed; report `results/s25-dev/ideas-report.md`, results `results/s25-dev/`, logs
`build/bench-logs/2073-2101`, patches `scripts/patches/s25-*`), then the builder: its candidates 1 and 3. Commits `46a7390` and `cea8b4b`.

**Investigation: where the mixed static frame goes.** The mixed static view is **CPU bound**, unlike the old server scene (yaw 0: GPU
0.358 ms vs render thread ~0.6 ms; yaw 270: 0.425 vs 0.644), so CPU savings show in fps. Its client tick is ~0.53 ms (the 0.93 ms in
section 8 item 2 was the server scene). Light profiler at depth 8, confirmed by the entity cost probe and counters:
- Armor stand chain ~0.27 ms (44%): extraction 0.082 ms (probe 261 ns x 242), `submitFeatures` 0.032, `prepare/Entity Model` 0.088 (396 ns
  x 242), `InstancedModels.prepareDraws` 0.028 (ring map and copy 16 us, `RenderType.prepare` 9, rotate 2.6), `InstancedModels.draw`
  0.051 (65 draws: `setPipeline` 18 us, `drawIndexed` 15, vertex buffers 7, textures 4.5, default uniforms 4).
- The cause of the instanced part: `ArmorStandModel.setupAnim` sets `basePlate.yRot = -yRot`, so each distinct stand yaw (37 in the scene)
  gave a different part snapshot and its own group: 65 groups per frame for 244 instances, 22 of 242 visible stands over the 32 groups
  per model cap (vanilla's path), ~1.4 us of CPU per group per frame.
- Other: `queueSubmit` 0.057 ms (clouds' texel buffer, below), `awaitGpu` 0.039, `present` 0.031, GUI 0.027, `setupFrameGraph` 0.028,
  sky 0.015, retained draw recording 0.011, block entity extraction 0.012.
- Yaw 270: block entities 0.122 ms, mostly ~3.6k closed books at ~30 ns; still CPU bound. GPU by pass: yaw 0 retained 0.071 / instanced
  0.067 / solid terrain 0.089 ms of 0.358; yaw 270 0.137 / 0.042 / 0.110 of 0.425; not the limiter.
- Tick 0.53-0.54 ms: entities 0.22 (armor stands 114 us, the local player 104-113 us, mostly cold once-per-tick code), table batch 0.08,
  blocks atlas animation 134 us, animateTick ~0.12. Tick frames take 1.24-1.46 ms (0.55-0.62 without) and set the p99; a long tail.
- Allocation (JFR, render thread ~310 MB/s, ~0.19 MB/frame): armor stand submit pose copies 20.5% (`Matrix4f` 11.3, `Matrix3f` 6.0, `Pose`
  3.2), `ModelFeatureRenderer$Submit` 6.7, `InstancedModels.prepareDraws` per-group objects ~16, `CrossbowItem.getChargeDuration`'s
  enchantment iterator for empty stacks 6.3, `VkWriteDescriptorSet` 5.2-5.9, `DynamicGpuData$ChunkSectionInfo` 5.8, `RenderTypes.entityCutout`
  memo 7.8, `BlockPos.containing` in `getPackedLightCoords` 2.9 (`scripts/patches/s25-AllocSites.java.txt` lists samples by site).

**Join and teleport frames** (`-Pbench.slowFrameMs=8` from the diagnostics patch: every frame over 8 ms logged with its profiler tree):
- Frame 0 (108-155 ms): `submit/destroyQueue` 20-41 ms is vanilla's new-world auto icon (`GameRenderer.takeAutoScreenshot` ->
  `Screenshot.takeScreenshot`: a per-pixel `getInt`/`setPixelABGR` loop over 2M pixels, run as a destroy-queue callback; confirmed by a destroy
  queue probe and JFR); the first (cold) extraction of 296 armor stands 12-13 ms, their cold model prepare 12-17 ms, chat 2-3 ms.
- The next tick: packets 44-54 ms (19 `SectionBlocksUpdate` 33-38 ms, 1040 `BlockEntityData` 5-8 ms): the scene's construction; tick 16 ms cold.
- Frame 1: `runLightUpdates` 13.6 ms, `sectionUpdates` 7 ms (vanilla). Later: `updateSectionOcclusion` 8 ms (vanilla) and GC frames.
- Frames 3-6: retained builds, 2-3 per frame, ~170 ms in all, ~85% JIT-cold (the first 520 block entity build 55-63 ms). Warm, the same 8192
  block entities take ~25 ms. A warm 2048 build (1792 chests + 256 signs) 6.6 ms: `SectionFaceGroups.sortByPlanes` 3.3 (keys 0.58,
  `Arrays.sort` 1.6, planes 0.22, gather 0.97), upload 1.05, vertex writes 0.77, checks 0.37, extract 0.24, submit 0.30, retainable 0.28,
  light 0.14. The pan's first view of a section: an 11 ms build.

**Ranked candidates** (the report has designs and traps):
1. Armor stand base plate as its own instance: done (below).
2. Clouds without a texel buffer: vanilla's `CloudRenderer` binds its faces as a texel buffer, which costs ~20 us per frame in `vkQueueSubmit2`
   (twenty-fourth session) plus a `VkBufferView` per draw. Clouds off vs on (single runs, not interleaved): `queueSubmit` 0.0566 -> 0.0273 ms,
   frame 0.608 -> 0.563 ms. Exact design: the same 3 bytes per face in an integer R8 2D texture read with `texelFetch`, written when the mesh
   rebuilds; needs copies of `CLOUDS`, `FLAT_CLOUDS` and the two OIT sets. Expected ~4-5% mixed static after candidate 1. Not done.
3. Radix sort in `SectionFaceGroups.sortByPlanes`: done (below).
4. `InstancedModels` per-group overhead (~1.4 us per group per frame; ~15 us after candidate 1): one shared instance ring, `prepare()` cached
   per render type, no state rebinding between groups sharing a pipeline, or one indirect multi-draw per pipeline. ~2% after 1. Not done.
5. Screenshot readback (`Screenshot.takeScreenshot`, 20-40 ms per F2 and on a new world's first join): an exact bulk row copy, ~1-3 ms. Not done.
6. Small per-entity items, not measured individually: a front cache for `RenderTypes.entityCutout`'s memoize lookups; skipping the empty-stack
   enchantment iteration in `CrossbowItem.getChargeDuration` (trade-off: bypasses mods hooking `modifyCrossbowChargingTime` for empty stacks).
7. Trade-off idea only: skip extraction and submission of unchanged armor stands (not exact for mods).

**Dead ends:** retained GPU work and hidden sign text (CPU bound, nothing to gain); closed books on the GPU (not exact: JOML's `rotateY` uses
`Math.sin`, and reusing rotations across tables works only because the bench places them in one tick); the tick (long tail); vanilla's join
work (packets, light, occlusion, scene construction; builds are ~85% JIT-cold; spreading builds was binned in the nineteenth session);
particles (the 500k packet 94 ms + 22 ms drain; lowering the cap changes vanilla's acceptance odds); the frame series' block entity column
reads 0 in the mixed scene (everything is retained or instanced; harmless).

**New diagnostics** (all in `scripts/patches/s25-diagnostics.patch`, not committed): `-Pbench.slowFrameMs=N` (`SlowFrameProbe`: each slower
frame's profiler tree), per-packet timing, destroy queue contents, retained build timers per phase (logged for builds over 3 ms), profiler
sections `ruthenium:retainedBuild`/`prepareDraws`/`retainedDraw`/`instancedDraw`, `InstancedModels` counters every 4096 frames,
`-Pjvm.ruthenium.s25.rebuildEvery=N` (every retained batch rebuilt every N frames: warm build times), mixed scene options `bench.s25NoClouds`,
`bench.s25SameYaw` and a `zoom0` shot with `zoomShot`.

**Armor stand base plate as its own instance** (`46a7390`, switch `armorStandPlates`, "Separate Armor Stand Plates", needs Instanced Models).
`ArmorStandModel.setupAnim` turns the plate back by the stand's yaw (it keeps its world orientation). With the switch on,
`InstancedModels.tryAdd(Submit)` hides and unturns the plate (`visible`, `yRot`) while the body's snapshot is taken and its group found
(its mesh is then the body without the plate), restores them, and adds the plate to a per-model plate group (`PLATE_KEY`): its mesh is the
plate alone at its scale (offset and rotation zeroed by hand and restored; `storePose`/`loadPose` would drop the scale: the prototype's
trap), recorded in the group; its instance is the submit pose times the root's and the plate's `translateAndRotate` with the plate's scale
set to 1 for the call (a rotation; `isOrthogonal` checked). A plate whose scale differs from its group's, a non-rotation, or a full group
cap takes vanilla's path for the whole stand. Both groups are found before any instance is added; both use the submit's light. The plate is
read through an accessor of `ArmorStandModel.basePlate` (no lookup per submit). Groups are dropped at the next frame when the switch
changes, also when it is flipped by reflection (`ToggleAb`, close-ups), since the groups' keys differ between the settings. In the mixed
scene: 7 groups for 508 instances, 0 fallbacks (the investigation's counters on the prototype).

Harness: the mixed scene got `bench.toggleField` (`ToggleAb` after the particle phase, once the particles are gone) and frozen close-ups
(`bench.closeShots`, `bench.closeToggle`, comma-separated fields allowed, clouds off): `close0` the grid's first rows, `close1` the grid
from above (small stands in rows 3 and 7), `close2`/`close3` 13 stands summoned for them behind the camera (arms and poses, small, no base
plate, invisible, invisible small, marker, small marker with arms; 1.25 blocks apart so plates don't overlap) from the front and from above.
Neither changes the measured phases.

*Exactness* (frozen close-ups, 8 shots per position, `scripts/StableDiff.java`; `s25-dev/plate-parity`, logs `2102-2104`):

| close-up | on vs off, 1 level | on vs off, 2 levels | vanilla path\* vs switch on, 1 / 2 levels | vanilla path\* vs switch off, 1 / 2 levels |
|---|---|---|---|---|
| close0 (grid, front) | 24817 | 0 | 251567 / 3 | 272844 / 3 |
| close1 (grid, above) | 24631 | 0 | 220966 / 8 | 239902 / 8 |
| close2 (variety, front) | 3377 | 0 | 23332 / 6 | 43543 / 157 |
| close3 (variety, above) | 1499 | 0 | 10834 / 1 | 19885 / 117 |

\* `closeToggle=instancedModels,instancedParts`: every other shot draws the stands through vanilla's writes. Across the switch, every
differing pixel is 1 level and on a plate's side face: the plate's normal is quantized to bytes before the instance's rotation (axis-aligned,
exact) instead of after the stand's yaw and the counter-turn (switch off: quantized in a diagonal direction, a shorter vector after the
rotation; vanilla: quantized after both, close to axis-aligned). Against vanilla's path the switch on has fewer 1-level pixels than off in
every close-up and none of off's 2-level pixels (157 and 117: plate sides at diagonal yaws). The ~0.25M 1-level pixels against vanilla's path are the existing
instancing's rounding of the bodies (section 4, `InstancedModels`: "the normal is quantized before the transform instead of after it"); the
large-difference pixels shared by both runs (close0 3, close1 8) are silhouette edges.

**Not explained:** 7 pixels differ from vanilla's path by 8-55 levels in the switch-on run only (close2: 5 px on a texel edge between body
and shoulder of the small marker stand with arms, 1 px on the invisible stand's ghost; close3: 1 px), while the switch-off run has them as
vanilla does. They don't follow the switch's current state: a launch starting with the switch off and toggling it (`2133`, 4 shots: off, on,
off, on) shows vanilla's values in its first shot only and the switch-on values in the three later shots, the second switch-off shot
included; a launch with the switch off throughout that toggles only `instancedModels` (`2134`; that toggle doesn't clear the groups) shows
vanilla's values in all shots. So they follow when the groups were built (during the benchmark, or at the close-ups, which a change of
`armorStandPlates` forces), not the plate split itself; the mechanism (some state in the group mesh build or the draw that depends on the
build's moment) was not found. Open; reproduce with `-Pbench.closeShots=4 -Pbench.closeToggle=armorStandPlates -Pvm.armorStandPlates=false`
and `scripts/StableDiff.java` at threshold 3 on `close2` (or compare pixel 902,543 across the shots).

*Numbers* (switch off -> on, fps; render thread ms per frame):

| | static | pan | initial | particles |
|---|---|---|---|---|
| mixed, 3 interleaved pairs (`s25-plate-mixed`) | 1599.2 -> 1799.5 (every pair), p99 781.0 -> 845.6, 0.54 -> 0.42 | 1651.6 -> 1715.4 (every pair), 0.44 -> 0.37 | 1114.1 -> 1270.8 | 1281.3 -> 1432.2 |
| mixed, in-launch, 8 rounds (`s25-plate-toggle-mixed`) | 1632.9 -> 1827.5 (8 of 8), 0.538 -> 0.419 | 1694.2 -> 1742.5 (8 of 8), 0.429 -> 0.384 | | |
| entities, in-launch, 8 rounds (`s25-plate-toggle-entities`; no armor stands) | off/on 1.0017 (7 of 8; mean 1.0023) | off/on 1.0019 (4 of 8) | | |

The prototype's numbers (investigation, 4 pairs: static ~1600 -> ~1800 every pair, render thread 0.56 -> 0.42) reproduced. The upper bound
(every stand at yaw 0, `bench.s25SameYaw`: 1594 -> 1829) is about the same: the plate split recovers nearly all of the grouping loss.

**Radix sort in `SectionFaceGroups.sortByPlanes`** (`cea8b4b`, no switch: exact by construction, internal to the mod's own retained path,
like the twenty-fourth session's texture change). The keys (group << 52 | sortable plane << 20 | quad index) arrive in ascending quad
order, so a stable LSD radix sort of bits 20..55 (three 12-bit passes after one histogram pass, a pass skipped when its digit is the same in
every key) gives `Arrays.sort`'s order. The reordered quads are also gathered straight into the buffer that `createBuffer` uploads
(`sortByPlanes(mesh, target)`; the mesh stays untouched) instead of a scratch copy copied back into the mesh: the optional item of the
report, simple and measurable. Warm builds (temporary timers, every batch rebuilt every 40 frames, medians of the builds after the first
third of each run, ABBA, `s25-radix-time`, `s25-gather-time`):

| step | 2048 block entities | 1536 | 520 |
|---|---|---|---|
| `Arrays.sort` -> radix: sort step | 1.50 -> 0.28 ms | 1.07 -> 0.21 | 0.35 -> 0.08 |
| `Arrays.sort` -> radix: build | 6.27 / 6.20 -> 5.69 / 4.94 | 4.67 / 4.66 -> 4.27 / 3.69 | 1.55 / 1.55 -> 1.44 / 1.24 |
| copy back -> direct gather: build | 5.09 / 5.07 -> 4.63 / 4.68 | 3.70 / 3.78 -> 3.46 / 3.50 | 1.26 / 1.27 -> 1.19 / 1.20 |

(The report's 6.68 -> 5.50 ms and sort 1.6 -> 0.35 reproduced within run-to-run spread; build medians move ~0.5 ms between runs of one side,
the sort step doesn't.) Exactness (`scripts/patches/s25-radix-check.patch`, `-Pjvm.ruthenium.radixCheck=true -Pjvm.ruthenium.rebuildEvery=40`,
`s25-radix-check`): 10515 sorts over 185.9M quads, and with glowing signs and chest events 19470 sorts over 343.9M quads, 0 mismatches; in
that run the gathered bytes and groups also equalled the old in-place path's in 9735 of 9735 builds (a temporary check not in the patch).
Mixed scene, commit A/B `46a7390` vs `cea8b4b`, 3 pairs (`s25-radix-mixed`): static 1801.8 vs 1811.2, pan 1718.2 vs 1709.5, initial 1259.2
vs 1264.4 (p99 309.8 vs 334.9), particles 1433.9 vs 1460.8: no regression. The scene's builds happen cold, after the join (~85% JIT), so
the gain shows in warm rebuilds (block changes, a section's first view in a pan), not in the initial phase.

**Files:** `results/s25-plate-mixed`, `s25-plate-toggle-{mixed,entities}`, `s25-dev/plate-parity`, `s25-radix-time`, `s25-gather-time`,
`s25-radix-check`, `s25-radix-mixed`; logs `build/bench-logs/2102-2125` and `s25-*.out`. Patches: `s25-diagnostics.patch` (against
`681f1f2`; it touches `InstancedModels` and `SectionFaceGroups`, so expect conflicts after this session), `s25-AllocSites.java.txt`,
`s25-radix-check.patch` (against `cea8b4b`). The prototypes `s25-armor-stand-plate.patch` and `s25-radix-sort.patch` are superseded by the
commits.

**Found on the way:** results of plain runs left in `results/` are swept into side a by the next `run-ab.sh` (it moves every `ruthenium-*`
file; here three close-up runs landed in the plate A/B's side a and were moved out by timestamp). Move them out first (section 6.4).

Still open after the first builder: candidates 2 (clouds without a texel buffer), 4 (per-group overhead), 5 (screenshot readback) and 6 of
the list above; the OpenGL backend was not measured for either change. Part 2 below did 2, 4 and 5.

#### Part 2: clouds, instanced groups, screenshots (second builder)
A second builder agent on `26.3` (main tree), after the first, every Gradle command under the GPU lock. Commits `1897102`, `7193e1d`,
`5307cfd`. Results in `results/s25b-dev/` (parity maps, check runs), `s25b-cloud-toggle-mixed`, `s25b-cloud-mixed` (A/B),
`s25b-cloud-toggle-gl`, `s25b-groups-toggle` (all task 2 runs with their logs), `s25b-shot-time`; logs `build/bench-logs/2135-2165`,
`s25b-*.out`.

**Clouds without a texel buffer** (`1897102`, switch `cloudFaceTexture`, "Cloud Face Texture"; candidate 2). With the switch on,
`CloudRenderer.buildMesh` writes its faces into a scratch buffer (`@ModifyArg` on the call in `prepare`), which is copied into vanilla's
mapped texel buffer as it would have been written (vanilla's state is unchanged and the switch can flip at any frame) and uploaded with
`writeToTexture` into an R8_SINT 2D texture, 1024 texels wide, byte i at (i % 1024, i / 1024), whole rows; only when the mesh rebuilds (cell
change, camera crossing the layer, cloud setting change; turning the switch on forces one). The draw (`render(RenderPass, RenderPipeline)`,
used by `render` and every OIT stage) uses copies of `CLOUDS`, `FLAT_CLOUDS` and the three pipelines of `OIT_CLOUDS` and
`OIT_FLAT_CLOUDS`, whose `CloudInfo` bind group layout has `CloudFaces` as a `COMBINED_IMAGE_SAMPLER` (nearest, clamp), and the vertex
shader `ruthenium:core/clouds` (vanilla's with `texelFetch(isampler2D, ivec2(i & 1023, i >> 10), 0).r`: the same sign-extended ints and
arithmetic). One texture suffices as with the label poses (twenty-fourth session): the copy is recorded outside render passes, which end
with a full barrier on Vulkan; OpenGL orders `glTexSubImage2D` itself. A copy that isn't compiled leaves vanilla's draw.

*Exactness:* frozen cloud shots (`bench.cloudShots=8`, `bench.closeToggle=cloudFaceTexture`; ticks frozen, so all 8 shots of a view are
identical unless the switch changes them), `StableDiff` at threshold 1: **0 px in all 8 views on Vulkan and on OpenGL** (fancy below,
inside, above; fast below and above; improved transparency fancy below and inside, fast below). A temporary log showed every copy drawing
(`clouds`, `flat_clouds`, `oit_{depth_bounds,transmittance,accumulate}_{clouds,flat_clouds}`) on both backends.

*Numbers* (switch off -> on):

| | static | pan | initial | particles |
|---|---|---|---|---|
| mixed, in-launch, 8 rounds, light profiler (`s25b-cloud-toggle-mixed`) | 1781.0 -> 1827.5 (8 of 8), 0.444 -> 0.401 ms | 1729.0 -> 1733.6 (5 of 8; GPU bound) | | |
| mixed, 3 interleaved pairs (`s25b-cloud-mixed`) | 1786.7 -> 1847.6 (every pair), p99 805.2 -> 810.0, 0.45 -> 0.39 ms | 1712.0 -> 1731.4 (every pair), 0.40 -> 0.36 | 1280.0 -> 1350.3 | 1446.3 -> 1491.7 |
| OpenGL mixed, in-launch, 8 rounds (`s25b-cloud-toggle-gl`) | 1575.4 -> 1604.2 (7 of 8), 0.617 -> 0.612 ms | 1617.5 -> 1636.3 (7 of 8) | | |

Light profiler, on minus off, both phases: `submit/queueSubmit` **0.0457 -> 0.0240 ms (-21.7 us)**, `executeFrameGraph/main` -3.5 us (the
`VkBufferView` per push), `present` -4 us; in the pan `awaitGpu` grows by as much (GPU bound there), in the static view the frame is 15 us
shorter. The report's estimate (~4-5% static) was a little high; +3.4% measured.

**`InstancedModels` per-group overhead** (`7193e1d`, no switch: exact, the mod's own draw path; candidate 4). Every group's instances
now go into one ring buffer per frame (a slice per group), each render type is prepared once per frame (`RenderType.prepare` writes a
dynamic transform and builds a texture list each call; only the textures are used), and the draws keep the pipeline's bindings while the
pipeline and the prepared render type stay the same, as `InstancedParts.draw` already did. Since a Vulkan `setUniform` marks the descriptors
dirty even with the same value, the old per-group rebinding pushed descriptors (a `VkWriteDescriptorSet`) for every draw; now only on a
pipeline or texture change. Measured with temporary switches and `ToggleAb` plus temporary profiler sections around `prepareDraws` and
`draw` (light profiler at depth 9), each part off with the others on:

| part (65 groups: `armorStandPlates=false`, 244 instances; rounds 0-2) | draw | prepareDraws | static fps off -> on | render thread alloc MB/s |
|---|---|---|---|---|
| all three | 36.6 -> 7.5 us | 17.1 -> 12.7 us | per round 1733/1699/1721 -> 1762/1774/1750 | 318 -> 256 |
| bindings kept | 46.9 -> 7.4 | = | 1679.5 -> 1720.9 (3 of 3) | 246 -> 238 |
| one prepare per render type | 20.3 -> 7.1 (shared textures let the bindings stay) | 18.9 -> 12.0 | 1728.2 -> 1739.8 (3 of 3) | 291.6 -> 241.4 |
| one instance ring | 8.1 -> 6.9 | 16.0 -> 12.2 | 1732.1 -> 1748.7 (2 of 3) | 258.4 -> 242.5 |
| all three, default (plates on, 7 groups, 508 instances), 8 rounds | 5.7 -> 2.9 | 8.3 -> 6.9 | off/on 0.9990 (noise) | 251 -> 246 |

So the uninstrumented cost was ~0.8 us per group (53.7 us for 65 groups), mostly the draws' rebinding, not the ~1.4 us the investigation's
per-step timers gave; at the default 7 groups the change saves ~4 us per frame (not resolvable in fps). All three parts measured in their
sections or in allocation, so all were kept. *Exactness:* frozen close-ups (`closeShots=8`, toggling all three), `StableDiff` threshold 1:
0 px in close0-3 on Vulkan, OpenGL and Vulkan with plates off. The entity scene has no `InstancedModels` groups (no armor stands or
enchanting tables), and `InstancedParts` already shared its ring and bindings, so the many-groups case was measured in the mixed scene with
plates off.

**Found on the way: stale instanced groups with plates off.** ~60 s after the stands spawn, the server's periodic rotation sync re-sends
their yaws quantized to a byte, so with `armorStandPlates=false` every stand's plate angle (part 9's `yRot` in the snapshot) changes, the
32 groups per model are already full of the old angles (groups are never evicted), and most stands take vanilla's path from then on (1 group
drawn instead of 65; static ~1720 -> ~1460 fps). The default (plates on) doesn't depend on the yaw. Evicting groups unused for some frames
would fix it; not done. It also limits `ToggleAb` with plates off to ~3 static rounds (section 6.6).

**Screenshot readback** (`5307cfd`, switch `fastScreenshots`, "Fast Screenshots"; candidate 5). `Screenshot.takeScreenshot` at full size
with a 4 byte color format: the read back frame is copied two pixels per long (`memGetLong | 0xFF000000FF000000`, row `height - 1 - y`),
the same bytes as vanilla's `getInt`/`setPixelABGR` loop on a little-endian machine with a native-order mapped buffer (both checked;
otherwise vanilla's loop). Downscaled shots (panorama) and other formats keep vanilla's path. *Exactness:* check mode
(`scripts/patches/s25-screenshot-check.patch`): 27 of 27 readbacks identical to vanilla's loop on Vulkan and 27 of 27 on OpenGL (the icon,
scene shots, close-ups, cloud shots; 183 to 115k source pixels per frame had alpha below 255, so the OR matters); the PNGs of frozen shots
across the switch are byte-identical files (20 of 20). *Numbers:* the new world's icon readback callback (frame 0), 2 interleaved pairs:
vanilla 23.8 / 32.6 ms, bulk 10.9 / 14.5 ms. The copy alone on the same pixels: first readback 12.0-14.4 ms (vanilla loop) vs 8.3-9.1 ms;
later ones 2.5-3.2 vs 0.7-2.8 ms (the fresh 8 MB image's page faults dominate both). The investigation's 20-41 ms was the whole callback
(map, image, loop), not the loop; frame 0 (110-140 ms) is too noisy to show ~13-18 ms in 2 pairs. The benchmark's own screenshots take
100-190 ms per callback on both paths: the gametest writes the PNG inside the callback.

Still open: the stale groups with plates off (above); candidates 6 (small per-entity items) and 7 (trade-off idea) of the list above; the
seven close-up pixels of the plate change; the plate change and the radix sort on the OpenGL backend (not measured). All are listed in
section 8 item 2. All but candidate 7 were done in the twenty-sixth session (below).

### Twenty-sixth session (2026-10-29/30)
Three subagents worked on the three open items of section 8 item 2, at most two at a time. Every Gradle command ran under the GPU lock.
- **OpenGL measurements:** in the main tree, measurement only.
- **The eviction fix:** in a worktree, branch `worktree-agent-a7e3cdc31ceddcbd0`, merged as `d86fbab`; its results and logs are in that
  worktree's `build/`.
- **Candidate 6:** in the main tree after the OpenGL agent finished.
Commits: `36bb953`, `b6260b7` (eviction), `30f72b6`, `a9c1d7f` (candidate 6). Main tree logs `build/bench-logs/2167-2216`; worktree logs 1-63.

#### OpenGL: the plate change (`46a7390`) and the radix sort (`cea8b4b`)
**Neither regresses on OpenGL. The plate split gains much more there than on Vulkan.**

*Plates.* Same method as on Vulkan: the tree at `46a7390`, `run-ab.sh` with `-Pvm.armorStandPlates=false` against `=true`.

| switch off -> on | static | pan | initial | particles |
|---|---|---|---|---|
| OpenGL, 4 interleaved pairs (`s26-gl-plate-mixed`, logs 2167-2174) | 1202.0 -> 1571.3 (+30.7%, every pair), p99 566.5 -> 664.4, 0.80 -> 0.61 ms | 1440.7 -> 1591.1 (+10.4%, every pair), 0.67 -> 0.61 | 825.6 -> 1011.8 (every pair), 1.18 -> 0.96 | 890.9 -> 1226.3 (+37.6%, every pair), 1.08 -> 0.79 |
| Vulkan, 3 pairs (`s25-plate-mixed`) | 1599.2 -> 1799.5 (+12.5%), 0.54 -> 0.42 | 1651.6 -> 1715.4, 0.44 -> 0.37 | 1114.1 -> 1270.8 | 1281.3 -> 1432.2 |
| OpenGL, in-launch, 8 rounds (`s26-gl-plate-toggle-mixed`, log 2176) | 1228.1 -> 1587.0 (8 of 8), 0.794 -> 0.615 | 1471.1 -> 1619.3 (8 of 8), 0.661 -> 0.594 | | |

- **Why the gain is larger:** the 58 extra groups with plates off cost 0.19 ms of render thread on OpenGL, about 3.3 us per group. On
  Vulkan they cost 0.12 ms, about 2 us per group. Both were measured at `46a7390`, before `7193e1d`.
- **The stale-group bug didn't touch the interleaved runs.** Every phase ended within the ~60 s window after spawn: pan ~33 s, static ~41
  s, particles ~51 s, checked per log.
- **The in-launch toggle dipped once.** Static round 4 ran 60-64 s after spawn, so the sync landed in it.
- **Parity** (`s26-gl-plate-parity`): every close-up count matches Vulkan's table within 1 px. Across the switch, 0 px differ by 2 levels
  or more.

*Radix sort with direct gather.* Each build was timed by a temporary timer, applied after each checkout and reverted after the run.
`-Pjvm.ruthenium.s26.rebuildEvery=40` marked every batch dirty every 40 frames. 2 ABBA pairs; the tools are in
`s26-gl-radix-time/tools/`.

| warm builds, `46a7390` -> `cea8b4b` | 2048 block entities | 1536 | 520 |
|---|---|---|---|
| OpenGL build | 5.78 / 5.74 -> 4.46 / 4.33 ms | 4.31 / 4.30 -> 3.32 / 3.35 | 1.49 / 1.49 -> 1.12 / 1.15 |
| OpenGL `sortByPlanes` call (keys, sort, planes, gather) | 3.33 / 3.34 -> 1.76 / 1.76 | 2.40 / 2.42 -> 1.30 / 1.30 | 0.76 / 0.77 -> 0.42 / 0.42 |
| Vulkan build (twenty-fifth session) | 6.27 / 6.20 -> ~4.65 | 4.67 / 4.66 -> ~3.5 | 1.55 -> ~1.2 |

- **The gain:** -1.37 ms (-24%) per warm 2048 build on OpenGL, against -1.55 ms on Vulkan.
- **With rebuilds every 40 frames:** static 833.9 / 831.1 -> 920.3 / 937.6 fps, both pairs.
- **The commit A/B in fps** (3 pairs, `s26-gl-radix-mixed`, logs 2181-2186): 1542.4 vs 1554.9 static, 1574.1 vs 1594.6 pan, render thread
  equal (0.63 ms). No regression; the scene's own builds are cold.

*Still open on OpenGL:*
- The mixed static view's render thread is ~0.2 ms slower than on Vulkan (0.61 vs 0.42 ms, plates on). It hasn't been profiled.
- `7193e1d` was only checked for exactness on OpenGL, not timed.

*Script quirk:* `run-ab-commits.sh` started from a detached HEAD fails its final `git checkout -q ""`. It's harmless.

#### Stale `InstancedModels` groups evicted (`36bb953`, `b6260b7`; no switch)
**The design** (`InstancedModels.findGroup`):
- Eviction happens only when a model is at its cap (still 32) and needs a new group, and at most once per frame. The model's least recently
  used group goes, if it has been unused for more than `EVICT_AFTER_FRAMES = 1024` frames and has no instances this frame. Below the cap
  nothing is evicted, so models out of view keep their groups.
- **Why one eviction per frame:** a model whose groups all went idle could come back with more keys than its cap. Without the limit it
  would evict and rebuild a group for every key past the cap, in turn, in one frame.
- **What counts as used:** a matching lookup stamps the group, and `prepareDraws` stamps every group with instances once per frame
  (`b6260b7`; `36bb953` stamped per instance). The match stamp also stops a plate lookup from evicting the same stand's body group, which
  was found just before it.
- **Why 1024 frames:**
  - Rebuilds are bounded to 32 per model per 1024 frames, about 1 us per frame at most.
  - That is far more than the frames in flight, so the GPU can't still be reading a closed mesh.
  - Recovery after the sync takes about 0.6 s.
- An evicted group is closed at once. If it is the closed book group, `closedBookGroup` and `closedBookRenderer` are reset.
- **The books can't thrash.** They have one key (`BOOK_CLOSED`), so one group, and the cap is per model. 0 book evictions in every run.
- **No switch.** It's a fix on the mod's own path that only changes which stands get a group.

**Builds in a plates-off run:**
- 195 in all: 65 at the start, 64 at the sync, and about 66 during the 360° pan.
- The pan builds are overflow keys (37 per model against the cap of 32) taking the slots of stands out of view. That is at most 1 per frame,
  20-110 us each, and pan fps doesn't drop.
- With plates on: 0 evictions in every run.

**Numbers** (mixed scene, Vulkan, worktree):
- In the property A/B, the off side was the new code with eviction disabled by a temporary property, removed before the commit.
- The commit A/Bs checked out each commit in turn in the worktree.
- `-Pbench.initialSeconds=75` puts every phase after the sync, which lands ~63 s after spawn. The standard phases all run before it.

| | static | pan | initial | particles |
|---|---|---|---|---|
| plates off, after the sync, property A/B, 3 pairs (`s26-platesoff-late`) | 1526.1 -> 1804.2 (every pair), 0.62 -> 0.45 ms | 1651.3 -> 1713.9 | 1708.5 -> 1765.7 | 1200.9 -> 1442.5 |
| plates off, after the sync, `1a43c7d` vs `36bb953`, 2 pairs | 1520.1 -> 1811.4 (every pair) | 1653.2 -> 1704.1 | 1693.8 -> 1773.9 | 1197.0 -> 1440.4 |
| plates on, `1a43c7d` vs `b6260b7`, 4 pairs (`s26-final-plateson`) | 1837.2 vs 1842.1 | 1721.6 vs 1731.3 (every pair) | 1359.5 vs 1345.6 (noisy join) | 1478.0 vs 1492.2 (every pair) |
| plates on, yaw 270 (closed books), 3 pairs (`s26-final-yaw270`) | 1607.2 vs 1607.4 | | | 1520.3 vs 1518.5 |

- **Group counters after the sync** (temporary probe):

  | | groups drawn | instances | stands on vanilla's path |
  |---|---|---|---|
  | without eviction | 2 | 25 | 241 |
  | with eviction | 65 | 246 | 20 (37 grid keys per model, as before the sync) |
  | cap 64, no eviction | | | 238 (the client's yaw interpolation makes transient keys) |

- **Why the stale state was slower than instancing off:** each fallback stand scanned 65 groups with `Arrays.equals` before taking
  vanilla's path. `ToggleAb` with plates off (`toggleField=instancedModels`): without eviction, off/on was 1.09 from round 1 on; with
  eviction it is 0.92 in all 8 rounds. **`ToggleAb` with plates off is valid past round 3 now.**
- **`b6260b7` exists because `36bb953` measured -0.5% static with plates on** (1849.3 vs 1840.0, old faster in 4 of 5 pairs, two of them
  by under 1 fps): a per-instance store for thousands of books. With `b6260b7`, static, pan and yaw 270 are even or better.

**Exactness** (frozen close-ups after the sync, `s26-close/*`). The harness's plate toggles clear the groups, so only close0's shot 0 shows
the groups the benchmark left.
- **Plates on, across launches with and without eviction:** 0 px in 16 of 16 shots.
- **Plates off:** 0 px in 15 of 16. The exception is close0 shot 0, where the no-eviction run draws the stale stands through vanilla's
  path.
- **Groups rebuilt after eviction against fresh groups,** with the cap raised to 64 for the check so the overflow choice can't differ:
  0 px in all four close-ups.

**The "seven unexplained pixels" are explained** (subagent's analysis, consistent with every log):
- The variety stands in close2 took vanilla's path in the first shot because their model was at its cap, filled with stale or grid keys,
  until a plate toggle cleared the groups. After that they were instanced.
- The 8-55 level differences are the known instanced-vs-vanilla rounding on a texel edge, visible only where those stands are instanced.
- That gives 2133's pattern: pixel 902,543 has vanilla's `7e6642` in shot 0 and `473820` in shots 1-3. It also gives 2134's: no clearing
  toggle, so vanilla's values in every shot.
- The OpenGL run shows the same.

**Left:**
- With plates off, ~20 stands still overflow the 37-key grid. Raising the cap was not measured.
- The close-up harness can't check the groups the benchmark left beyond close0's first shot, because its toggles clear the groups.
- The eviction fix was not measured on OpenGL.

#### Candidate 6: render type front cache (`30f72b6`) and empty crossbow charge time (`a9c1d7f`)
**Both are exact for vanilla and each has a switch. Neither moves the mixed static fps:** that view now waits on the GPU (`awaitGpu` ~0.13
ms of a 0.52 ms frame), and the wait absorbed every microsecond saved. The investigation's +2-4% estimate was wrong. The real gains are
about 3 us of render thread per frame each, plus 9-12% less render-thread allocation.

*The probe* (`scripts/patches/s26-memo-probe.patch`, `s26-c6-probe`) counted `Util.memoize` lookups on the render thread:
- **Mixed scene:** `RenderTypes.ENTITY_CUTOUT` 207 per frame, 1 key. Everything else is below 0.1 per frame.
- **Entity scene:** `ENTITY_CUTOUT` 455 per frame. That is 87 equal keys, but more than 4096 distinct `Identifier` objects, because
  villager clothing and other textures build them every frame.
- **One-argument memos** (`ENTITY_SOLID_Z_OFFSET_FORWARD` 151 per frame, `ARMOR_CUTOUT_NO_CULL` 99, `EquipmentLayerRenderer`'s ~110) don't
  allocate: they hit `ConcurrentHashMap.computeIfAbsent`'s first-node fast path. They were left alone.
- **Two-argument memos allocate on every call.** `Util$11.apply` builds a `Pair`, a lambda and the `Object[]` for its hash. That was 6.6%
  of render-thread allocation in the twenty-fifth session's JFR.
- **The crossbow:** `CrossbowItem.getChargeDuration`'s enchantment iterator was 6.8%, called from
  `HumanoidMobRenderer.extractHumanoidRenderState` for armor stands, humanoid mobs and players.

*Design:*
- **"Render Type Lookup Cache", `renderTypeCache`.**
  - A `@ModifyExpressionValue` wraps every `Util.memoize(BiFunction)` in `RenderTypes.<clinit>` in a `RenderTypeFrontCache`:
    `ENTITY_CUTOUT`, `_Z_OFFSET`, `_DISSOLVE`, `ENTITY_TRANSLUCENT`, `_EMISSIVE`, `BEACON_BEAM` and `OUTLINE`.
  - The cache is two-way set associative: 512 sets of immutable `(first, second, result)` records.
  - A hit needs `==` or `equals` on both arguments, `Pair.equals`' test. `null` arguments and misses go to vanilla's memo.
  - Each entry is written with one reference store, so it is thread safe.
  - A direct-mapped first version missed 11% of lookups in the players scene. The two-way version misses 76 of ~1M there.
- **"Skip Empty Crossbow Charge Time", `emptyCrossbowCharge`** (on by default; the user's decision, 2026-10-29).
  - A `@WrapOperation` on the `getChargeDuration` call in `extractHumanoidRenderState` returns 25 (`Mth.floor(1.25F * 20F)`) when
    `stack.isEmpty()`. Otherwise it calls vanilla.
  - This is exact for every empty stack: their `getComponents()` is `DataComponentMap.EMPTY`, so even a count-0 enchanted crossbow
    iterates nothing.

*Exactness* (`scripts/patches/s26-c6-check.patch`): every cache hit also asked vanilla's memo, and every skip also called vanilla.

| run | render type hits | of them with an equal but not identical key | crossbow calls | mismatches |
|---|---|---|---|---|
| mixed | 14.9M | 1 | 15.0M | 0 |
| entities | 6.0M | 1.54M | 1.01M | 0 |
| players | 0.97M | 71k | 2.0M | 0 |
| entities with AI (first version) | 4.0M | 0.57M | 0.38M (9784 non-empty went to vanilla) | 0 |

Synthetic stacks check the result itself: vanilla returns 25 for `EMPTY`, for air and for a count-0 Quick Charge III crossbow, and 10 for
the count-1 control.

*Numbers* (switch off -> on, in-launch, 8 rounds; KB/frame is render-thread allocation, lower in 8 of 8 everywhere):

| | static fps | KB/frame | light profiler |
|---|---|---|---|
| `renderTypeCache`, mixed | 1854.4 vs 1853.3 (4 of 8) | 126.2 -> 111.4 | `submitFeatures` -3.2 us, `awaitGpu` +3.8 us |
| `renderTypeCache`, entities | 356.4 -> 361.5 (+0.9%, 7 of 8) | 2085 -> 2027 | |
| `renderTypeCache`, OpenGL mixed | 1616.0 -> 1638.1 (+0.9%, 6 of 8) | 132.8 -> 117.6 | |
| `emptyCrossbowCharge`, mixed | 1852.0 vs 1853.4 (4 of 8) | 111.7 -> 100.9 | `extract/level/entities` -2.8 us, `awaitGpu` +3.2 us |
| `emptyCrossbowCharge`, entities | 363.9 vs 363.5 (noise) | 2034 -> 2015 | `extract/level/entities` -4.8 us |
| `emptyCrossbowCharge`, OpenGL mixed | 1624.2 -> 1671.0 (6 of 8; the off rounds drifted, so read it as no loss) | 112.5 -> 101.1 | |

**Both switches together, mixed, 3 interleaved pairs** (`s26-c6-ab-mixed`, logs 2210-2215):
- static 1843.9 -> 1864.7 (one pair equal, two higher)
- initial 1376.3 -> 1405.5 (every pair)
- particles 1496.2 -> 1508.8
- pan 1738.8 vs 1736.1
- static allocation ~235 -> ~205 MB/s; static GCs 6/9/8 -> 5/6/5

Mod compatibility notes are in section 8.

### Twenty-seventh session (2026-10-30): render-thread allocation
A subagent in the main tree, alone on the machine, every Gradle command under the GPU lock. **12 commits, `57568aa`..`fbc5245`.** Results are
in `build/run/bench/results/s27-alloc-*`, logs in `build/bench-logs/2217-2316`. The check patches are `scripts/patches/s27-*-check.patch`.

**Method:**
- **Ranking:** JFR allocation samples of the 20 s static phase, with the throttle raised:
  `-PbenchJfrPeriod=20ms,jdk.ObjectAllocationSample#throttle=2000/s`.
- **Confirmation:** each fix was confirmed with `ThreadMXBean` render-thread bytes in `ToggleAb`, 8 rounds (KB/frame = render-thread MB/s ×
  1000 / fps).
- **No-switch changes** were measured through a temporary `Config` field, removed before the commit.
- **Exactness:** check-mode patches, and frozen close-ups (8 shots, `StableDiff` at threshold 2). The pools were also checked for lifetime:
  last frame's poses poisoned with NaN.
- **New trap:** JFR's allocation ranking also needs a second method. JFR showed a MixinExtras `LocalRefImpl` at 9-12% of mixed allocation,
  but removing it changed nothing in 3 pairs: with JFR off it is scalar-replaced.

**Session start `a3619a4` against `fbc5245`**, static view, interleaved commit A/B (Vulkan 3 pairs, OpenGL 2 pairs, `s27-alloc-final-*`,
logs 2285-2310):

| scene | KB/frame | GCs | fps | render thread ms |
|---|---|---|---|---|
| entities | 2156 -> 1045 (-52%) | 24 -> 13 | 332.7 -> 339.5 (every pair) | 2.91 -> 2.82 |
| players | 1296 -> 747 (-42%) | 21 -> 12 | 476.9 -> 513.3 (+7.6%, every pair) | 2.02 -> 1.88 |
| mixed | 105.2 -> 63.4 (-40%) | 5 -> 4 | 1850.7 vs 1852.1 (GPU bound) | 0.39 vs 0.40 (quantised) |
| entities, OpenGL | 2096 -> 1057 | 20 -> 11.5 | 276.4 -> 279.5 (both pairs) | 3.54 -> 3.48 |
| players, OpenGL | 1299 -> 737 | 16.5 -> 8 | 398.0 -> 427.5 (+7.4%, both pairs) | 2.46 -> 2.25 |

- **Pan KB/frame:** entities 414 -> 227, players 222 -> 191, mixed 125 -> 103.
- **Open: the mixed `initial` (join) phase** read 1386.0 -> 1348.9, lower in all 3 pairs. A follow-up of `a3619a4` against `fe0ac8d`
  (`s27-alloc-initial-mixed`, logs 2311-2316) gave 1414.3 vs 1392.8, lower in 2 of 3. That phase spreads about ±3% run to run, so this is
  unresolved, not a confirmed regression.

**Where the allocation went** (JFR share of render-thread allocation; logs 2217-2219 before, 2273-2275 after):

| site | entities before -> after | players before -> after | mixed before -> after |
|---|---|---|---|
| total KB/frame (JFR run) | 2320 -> 1140 | 1383 -> 772 | 111 -> 66 |
| Indigo scratch mesh (`setupMesh`) | 20.2% -> 0 | – | – |
| submit pose copies | 14.7% -> 4.4% (mostly the Fabric block model copy, then removed by `bcec9f5`) | 9.2% -> 0.2% | 32.6% -> 1.0% |
| text/name tag `Matrix4f` | 0.5% -> 0.4% | 9.5% -> 8.0% (the submit records and `Content$Text` are left) | – |
| `ModelPart` children iterator | 6.1% -> 0 | 9.9% -> 0 | – |
| `getNameTag` (display names) | 0.5% -> 0 | 11.2% -> 0 | – |
| light probe `Vec3`/`BlockPos` | 3.3% -> 0 | 1.8% -> 0.7% | 5.3% -> 8.5% (armor stands, vanilla path) |
| `InstancedParts` slices | 2.9% -> 1.1% | 0.8% -> 0.3% | – |
| `SubmitNodeStorage.order` lambda | 1.4% -> 0 | 1.9% -> 0.1% | 3.6% -> 0 |
| `TerrainDrawList` section info | 0.3% -> 0 | 0.6% -> 0 | 9.9% -> 0 |
| posed `addVertex` `Vector3f` | 1.9% -> 0 | 1.4% -> 0 | – |
| render states (`createRenderState`) | 28.2% -> 52.0% | 21.4% -> 36.1% | 1.2% -> 1.9% |
| `RenderType.prepare` | 3.5% -> 7.1% | 7.6% -> 13.2% | 1.4% -> 2.7% |
| submit records (`submitModel`) | 3.2% -> 6.4% | 2.4% -> 4.3% | 10.1% -> 17.4% |
| Fabric `permitArmorWithCustomRenderers` `Object[]` | – | – | 6.2% -> 9.5% |

The old list in section 8 item 3:
- the chest openness `Float` boxing no longer shows;
- `PoseStack.Pose` copies are pooled;
- the `CompactVectorArray` per sort is gone;
- the submit records and glyph instances remain (immutable records).

**Commits.** Numbers are in-launch `ToggleAb`, static view, 8 rounds, off -> on. Close-ups are 0 stable px unless noted. Every switch is on by
default.

1. **`57568aa` Skip the empty Fabric mesh of cached entity block models.** Switch "No Scratch Meshes for Cached Models" (`noScratchMesh`).
   - **Design:** Fabric's `update` overwrite calls `setupMesh`, which allocates a `MutableMeshImpl` for every new render state, every
     frame. For a model already in `BlockModelMeshCache`, the state is set up as `setupMesh` would, without a mesh. A merged-method inject
     on `setupMesh` tracks the states that might hold a Fabric mesh. `isEmpty` counts the cached mesh, and the model gets a shared seeded
     random.
   - **Numbers:**
     - Vulkan entities: 361.0 -> 372.3 fps (+3.1%, 8 of 8), 2.710 -> 2.615 ms, 2106 -> 1660 KB/frame, GCs 128 -> 99 (log 2220).
     - OpenGL: 284.2 -> 293.9 (+3.4%, 8 of 8), 2203 -> 1767 KB (log 2223).
2. **`a721c96` Render model part children from an array.** Switch "Model Part Child Arrays" (`partChildArrays`).
   - **Design:** `@Overwrite` of `ModelPart.render` with vanilla's body. With the switch on, children come from an array taken at
     construction, used while the map's size still matches.
   - **Check:** `s27-child-check.patch` gave 0 mismatches and 0 fallbacks over 55M renders each in entities and players.
   - **Numbers:**
     - entities: 370.4 -> 378.4 (7 of 8), 1739 -> 1574 KB (log 2227)
     - players: 478.5 -> 494.7 (+3.4%, 8 of 8), 1321 -> 1200 (log 2228)
     - OpenGL players: 415.2 -> 425.6 (8 of 8), 1284 -> 1162 (log 2231)
3. **`2343806` Take submit pose copies from a per-frame pool.** Switch "Pooled Submit Poses" (`pooledSubmitPoses`).
   - **Design:** a `WrapOperation` on vanilla's `Pose.copy` calls in `SubmitNodeCollection` returns pooled poses, set with `Pose.set`. The
     pool is freed at the head of `Minecraft.renderFrame`. It is render-thread only and capped at 65536.
   - **Check:** `s27-pose-check.patch` poisons last frame's poses with NaN. Close-ups: 0 stable px in all three scenes.
   - **Numbers:**
     - entities: 375.1 -> 381.1 (7 of 8), 1575 -> 1304 KB (log 2232)
     - mixed: 1857.5 vs 1867.0, 0.410 -> 0.385 ms, 100.4 -> 65.5 KB (-35%), GCs 22 -> 13 (log 2233)
     - players: fps neutral, 1211 -> 1094 KB
     - OpenGL entities: fps neutral, 1582 -> 1311 KB
4. **`fe0ac8d` Pool the matrix copies of text, name tag and shadow submits too.** Same switch as `2343806`.
   - **Design:** wraps `new Matrix4f(pose)` in the name tag, text, shadow, leash and moving block submits.
   - **Limit found:** MixinExtras' `WrapOperation` doesn't reach methods that another mixin merged (Fabric's submits), named or wildcarded.
     This was checked in the exported class.
   - **Numbers:** players 498.9 -> 502.8 (6 of 8), 1165 -> 982 KB (log 2242); OpenGL players 1210 -> 1027 KB.
   - **Check:** poison close-ups are 0 stable px apart from the obfuscated label.
5. **`faa4045` Return a submit order's existing collection without a lambda.** No switch.
   - **Design:** an inject at the head of `SubmitNodeStorage.order`. Its `CallbackInfoReturnable` is scalar-replaced.
   - **Numbers:** mixed 61.0 -> 57.0 KB; entities 1300 -> 1266 KB, 375.3 -> 377.3 fps (6 of 8).
6. **`d2c512e` Keep the vertex buffer slices of instanced part draws.** The mod's own code, no switch. Each batch keeps its mesh slice, and
   instance slices are kept per draw command, 4 per ring buffer. Entities 1263 -> 1209 KB, fps neutral.
7. **`c71567f` Keep each section mesh's terrain section info record.** The mod's own code, no switch. Mixed 61.9 -> 51.7 KB.
8. **`a121d7c` Transform posed vertices and normals into a scratch vector.** No switch.
   - **Design:** `BufferBuilderM` overrides the `addVertex(Matrix4fc, …)` and `setNormal(Pose, …)` defaults, and `LazyVertexBuilder`
     delegates to them.
   - **Numbers:** entities 378.4 -> 380.9 (8 of 8), 1206 -> 1163 KB; players 986 -> 967 KB.
9. **`d99bd78` Find entity light without a new probe vector and block position.** Switch "Light Probe Without Garbage"
   (`noAllocLightProbe`).
   - **Design:** for classes that keep `Entity.getLightProbePosition`, the eye position is computed with the same double operations, and
     `BlockPos.containing` returns a scratch `MutableBlockPos`.
   - **Check:** `s27-probe-check.patch` gave 0 mismatches over 12M probes.
   - **Numbers:** entities 1191 -> 1091 KB, players 981 -> 947 KB; fps within noise.
10. **`e092159` Keep entity name tags while their display name's inputs are the same.** Switch "Keep Display Names" (`keepDisplayNames`).
    - **Design:** the kept name tag is returned without calling `getNameTag` while all of these are the same objects: the team, its prefix,
      suffix and color `Optional`, the custom name, the type and the UUID. This applies only to classes that keep vanilla's name and team
      methods, checked per class.
    - **Check:** `s27-name-check.patch` gave 0 mismatches over 3M kept name tags.
    - **Numbers:**
      - players: 508.8 -> 529.0 (+4.0%, 8 of 8), 1.910 -> 1.820 ms, 890 -> 736 KB (log 2269)
      - OpenGL: 421.5 -> 436.8 (+3.6%, 8 of 8), 944 -> 790 KB (log 2272)
11. **`bcec9f5` Pool the pose copy of cached entity block models' Fabric submits.** Same switch as `2343806`.
    - **Design:** `BlockModelRenderStateM` marks the next copy, and an inject in `Pose.copy` (`PoseCopyM`) serves it from the pool.
    - **Numbers:** entities 1079 -> 1037 KB, fps neutral. Poison close-ups 0 stable px, at up to 4334 poses per frame.
12. **`fbc5245` Decode sorted draws' centroids into an array kept per size.** No switch. Entities 1048 -> 1028 KB.

**OpenGL coverage:** `ToggleAb` for switches 1, 2, 3, 4 and 10. The other commits were covered on OpenGL only by the final combined A/B. The
settings screen check passed (log 2284).

**Binned** (`scripts/patches`, with README rows):
- `s27-light-localref.patch`: the JFR-only `LocalRefImpl`. No change in 3 pairs.
- `s27-shadow-scratch.patch`: a scratch vector in `ShadowFeatureRenderer.shadowVertex`. -15 KB per frame in players, but fps was not
  resolvable: lower in 6 of 8 rounds in one launch, even in the other.

**Not built:**
- Fabric's merged mesh submits: `WrapOperation` can't reach their copies, so only the block model case got the copy marker.
- The `fastCulling` / `keepBelowName` `original.call` boxing, about 1.5% in JFR: fixing it would bypass other mods' wrappers.

### Twenty-eighth session (2026-10-30): the mixed `initial` phase over the twenty-seventh session
A short measurement task in the main tree. Two other subagents worked alongside it: the OpenGL render thread gap in a worktree, then the
mixed scene's GPU time. Every run was Vulkan, mixed scene, interleaved ABBA, under the GPU lock. Results are in
`build/run/bench/results/s28-initial-*`, logs `2317-2408`. Nothing was committed.

**Verdict: noise.** Any phase-level effect is at most ~1%, below what these A/Bs can resolve. It doesn't localise to a commit range or a
profiler section.

| A/B (initial phase) | dir | pairs | A | B | B/A | B lower in |
|---|---|---|---|---|---|---|
| `a3619a4` vs `fbc5245` (twenty-seventh session final) | `s27-alloc-final-mixed` | 3 | 1386.0 | 1348.9 | -2.7% | 3 of 3 |
| `a3619a4` vs `fbc5245` | `s28-initial-ab` | 8 | 1392.9 | 1373.7 | -1.4% | 6 of 8 |
| `a3619a4` vs `fbc5245`, repeat | `s28-initial-ab2` | 8 | 1381.0 | 1383.7 | +0.2% | 3 of 8 |
| all direct pairs pooled | | 19 | | | pair median -0.96%, sd 1.99% | 12 of 19 |
| `a3619a4` vs `fe0ac8d` (commits 1-4) | `s28-initial-bisect-fe0` | 8 | 1382.8 | 1401.0 | +1.3% | 2 of 8 |
| `fe0ac8d` vs `fbc5245` (commits 5-12) | `s28-initial-bisect-5to12` | 8 | 1386.7 | 1384.4 | -0.2% | 5 of 8 |
| HEAD, the five switches off vs on | `s28-initial-switches` | 8 | 1386.4 | 1386.1 | -0.0% | 5 of 8 |

- **Each A/B carries its own bias of about ±1.5%.** The same `a3619a4` vs `fe0ac8d` comparison read -1.5% (3 pairs) and then +1.3% (8
  pairs). Single pairs spread -6.2% to +4.5%.
- **Static and pan are flat or better in the same runs.**
- **The only consistent signal is in the first second after the join frames.** In the 0.5-1.0 s window, B has 6.8% fewer frames (15 of 19
  pairs). That is ~20 frames of ~6900, about 0.3% of the phase.
  - It is the JIT warm-up window, where frames average ~1.6 ms against ~0.55 ms later.
  - It shows up in every split, including switches on vs off on one build, so it can't be tied to a commit.
  - A light profile of just that window (`s28-initial-light`, 6 pairs) showed no section moving consistently.
  - It's probably warm-up of the new code paths; that is inferred, not measured. Not worth a fix.
- **Join frames, GCs and tick frames show no side bias.**

**Method for future initial-phase questions:** compare per-frame CSV time windows, not the phase median. Resolving 1% needs about 20
pairs. Scripts:
- `scripts/init_windows.py`: frames per time window per pair
- `scripts/init_window_split.py`: GC, tick and plain-frame split of a window
- `scripts/ab_pool.py`: pooled pair ratios across A/B dirs
- `scripts/lp_ab.py`: light-profile tree and no-tick-frame diff per side

### Twenty-eighth session (2026-10-30): the mixed static view's GPU time and the frame gap
Main tree, Vulkan and OpenGL, every run under the GPU lock. Results in `build/run/bench/results/s28-gpu-*` (dev runs in `s28-gpu-dev/*`),
logs `2409-2515` (2439, 2440 and 2491 are failed starts, renamed `*-FAILED-*`). Commits `651076a`, `c7490aa`, `7dd8dbf`, `eff5a2c`,
`2603947`, `6e3c05a`, `5a8a0f4`, `1213c12`, `7ab33ac`.

**Session start `6d556a5` vs end `7ab33ac`,** interleaved commit A/B:

| scene | static | pan | particles | initial |
|---|---|---|---|---|
| mixed, Vulkan, 3 pairs (`s28-gpu-final3-vk`) | 1856.5 -> 2290.1 (+23.4%, every pair), p99 822.5 -> 960.5 | 1733.2 -> 1926.4 (+11.1%, every pair) | 1493.5 -> 1775.8 (+18.9%, every pair) | 1382.2 -> 1407.7 (2 of 3) |
| mixed, OpenGL, 2 pairs (`s28-gpu-final3-gl`) | 1643.2 -> 1720.2 (+4.7%, both) | 1649.1 -> 1750.6 (+6.2%, both) | 1279.4 -> 1338.6 | 1050.4 vs 1044.4 |
| block entities, Vulkan, 2 pairs (`s28-gpu-final3-be`) | 1361.6 -> 1333.1 (**-2.1%, both**) | 1851.3 -> 1951.0 (+5.4%, both) | | |

**Where the GPU frame goes.** New probe `-Pbench.gpuTimeline=frame|passes` (`GpuTimelineProbe`, `651076a`): GPU timestamps around every
render pass and inside the main pass, CPU events, and calibrated timestamps that place the GPU marks on `System.nanoTime`. Static view,
Vulkan, before any change (log 2410), us per frame:

| segment | us |
|---|---|
| frame start -> Sky pass (frame clear and writes) | 17.8 |
| Sky | 14.4 |
| opaque terrain (solid + cutout) | 87.8 |
| retained block entities (chests 51k quads; sign text 30k quads) | 71.8 |
| instanced models (armor stands 12.6k quads in 484 instances; books 864 quads) | 67.3 |
| translucent terrain | 1.7 |
| clouds | 7.1 |
| main pass end | 3.3 |
| main pass -> GUI (the GUI depth clear) | 11.7 |
| "GUI before blur" pass (only the vignette at this view) | 27.7 |
| swapchain blit + acquire wait | 50.1 |
| **GPU busy (frame mark -> submit mark)** | **360.9** |

Eliminations (`ToggleAb`, 8 rounds, `scripts/patches/s28-gpu-elimination.patch`, logs 2426-2429): instanced armor stands with culling
+7.3%; retained models skipped +15.2%; vignette skipped +6.8%; retained sign text skipped +0.1% (it costs nothing); retained chests without
the cutout's discard ±0 (log 2443); retained overlay / lightmap fetch removed -4.7 / -3.6 us (logs 2451-2453; not built, the exactness of
`mix` with alpha 1.0 is unproven).

**The frame gap: why 0.52-0.56 ms when the GPU is busy 0.36 and the render thread 0.39.** Static view, log 2410: GPU period 561 us, busy
361, gap between frames 200. The GPU was idle at `vkQueueSubmit2` in 49% of frames; submit to GPU start median 96 us; GPU end to the await
returning median 37 us. `awaitGpu` averages 131 us but is bimodal (median 6, p90 295). Vanilla's `VulkanCommandEncoder.submit` waits for
submit k-1 right after submitting k, so only one frame is ever queued; there is no readback or mapping stall. Binned
(`scripts/patches/s28-frame-pacing.patch`):
- **Three frames in flight** (logs 2411, 2441): removed the wait and the gap, but the frame's own GPU segments grew by as much (frame ->
  blit 313 -> 408 us, blit -> submit 48 -> 118). Static on/off 0.9934, faster in 1 of 8.
- **Busy-waiting on the semaphore** (log 2412): +0.3%. The 40 us stay, so they are on the GPU side.
- **A new `VkSurfaceKHR` after FIFO** (log 2424): no change.

So the gap is GPU-side presentation work plus submit latency. Cutting GPU busy time is the lever, and it converts into fps.

**Harness finding: the present path depends on whether the window's first swapchain was FIFO.** `options.txt` in the run directory starts
with `enableVsync:true`, and the harness turns vsync off only after start: the swapchain goes FIFO, then IMMEDIATE. Once a FIFO swapchain
has existed on the window, every later swapchain stays in a slower present path for the session: CPU `present` ~24 us and GPU gap ~200 us,
against ~76 and ~120. Diagnostics `-Pbench.presentMode=` and `-Pbench.extraReconfigures=`: FIFO -> IMMEDIATE -> IMMEDIATE is slow,
IMMEDIATE -> FIFO -> IMMEDIATE is slow, IMMEDIATE from the start is fast (logs 2420-2423). Mixed static, default vs IMMEDIATE from the
start: 1832 vs 2024/2143 (2 pairs, `s28-gpu-dev/presentab`); on the final build 2065 -> 2338 static and 1820 -> 2263 pan (logs 2448/2449).
OpenGL shows no effect (log 2447 vs 2446). **Not changed:** starting the bench with vsync off would raise every Vulkan baseline by 13-24%
and break comparability with history. **Changed at the user's request:** bench tasks start with vsync off since the commit after `9d389c5` (section 6.4). Players who start with vsync off are in the fast path;
those who turn it off in game stay slow until they restart. In the fast path the deep queue read +7.0% (7 of 8, drifting rounds, log 2441),
unconfirmed by interleaved launches; it adds a frame of latency when GPU bound.

**`c7490aa` Skip the vignette while it is black** (switch "Skip Black Vignette", `skipBlackVignette`, impact MEDIUM). `HudVignetteM` wraps
the blit in `Hud.extractVignette` and skips the quad when the pipeline is vanilla's `VIGNETTE` and the color's RGB is 0: that blend (ZERO,
ONE_MINUS_SRC_COLOR / ZERO, ONE) leaves every pixel unchanged. The color is black in daylight about 30 s after the eye reaches full light.
At the static view the whole GUI pass goes: GPU busy 363.6 -> 329.4 us. Exact: close-ups 0 px at threshold 1 (logs 2430, 2434); a temporary
log confirmed color `ff000000` and the skip alternating. In-launch 8 rounds: Vulkan static 1952.9 -> 2096.8 (+7.4%, 8 of 8), pan 1787.2 ->
1897.8 (+6.2%, 8 of 8) (log 2435); OpenGL static 1637.4 -> 1701.7 (+3.9%, 8 of 8), pan 1667.0 -> 1807.3 (+8.4%, 8 of 8) (log 2437).

**`7dd8dbf` Cull the inside faces of instanced models made of closed opaque boxes** (switch "Cull Closed Model Insides",
`closedModelCulling`, MEDIUM). When a group's mesh is built, `ClosedBoxes` checks that every quad belongs to a closed, convex box wound
outward (6 consecutive quads over 8 exactly shared corners, both triangles wound alike, no corner in front of a face's plane), and that every
texel a face samples with nearest filtering passes the cutout (`TransparentFaces.TextureAlpha.isOpaqueNearest`, for plain textures with
their own NEAREST sampler). Each frame such a group is drawn with the new `CUTOUT_CLOSED` pipeline only if every instance has a positive
determinant and its bounding sphere (times the Frobenius norm, plus 1 block) is clear of the eye. Exact: close-ups 0 px (logs 2433, 2434);
6 of 6 and 13 of 14 groups culled (the 14th not closed). Vulkan static 1977.0 -> 2099.8 (+6.2%, 8 of 8; instanced 68.5 -> 37.9 us), pan
1855.9 -> 1896.5 (+2.2%, 8 of 8) (log 2436); OpenGL static 1762.6 vs 1757.5 (4 of 8; CPU bound), pan 1792.0 -> 1802.2 (6 of 8) (log 2438).

**`6e3c05a` + `7ab33ac` Retained cell culling** (switch "Retained Cell Culling", `retainedCellCulling`, MEDIUM). Opaque entity-pipeline
parts get one builder per 8³-block cell of the section, one draw per cell with its vertex bounds; a cell whose bounds plus 0.5 block lie
outside the frame's cull frustum is skipped (`beginFrame` now gets the frustum). Text and outline sign draws stay one per render type, so
render type order is unchanged. Batches rebuild when the switch flips. `7ab33ac` draws a part's cells facing group by facing group, nearest
cell first, with shared transforms and bindings (cell-index order had cost the block entity scene 448 vs 287 us of retained GPU). Exact: 3
new chest cube close-ups (`2603947`, close4-6, plus a 10-tick wait after close-up toggles); 0 px in close0-6 on Vulkan (logs 2467, 2471,
2494) and OpenGL (logs 2470, 2497). Retained GPU at the mixed static view 71.8 -> 27.7 us.

| in-launch 8 rounds, off -> on | static | pan |
|---|---|---|
| mixed, Vulkan (log 2495) | 2100.2 -> 2329.5 (+10.9%, 8 of 8) | 1901.3 -> 2020.6 (+6.3%, 8 of 8) |
| mixed, OpenGL (log 2498) | 1732.9 -> 1766.3 (+1.9%, 7 of 8) | 1809.8 -> 1861.0 (+2.8%, 8 of 8) |
| block entities, Vulkan (log 2496) | **1459.5 -> 1417.8 (-2.9%, lower in 7 of 8)** | 1962.8 -> 2076.0 (+5.8%, 8 of 8) |
| facing the cube (log 2466) | neutral | |

The block entity scene's static loss comes from dense sections whose cells are all in view: retained ~308 vs ~288 us, render thread +13 us.
Ruled out: draw count, descriptor pushes, nearest-first vs plane order (log 2513). Splitting draws into 16 pieces (logs 2514/2515) made no
difference. `1213c12` adds `bench.toggleField` to the block entity scene; `eff5a2c` / `5a8a0f4` add the patches and README rows. The
settings screen check passed (log 2473).

### Twenty-eighth session (2026-10-30): OpenGL's slower render thread (worktree)
Branch `worktree-agent-a97bddc0c248f6591` on `778932f`, merged in `943c1cf`. Results and logs `0-46` are in that worktree's `build/` (another
agent's matrices ran at the same time, serialised by the lock). Commits `9091a82` (skip the frame's first clear of the main target when the
level clears it), `3bfb9a8` (skip the hand's depth clear when the GUI's depth clear comes first; same switch), `0bd5fbf` (harness:
`GlCommandEncoderProfileMixin` splits OpenGL's submit into `fenceSync`, `awaitGpu`, `transientRotate`, as on Vulkan), `0669364` (patches
`s28-gl-diagnostics.patch` and the binned `s28-retained-multidraw.patch`).

**The gap, ranked.** Light profiler depth 8, static, plates on, mean of 2 runs per backend (`s28-gl-profile`). Vulkan 0.5425 ms/frame,
`awaitGpu` 0.122, so ~0.42 busy; OpenGL 0.6345, `awaitGpu` 0.005. Busy gap ~0.214 ms. OpenGL isn't GPU bound (GPU frame <= 0.444 ms vs
Vulkan 0.360, `-Pbench.gpuFrame`); its render thread is busy for the whole frame. `gl:*` sections at depth 16 (`s28-gl-diagnostics.patch`)
agree.

| OpenGL − Vulkan | us/frame | owner |
|---|---|---|
| present (`SDL_GL_SwapWindow`) 0.106 vs 0.025 ms, all inside SwapBuffers | +81 | driver |
| clears: 4 per frame (frame start color+depth, level clear pass, hand depth, GUI depth), ~13-16 us each vs ~1 us; the first framebuffer bind of a frame costs ~12 us wherever it lands | ~+50 | vanilla's GL backend |
| frame graph passes other than clears: sky +21, main pass begin/end +11, terrain +9 (vanilla); retained block entities +19 (23 draws, 7 pipeline binds, 115 `setUniform`), instanced models +6.5 (7 draws) (the mod) | ~+66 | |
| swapchain blit (`glBlitFramebuffer` 24.9 us) | +16 | driver / vanilla |
| `glFenceSync` | +14 | vanilla |
| GUI draw, tick textures, extract, sound, fog, ... | ~+13 | vanilla |
| Vulkan-only submit work (queueSubmit 26, pool reset 4, transient 2, destroy queue 1) | −34 | |

GL call costs: `glDrawElements` ~0.6 us, pipeline bind ~0.7, per-draw setup ~0.5-1, pass begin ~5-6, pass end ~3-5; ~40 draws and ~20
`setPipeline` per frame on both backends (`-Pbench.s28Count`). GL debug output ruled out: vanilla's `glDebugVerbosity:1` forced to 0 gave
−11 us, +1.7% static (2 pairs, `s28-gl-debug`); it's vanilla's option, not worth a switch.

**`7193e1d` on OpenGL** (plates off, 65 groups, in-launch 8 rounds, each part off with the others on; temporary switches now in
`s28-gl-diagnostics.patch`; `s28-gl-7193-toggle`): all three parts 1228.4 -> 1448.1 (**+17.9%**, 8 of 8), render thread 0.795 -> 0.668 ms;
bindings kept +7.5%; one prepare per render type +4.7%; one instance ring +11.5% (prepare −35 us, `awaitGpu` −34). On Vulkan the ring is
within noise and all three ~+2%. On OpenGL, 65 per-group mapped rings cost CPU and GPU.

**Kept: "Skip Redundant Frame Clear"** (`skipFrameClear`, impact LOW; `render/FrameClear`, `GameRendererM`, `FrontendCommandEncoderM`,
`FrontendGpuSurfaceM`). `GameRenderer.render`'s start-of-frame color+depth clear of the main target is only remembered when the level is
drawn, and so is `render3dHud`'s depth clear of the main target (the hud target's depth, read by post effects, is left alone). A full clear
of the same textures drops the remembered clear (the level's clear pass, or the GUI's depth clear). Any other use performs it first (a render
pass with them attached, another clear, an upload or copy, the swapchain blit), as does the end of `GameRenderer.render`. Temporary
counters: 69632 dropped and 0 performed on OpenGL, 49152 / 0 on Vulkan; with the hand drawn (creative) the deferred depth clear was
performed 6145 times. The hand clear only saves when no hand is drawn (spectators, third person, hidden HUD, sleeping).
Exact: `StableDiff` threshold 1, frozen close-ups close0-3, 0 stable px on OpenGL and Vulkan for each clear (`s28-frameclear-close`,
`s28-handclear-close`), and in creative with the hand drawn with the whole switch toggled (`s28-frameclear-creative`).

| in-launch 8 rounds, static, off -> on | OpenGL | Vulkan |
|---|---|---|
| first clear (`9091a82`, `s28-frameclear-toggle`) | 1573.6 -> 1611.6 (+2.4%, 7 of 8), −19.7 us | 1844.7 -> 1859.3 (+0.8%, 7 of 8), `awaitGpu` −4.4 us |
| hand depth clear (`3bfb9a8`, `s28-handclear-toggle`) | 1599.9 -> 1651.0 (+3.2%, 7 of 8), hand 13.5 -> 0.2 us | 1854.3 -> 1872.4 (+1.0%, 8 of 8) |

Session start `778932f` vs end `0669364`, 3 interleaved pairs (`s28-final-gl`, `s28-final-vk`):

| | static | pan | initial | particles |
|---|---|---|---|---|
| OpenGL | 1603.1 -> 1643.6 (+2.5%, 3 of 3), cpu 0.61 -> 0.60 | 1605.2 -> 1682.0 (+4.8%, every pair), cpu 0.61 -> 0.57 | 1026.0 -> 1052.8 (2 of 3) | 1233.5 -> 1286.8 (+4.3%, every pair) |
| Vulkan | 1838.8 -> 1869.1 (+1.6%, every pair) | 1724.6 -> 1748.3 (+1.4%, every pair) | 1345.5 vs 1320.7 (noisy join) | 1489.0 vs 1491.3 |

Busy gap after: ~0.19 ms (OpenGL 0.606 vs Vulkan 0.416). Settings screen check passed (`s28-settings`).

**Binned:** retained multi-draw (`s28-retained-multidraw.patch`): a draw's plane and sign runs as one `glMultiDrawElementsBaseVertex`. 0 px
in 4 chest cube views, but OpenGL static off/on 1.0006 (4 of 8), −2.7 us: the 23 retained draws are mostly single runs.

**After the merge** (`943c1cf`, both agents' switches together): frozen close-ups close0-6 (4 shots, `closeToggle=skipFrameClear`, vignette skip, closed model culling and retained cell culling on) are 0 stable px at `StableDiff` threshold 1 on Vulkan and OpenGL (`s28-merge-close`, logs 2516, 2517). With the vignette skipped and nothing else in the HUD, the GUI-before-blur pass is gone, and the deferred `render3dHud` depth clear is dropped by `GameRenderer.render`'s GUI depth clear, which still runs. The settings screen check passed with all of the session's switches (log 2518).

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
blocks, plane culling and the mod compatibility trade-offs). On 2026-10-29 the user accepted the mod compatibility
trade-offs of the nineteenth (both parts), twenty-first, twenty-second and twenty-third sessions as well, and those of the twenty-fifth
session's part 2 (clouds and screenshots). The notes stay as reference; the
`TransparentFaces` cache miss with instanced parts off was fixed in the nineteenth session (section 7).
Twenty-first session, mod compatibility only (exact for vanilla, switch "Reuse Sort Order"): staged draws whose sorting is
vanilla's distance sort no longer call `MeshData.SortState.writeSortedIndexBuffer` (already an `@Overwrite` since the first
sessions), so a mod injecting there or into `VertexSorting.sort` is bypassed for them; a mod handing the group its own
`VertexSorting` keeps vanilla's call.
Twenty-second session, mod compatibility only (accepted by the user on 2026-10-29; exact for vanilla, switch "GPU Player Poses"): player
body submits of the exact `PlayerModel` class with vanilla's `entity_translucent` pipeline are no longer written into the group's
vertex builder, so mods that act on those vertices (a `VertexConsumer` wrapper, mixins into `Cube.compile`, `ModelPartCubeM` or
`BufferBuilder`) or on the draw (`PreparedRenderType.drawFromBuffer`, `RenderTypeFeatureRenderer.executeGroup`) don't see them;
shader packs that swap vanilla's pipelines would not swap the `player_pose` copy (as with the instanced and retained pipelines).
Posing mods (`setupAnim`, part fields, `translateAndRotate`), mods that draw inside `ModelPart.render` (detected: vanilla's path) and
mods with their own model class or render type are unaffected.
Twenty-third session, mod compatibility only (exact for vanilla, switch "Retained Labels"; accepted by the user on 2026-10-29): text submits of
kept text are no longer written into the group's vertex builder, so mods that act on those vertices (a `VertexConsumer` wrapper, mixins
into `BakedSheetGlyph.render` or `BufferBuilder`, or the upload's sort of those draws) or on the draw (`PreparedRenderType.drawFromBuffer`,
`executeGroup`) don't see them; shader packs that swap vanilla's text pipelines would not swap the `label_*` copies. Mods with their own
`TextRenderable`s, outlined or obfuscated text keep vanilla's path.
Twenty-fifth session, mod compatibility only (switch "Separate Armor Stand Plates"; the same rounding class as the instanced armor stands
it extends: across the switch only 1-level differences on the plates' side faces, and closer to vanilla's path than without it; 7 pixels in
the close-ups that follow when the groups were built are open, section 7): vanilla `ArmorStandModel` submits now set the base plate's
`yRot` to 0 and `visible` to false for the moment the body's snapshot is taken and its group found, and render the plate alone with zeroed
offset and rotation when its group's mesh is built (all restored at once). A mod that changes the plate's scale per stand takes vanilla's
path for those stands (checked); a mod that observes those fields during `InstancedModels`' group build, or changes the plate's offset,
rotation or cubes per stand in ways other than through the fields the instance reads (`x`, `y`, `z`, `xRot`, `yRot`, `zRot` and the root's
transform), is affected. The radix sort and the direct gather of retained builds touch no vanilla code.
Twenty-fifth session, part 2, mod compatibility only (exact for vanilla: 0 px across the switches on Vulkan and OpenGL). **Accepted by the user
on 2026-10-29:** switch "Cloud Face Texture": the clouds are drawn with the mod's copies of vanilla's cloud pipelines and a `ruthenium:`
vertex shader, so shader packs and cloud mods that swap vanilla's cloud pipelines or `core/clouds` shader won't swap the copies (vanilla's
texel buffer is still written, so mods that read it keep working). Switch "Fast Screenshots" (**accepted by the user on 2026-10-29**): full size
screenshots no longer run vanilla's copy lambda in `Screenshot.takeScreenshot` (a cancelling `@Inject` at its head), so a mod injecting into
that lambda is bypassed; downscaled shots keep it. The `InstancedModels` change touches no vanilla code.
Twenty-sixth session, mod compatibility only (both exact for vanilla). Switch "Skip Empty Crossbow Charge Time" (`emptyCrossbowCharge`,
**on by default, the user's decision of 2026-10-29; kept on 2026-10-30 although it measured no fps gain, only less allocation**): a mod that hooks `EnchantmentHelper.modifyCrossbowChargingTime`,
`runIterationOnItem` or `CrossbowItem.getChargeDuration` to change the charge time of empty stacks is bypassed during humanoid extraction.
Non-empty stacks keep vanilla's call. Switch "Render Type Lookup Cache" (`renderTypeCache`, **accepted by the user on 2026-10-30**): vanilla's
`RenderTypes` methods are untouched. A mod reading the memo fields through an accessor gets the wrapper, which gives the same answers. A mod
injecting into `Util`'s memoizing `BiFunction` misses the hits of these memos. The group eviction touches no vanilla code.
Twenty-seventh session, mod compatibility only (each exact for vanilla, each switch on by default, **all accepted by the user on 2026-10-30**):
- **"No Scratch Meshes for Cached Models"** (`noScratchMesh`): mods that hook Fabric's `setupMesh`, or read a state's own mesh, see none for
  models in `BlockModelMeshCache`. A model that keeps the random it is handed gets the shared one.
- **"Model Part Child Arrays"** (`partChildArrays`): `ModelPart.render` is now an `@Overwrite` (as `compile` and `translateAndRotate`
  already were); injections into it still apply. A mod that replaces a child in the map without changing its size isn't seen while the
  switch is on.
- **"Pooled Submit Poses"** (`pooledSubmitPoses`, three commits): a mod that keeps submits or their poses past the frame they were made in
  sees them change.
- **"Light Probe Without Garbage"** (`noAllocLightProbe`): a mixin into `Entity.getLightProbePosition` itself is bypassed. Renderer
  overrides of `getBlockLightLevel`/`getSkyLightLevel` get a mutable position.
- **"Keep Display Names"** (`keepDisplayNames`): a mod that mutates a custom name or team prefix component in place, or changes display
  names through mixins into those methods, keeps showing the old name tag.
The no-switch commits (`faa4045`, `d2c512e`, `c71567f`, `a121d7c`, `fbc5245`) are exact. They change only the mod's own code, or vanilla
calls whose results stay the same.
Twenty-eighth session, mod compatibility only (each exact for vanilla, each switch on by default, **not yet reviewed by the user**):
- **"Skip Redundant Frame Clear"** (`skipFrameClear`): vanilla's two clear calls in `GameRenderer.render` and `render3dHud` are
  `@WrapOperation`-wrapped and may be skipped. A mod that wraps or injects at those calls sees them skipped, or performed later at the
  target's first use. A mod that samples the main color or depth texture in a pass drawing into other textures, before the level's clear
  pass or between the hand's depth clear and the GUI's, sees the previous contents instead of cleared values. Drawing into, copying or
  clearing those textures is covered.
- **"Skip Black Vignette"** (`skipBlackVignette`): a resource pack or shader pack that changes `core/position_tex_color` or the vignette's
  blending, so that a black vignette changes pixels, loses that effect.
- **"Cull Closed Model Insides"** (`closedModelCulling`): only the mod's own instanced `CUTOUT` groups are affected. See-through resource
  pack texels fail the check and keep the old drawing. A mod that relies on seeing an armor stand's inside from outside would differ.
- **"Retained Cell Culling"** (`retainedCellCulling`): the mod's own path only; bounds come from the actual vertices.
The twentieth session changed no mod code (its downloaded skin culling was binned), so it adds nothing to decide. Had it
been kept, its trade-off would have been: a mod that writes a downloaded skin's GPU texture without `DynamicTexture.upload`
leaves stale masks (faces missing until the next upload or reload).
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
- Nineteenth session, part 1: mod compatibility notes at the end of its entry in section 7 (accepted by the user on 2026-10-29).
- Nineteenth session, part 2 (players and holograms, accepted by the user on 2026-10-29), mod compatibility only; each switch is exact
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
     done; their body is drawn from a GPU pose buffer since the twenty-second session (`PlayerPoseDraws`), still one draw per
     player in its sorted place. Other translucent types stay vanilla.
1b. **Players and holograms, next** (section 7 "Players and holograms", in order of expected gain):
   - ~~`TransparentFaces` with many skins~~: done in the twentieth session. Part 1's cache fixed the resource skin loss (now
     neutral to +1.5%); testing downloaded skins was built and binned (no gain: the exactness margin leaves almost no outer
     layer face skippable; `scripts/patches/s20-downloaded-skins.patch`). Nothing left here.
   - ~~Reuse last frame's sort order~~: done in the twenty-first session, as a start order rather than a bit-identical reuse
     (players' idle arms bob every frame, so centroids are never identical): sort and indices 1465 -> 314 ns per player body,
     lobby static 328.0 -> 352.9 fps. What is left of the sorted draws: decoding the centroids (~0.37 us per player, vanilla's
     `decodeSortingPoints` with `ByteBuffer.getFloat` and a new `CompactVectorArray`), the key computation in the sort and the
     index write. Skipping the sort for opaque skins is still not exact (coplanar touching faces, section 7).
   - ~~The player body from a GPU pose buffer~~: done in the twenty-second session ("Bigger jobs" job 1, section 7): players static
     523.6 -> 603.5 fps, lobby 351.6 -> 397.3. What is left per body: vanilla's `setupAnim` and part tree walk (the capture runs
     it into a sink), ~12 pose writes and 72 centroids (8 corner transforms per cube), the sort from its start order and the
     index write, plus a draw per player (vanilla's count). Ideas, not measured: skip the part tree for players whose part angles
     and visibility didn't change (a still player's arms bob every frame, so only for hidden arms or spyglass holders); cover
     `entity_translucent_cull` (invisible players seen by teammates) with a culling copy of the pipeline.
   - ~~Retained/instanced billboard text~~: done in the twenty-third session (`RetainedLabels`, "Bigger jobs" job 2, section 7): all
     phases, the see-through pass and the sorted `text_polygon_offset` draws included, in vanilla's draws and order; lobby static 398.8 -> 479.1 fps (render thread 2.43 -> 2.03 ms), holograms only 1226.9 -> 1721.0. What is
     left per label: vanilla's submission (`submitNameTag`/`submitText`: pose stack, a `Matrix4f` copy, content and submit records),
     `SignTextCache`'s variant lookup, the draw selection and ~64 B of pose; per frame the sorted draw's centroids and sort while
     anything moves. Standalone text display backgrounds (one quad each, `texts` phase, sorted) and outlined (glowing sign) text are
     still written by vanilla's path.
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
   - **Block entity scene slower than at the end of the seventeenth session: resolved, the machine** (twenty-second
     session, section 7, nineteenth session, "After the merge"): the seventeenth session's own build (`8dee171`) runs at the
     same ~1370 static / ~1850 pan as HEAD now (3 pairs); the GPU frame is +0.17 ms in every view since a restart on 28/10
     ~21:21, outside the probed passes. Not ours; compare GPU bound numbers only within a session.
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
   - **Twenty-fifth session (section 7): the mixed scene replaces the server scene here, and its static view is CPU bound** (yaw 0:
     render thread ~0.6 ms, GPU 0.358 ms; yaw 270: 0.644 vs 0.425), so the GPU and server scene numbers in this item are history. Its
     client tick is ~0.53 ms (not 0.93). Done: armor stand base plates as their own instances (static +12.5%), radix-sorted retained
     builds gathered into the upload; in part 2 clouds without a texel buffer (mixed static +3.4%, `queueSubmit` -21.7 us), instanced groups
     sharing ring, prepares and bindings (~0.8 us per group before; -4 us at 7 groups), row-wise screenshot readback (icon callback 24-33 ->
     11-15 ms).
     Done in the twenty-sixth session (section 7):
     - the stale-group eviction;
     - the explanation of the seven pixels;
     - candidate 6 (render type front cache, empty crossbow charge);
     - the OpenGL numbers for the plate change and the radix sort.

     **The mixed static view is now effectively GPU bound** (`awaitGpu` ~0.13 ms of a ~0.52 ms frame). CPU savings there show in the
     render thread time and allocation, not in fps.
     Done in the twenty-eighth session (section 7): the GPU time (vignette skip, closed model culling, retained cell culling: Vulkan static
     +23.4%) and the frame gap (GPU-side present work and the one-frame queue); the OpenGL gap (~85% vanilla's GL backend and the driver;
     "Skip Redundant Frame Clear"). Left:
     - **Block entity scene static -2.1%** from retained cell culling where every cell of a dense section is in view. Untested fix: one
       merged buffer per part, laid out group-major across cells, so fully visible cells merge into the original few runs.
     - ~~The bench's present path~~: changed on 2026-10-30 at the user's request. Bench tasks start with vsync off (section 6.4), and
       the VSync toggle's tooltip tells players to restart after turning it off on Vulkan. Making the fast path reachable without a
       restart (it isn't a new `VkSurfaceKHR`, section 7) is still open.
     - **Remaining GPU at the mixed static view (Vulkan, us):** opaque terrain 88, instanced models 38, retained 28, blit + acquire 35-50
       (vanilla's), sky 14, the GUI depth clear ~10.
     - **Closed box culling for `InstancedParts`** (mob `entity_cutout`, `armor_cutout_no_cull`): the entity and players scenes are CPU
       bound, so it wouldn't show yet.
     - **OpenGL leads:** a binding cache in vanilla's GL `setupDraw` (~10 us redundant UBO and sampler binds per frame; needs a switch,
       raw GL mods could break it); the mod's retained draws (+19 us on GL, 7 pipeline switches) and instanced draws (+6.5 us, a mesh per
       draw). Present (+81 us), `glBlitFramebuffer`, `glFenceSync` and the first framebuffer bind are out of reach.
     - **Candidate 7 (a trade-off idea only):** skip extraction and submission of armor stands whose inputs didn't change. It needs dirty
       tracking and is not exact for mods. What it could save is below.
     - **Plates off:** 37 grid keys per model against a cap of 32 still send ~20 stands to vanilla's path. Raising the cap was not
       measured. This matters only with the switch off.
     After the plate change the armor stands still cost ~63 us
     extraction, ~38 us submission and ~60 us model prepare per frame; the static render thread is now ~0.39 ms per frame.
   - **Retained builds after joining or teleporting**: a section is built in one go; in the mixed scene's initial phase
     three frames build 2-2.7k block entities each in 95/38/27 ms (cold). Spreading builds over frames with a time budget
     was built, verified exact and binned (nineteenth session, `scripts/patches/s19-spread-builds.patch`): the unbuilt
     chests' vanilla path made more slow frames than it saved. The phase's largest frames (~120 ms, frames 0-3 after the
     teleport) are not builds and were not investigated; that is the lead for join smoothness. **Investigated in the twenty-fifth
     session:** frame 0 is vanilla's new-world icon (`Screenshot.takeScreenshot`, 20-41 ms) plus the cold first extraction and model
     prepare of the armor stands; the next tick's packets (44-54 ms) and frame 1's light updates (13.6 ms) are vanilla's; frames 3-6 are
     retained builds, ~85% JIT-cold (warm, the same 8192 block entities take ~25 ms).
   - Occlusion culling of block entities is **not** exact (chests are 14/16 wide; the ones behind show through).
3. **Allocation.** Done in the twenty-seventh session (section 7). Static render-thread KB/frame is now entities ~1045, players ~747 and
   mixed ~63. Left:
   - **Per-frame entity render states:** 52% of entity scene and 36% of players allocation (`ItemStackRenderState`, `LayerRenderState` with
     a `Matrix4f`, `AvatarRenderState`, ...). Reusing render states generally is a trade-off; only armor stands do it today
     (`ReusedRenderStates`).
   - **`RenderType.prepare` per draw:** 7% of entities, 13% of players. Much of it is our own `PlayerPoseDraws.capture` and
     `RetainedLabels.submit`, one prepare per body or label. A per-frame cache would have to check the model view, scissor, texture views,
     samplers and texture transforms, so it was skipped as too invasive.
   - **Immutable submit records:** `ModelFeatureRenderer$Submit`, `TextFeatureRenderer$Submit`, `Content$Text`.
   - **Fabric API's `permitArmorWithCustomRenderers` `Object[]`:** 9.5% of mixed.
   - ~~The mixed `initial` phase reading ~2% lower~~: noise, settled in the twenty-eighth session (section 7).
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

### Bigger jobs: player bodies and billboard text (written up after the nineteenth session; job 1 done in the twenty-second, job 2 in the twenty-third)
Both are the large items of item 1b. The numbers come from the players scene (section 6.10, section 7 "Players and
holograms"): lobby static ~326 fps, render thread ~2.96 ms of a ~3.1 ms frame, CPU bound (GPU 0.82 ms). Neither has been
prototyped. The decompiled facts below come from that session's notes; check them against the 26.3 sources (section 5)
before designing on them.

**1. Player bodies from a GPU pose buffer.** Done in the twenty-second session (`e52b45d`; section 7, "Twenty-second session",
answers both "find out first" questions: uniform buffer slices per draw on both backends, texel buffers only whole and with a
`VkBufferView` per descriptor push on Vulkan; the upload's sort reads centroids from the written vertices, so the posed draw keeps an
empty staged draw in its place and draws itself with its own sorted indices). The sketch below held, with three corrections:
consecutive bodies of one skin share a draw and one sort in vanilla (handled: up to 8 members per draw, mesh copies per member);
normals had to be quantized as vanilla writes them (vanilla's player normals are trusted, not renormalized, so lighting uses the
shorter decoded vector); and vanilla writes joining a posed draw must write its members first. The original write-up follows.
- *What it would remove:* the body's vertex writes, 2.8 us per player per frame (72 faces through `ModelPartCubeM`'s
  direct writes), plus most of their upload bytes. That is ~0.42 ms for the lobby's 150 players. The per-frame quad sort
  (1.8 us per player) is a separate cost: item 1b's "reuse last frame's order" takes it for still players, and this job
  only makes it cheaper for moving ones.
- *Why the existing instancing doesn't cover it:* `PlayerModel` uses `entity_translucent`. That is a strictly ordered
  group in the translucent phase, sorted per draw on upload, so the bodies can't be merged into `InstancedParts`'
  solid-phase batches. Moving them to the solid phase is not exact (it changes their order against the other translucent
  models; the reason first written here, that text backgrounds draw before the translucent models, is wrong, see the
  twenty-third session: the nameTags and texts phases draw after them).
- *Design:*
  - Keep one draw per player, in exactly the place vanilla's draw has in the group's order. vanilla already makes one
    draw per skin anyway.
  - Feed it from a part-space mesh instead of written vertices. The mesh is the `PlayerModel`'s cubes, written once per
    model layout (wide/slim) and UV mapping, with a part index per vertex. The `PartBatch` meshes of `InstancedParts`
    are the starting point.
  - Each frame write only the poses: ~12 parts × (pose rows + normal rows) per player, plus light, overlay and color,
    into a pose buffer. The vertex shader looks the pose up by part index. Use `instanced_part.vsh`'s `precise`
    transform in JOML's operation order, so positions stay bit-identical; normals then differ only in quantisation, as
    with instanced parts (≤5 px per close-up).
  - Sorting stays exact if the index order is vanilla's. Vanilla's comparator runs on quad centroids decoded from the
    written vertices, so compute the same centroids on the CPU from the posed corners: 4 position transforms per quad
    instead of a full vertex write. Then write the index buffer in that order. For still players, item 1b's
    order reuse skips even that.
  - Respect the parts vanilla hides (`visible`, `skipDraw`), the faces `TransparentFaces` skips (they must be absent
    from the mesh or masked per skin), and the hat and sleeve layer toggles. A changed visibility set selects a
    different index range, not a new mesh.
- *To find out first:*
  - Does renderpearl give a vertex shader indexed access to a per-draw buffer: a uniform buffer slice per draw, or a
    storage or texel buffer? `InstancedParts` only used per-instance vertex attributes. If not, the fallback is one
    instanced sub-draw per part. That keeps the order but adds ~12 draws per player, which probably costs more than it
    saves.
  - Does the upload's sort read centroids from the vertex data, or can it be handed precomputed ones without copying
    vanilla's code path?
- *Exactness checks:* frozen close-ups of every skin, layer and pose with the switch flipped. A temporary check that
  compares the CPU-side centroids and index order with vanilla's for every player draw. Players behind hologram
  backgrounds, and players inside other translucent geometry (water, stained glass).
- *Compatibility:* skin-layer, emote and avatar mods (3D skin layers, Emotecraft, Figura and the like) replace or extend
  `PlayerModel` parts or `ModelPart` rendering. Engage only for vanilla `PlayerModel` classes whose part tree matches
  the cached layout, and fall back to the vanilla path otherwise. Fabric's render state data is untouched, because
  extraction and submission stay vanilla.

**2. Retained or instanced billboard text** (name tags, and `center`-billboard text displays). Done in the twenty-third session
(`RetainedLabels`; section 7, "Twenty-third session", which also answers the "find out first" questions and corrects the phase order
claim of job 1). Two departures from the sketch below: labels are not instances (each vertex names its label's region in a texel buffer
of poses, so the sorted `text_polygon_offset` draws keep vanilla's cross-label quad order and every vanilla draw stays one draw), and the
see-through pass is covered too (its draws are kept as vanilla makes them; only their vertices are retained). The original write-up
follows.
- *What it would remove:* the per-frame glyph and background writes of each label. Measured: ~0.9 us per player name
  tag, ~0.67 us for an armor stand line's two name tag passes, and most of a text display line's ~1.0 us. In the lobby
  that is an upper bound of ~0.3–0.4 ms.
- *What makes it possible:* a label's quads are the same in label space every frame. Only the billboard matrix (camera
  rotation, scale, translation) changes. The text is already kept across frames (`nameTagTextCache`,
  `textDisplayTextCache`, `SignTextCache` lines with prepared glyphs), so a label-space mesh can be cached on the same key
  and dropped with it.
- *Design:*
  - Write each kept label's glyph and background quads once, in label space, into a shared text mesh buffer (one region
    per label, freed with the cache).
  - Each frame, per label: one matrix (3 pose rows), light and colors, and a reference to its region.
  - Draw them with one indirect multi-draw per (render type, font page), with a per-draw base instance into the matrix
    buffer. Without multi-draw indirect, per-label draws cost more than the writes they replace: the reason it was not
    tried in the nineteenth session.
  - Positions again use the `precise` JOML-order transform.
- *Hard constraint, ordering:* the see-through name tag pass is sorted and depends on vanilla's order, so only the
  normal pass and text displays are candidates. First find out which phase and group each text render type
  (`text`, `text_see_through`, `text_background`, `text_intensity`, the polygon offset variants) goes to, and whether
  vanilla may reorder it. If a type is strictly ordered, merging labels into one draw reorders overlapping labels. That
  is exact only where the type doesn't blend or the labels can't overlap on screen, which is easy to break with
  holograms stacked in a column.
- *To find out first:* indirect multi-draw and base-instance support in renderpearl, on both the Vulkan and OpenGL
  backends; the render type and phase facts above; obfuscated text, which is laid out every frame and must stay on the
  vanilla path.
- *Exactness checks:* frozen close-ups of stacked holograms, of labels overlapping players and each other, and of
  coloured, bold and shadowed text, with the switch flipped.
- *Compatibility:* mods that draw their own name tags, or change them in `submitNameTag` or the text renderer, would
  be bypassed. Engage only for vanilla's submit paths, and keep a switch.

**Order:** do item 1b's sort-order reuse first. It gives part of job 1's gain exactly, and it shows how much the sort
still costs for moving players. Of the two jobs, start with job 1 if the player count matters most (servers, lobbies),
and with job 2 for hologram-heavy hubs. Answer both "find out first" questions before writing code.

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
