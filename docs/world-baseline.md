# Ruthenium world baseline

What Ruthenium's world renderer costs on the routes the vanilla client was measured on, and where its frame time, memory and latency go. It measures an unfinished renderer: read [what the scene leaves out](#what-ruthenium-draws-and-what-it-does-not) before any ratio. The vanilla numbers are the dedicated-server runs of [baseline.md](baseline.md); the menus' own cost is in [menu-baseline.md](menu-baseline.md).

The release measurements here were taken on 2026-11-24. The first, from the working tree on top of `d447d22`, is of a renderer that meshed every loaded section and drew every one inside the view: everything from [Against vanilla](#against-vanilla) on, where nothing else is said. The second, on top of `50564e0`, is of [cave culling](#cave-culling), which is the default since. A third, on top of `92e7bb6`, is of the terrain drawn as vanilla's shader draws it, with fog and sky: [the picture against vanilla's](#the-picture-against-vanillas), which also holds the two clients' pictures against each other for the first time. A fourth, on top of `63b6734`, adds [water, lava and translucent blocks](#water-lava-and-translucent-blocks). On 2026-11-25 short runs on the same working tree followed for [animated textures and the camera inside fluids](#animated-textures-and-the-camera-inside-fluids).

## Machine and conditions

The laptop of the vanilla baseline, in Discrete GPU mode: NVIDIA GeForce RTX 4060 Laptop GPU, driver 620.70.06, KDE Plasma on Wayland, 2560 × 1600 at 240 Hz with a display scale of 2, variable refresh off, firmware profile `performance`, on AC. The window was 960 × 540 points, which is 1920 × 1080 pixels; the task checks the drawable. VSync off (`IMMEDIATE`), view and simulation distance 12, four meshing threads.

**The desktop is streamed on this machine**, as in the vanilla baseline. The task now records the GPU's video encoding sessions before and after every run. In the first set of runs below a viewer was connected for about half of them, and those runs have 1% lows 12 to 16% under the others of the same variant (idle 1082 against 1289 FPS, fly 1351 and 1367 against 1580). The second set ran with no session throughout. Whether a viewer was connected during the vanilla runs was not recorded.

The Intel iGPU needs the laptop's Hybrid mode. Nothing here ran on it, on AMD or on a Mac.

## Method

`cargo xtask bench dedicated --client ruthenium` runs Ruthenium in its window as it runs the vanilla client: a fresh copy of the world template, a vanilla 26.3 dedicated server on loopback set up over RCON, and one route file that says where the player starts and what is held on every client tick. See [the method in baseline.md](baseline.md#dedicated-server-runs-method). What is particular to Ruthenium:

- **The route is played in the window.** `ruthenium --join <address> --route <file>` steps the route before every frame it hands to the game thread. It never waits for the game thread; a run in which a stall let a tick go by without its input fails instead of recording something else. The keyboard and the mouse reach nothing in the world meanwhile, the mouse is not grabbed and losing the focus pauses nothing.
- **The same run or no result.** A run fails unless the position after every tick is the route's. With `--baseline`, a scene's record must also be the one the vanilla runs recorded: the same position on each of the 1440 ticks and the same chunks at the start and at rest. All 54 runs below were.
- **The scene is ready** when the player stands at the start with the start's chunks loaded for a second, and every loaded section has had its mesh on the GPU at one moment since. The vanilla bench client waits for its sections to have been built in the same way. Blocks keep changing somewhere in the loaded world for minutes after a join (random ticks are off; lava flowing underground is the likely cause, which was not confirmed), so there is no moment after which nothing is being built: it is part of every scene, for both clients.
- **Only the capture is measured.** 10 s of warm-up, then 60 s (1200 ticks); frame intervals are from present to present, as the vanilla mod takes them. A run that left something out fails: a mesh that found no room, sections left out of a frame, or frames held back behind a hidden window.
- **Variants.** `--variant <label>=<options>` runs the client with other options. The variants of a repetition run one after the other, scene by scene, so that they meet the same machine.

Medians of 3 runs throughout.

## Cave culling

Only what can be seen from the camera is meshed and drawn, as vanilla's cave culling decides it. Every mesh says which of its section's six faces can be seen from which, through the blocks in it that are not solid. A walk from the camera's section through those faces, breadth first, finds the sections that are reached: it never turns back in a direction opposite to one it has travelled, a section with blocks lets nothing through until it has a mesh, a section of air needs none, and more than three sections from the camera a step is only taken if the sections on the straight line back to the camera were reached before it. The walk runs on a thread of its own and starts again when the camera enters another 8-block cell. `--terrain-culling frustum` turns it off: every loaded section is then meshed, and drawn if it is inside the view, which is the renderer the sections after this one measured.

Recorded on 2026-11-24 from the working tree on top of `50564e0`, on the same machine with the same options, in three variants: the default, the frustum alone, and the default with one frame of GPU work in hand. Of 54 runs, 18 ran while a viewer was connected to the streamed desktop and are left out ([below](#a-viewer-on-the-streamed-desktop)); the other 36, three for every scene and variant, had no video encoding session before or after them. Every run recorded the baseline's position on each of its 1440 ticks. Medians of 3, as average FPS / 1% low / 0.1% low:

| Scene | Vanilla | Cave culling (default) | × vanilla | Frustum only | × vanilla | Culling, one frame ahead | × vanilla |
|---|---:|---:|---:|---:|---:|---:|---:|
| idle | 1456 / 896 / 638 | 3149 / 1694 / 1374 | 2.16 / 1.89 / 2.15 | 1840 / 1243 / 1013 | 1.26 / 1.39 / 1.59 | 3709 / 1763 / 1418 | 2.55 / 1.97 / 2.22 |
| spin | 1532 / 903 / 633 | 3328 / 1747 / 1415 | 2.17 / 1.93 / 2.24 | 2128 / 1354 / 1085 | 1.39 / 1.50 / 1.71 | 3948 / 1852 / 1528 | 2.58 / 2.05 / 2.41 |
| walk | 1410 / 807 / 512 | 3167 / 1655 / 1341 | 2.25 / 2.05 / 2.62 | 1845 / 1212 / 965 | 1.31 / 1.50 / 1.88 | 3769 / 1572 / 1043 | 2.67 / 1.95 / 2.04 |
| fly | 1569 / 802 / 404 | 4287 / 1784 / 1303 | 2.73 / 2.22 / 3.23 | 3332 / 1572 / 1114 | 2.12 / 1.96 / 2.76 | 5146 / 1739 / 1262 | 3.28 / 2.17 / 3.12 |

The frustum-only numbers are those of the first measurement again (idle 1849 / 1247 / 1022 and fly 3358 / 1567 / 1147 there, with no viewer).

**Against the targets** of 1.5 times vanilla's average and twice its lows, at the default options: the average is met in every scene (2.16 to 2.73), the 0.1% low in every scene (2.15 to 3.23), and the 1% low in walk (2.05) and fly (2.22), with idle at 1.89 and spin at 1.93. One frame ahead adds 18 to 20% to the averages and moves the 1% lows by less than the runs differ among themselves. The scene is as incomplete as before: [what it leaves out](#what-ruthenium-draws-and-what-it-does-not) has yet to be drawn, and no ratio here is a result until it is.

**Run to run.** At the default, two of a scene's three runs agree within 1% on average FPS and on the 1% low. In idle the third does too; in spin, walk and fly it was 1 to 6% lower on the average and 8 to 31% on the 1% low (fly: 1230 against 1784 and 1801), with no encoding session recorded. For walk and fly that run is the single one of the last campaign.

### The same pixels

The frame after each route's end is the same file byte for byte with and without cave culling, in all four scenes and all 54 runs. So is the picture at eight places looked at by hand against a vanilla 26.3 server on the benchmark world, each with culling and without: in a room dug at y -30 looking down a lit tunnel 46 blocks long, at the tunnel's corner into a side branch, under a shaft dug to the surface looking up, on the surface beside the shaft's mouth, as a spectator in a natural cave, as a spectator inside solid stone (where culling is off, as in vanilla, and the culled client draws the very sections the other does), in a room dug in the Nether's warped forest, and after leaving and joining again in one process. A first picture in the Nether differed: it was taken while 48 sections of the culled client were still on their way to a mesh.

Underground the difference is largest: joined in the tunnel, the client holds 754 meshes in 32 MiB of buffer pages where it held 4,024 in 124 MiB, and draws 139 sections where it drew 994.

### What a frame draws and costs

At the capture's end, with the frustum alone in brackets. Times are medians in milliseconds; work is the main thread's own.

| Scene | Sections drawn | Quads in view | Draws | Work | Of it terrain draws | GPU |
|---|---:|---:|---:|---:|---:|---:|
| idle | 405 (925) | 291,723 (522,119) | 722 (1,425) | 0.052 (0.141) | 0.039 (0.102) | 0.206 (0.310) |
| spin | 405 (927) | 292,361 (523,789) | 722 (1,428) | 0.052 (0.130) | 0.039 (0.101) | 0.194 (0.262) |
| walk | 404 (893) | 364,355 (541,856) | 706 (1,396) | 0.054 (0.153) | 0.040 (0.115) | 0.200 (0.304) |
| fly | 361 (473) | 316,292 (349,543) | 638 (777) | 0.045 (0.080) | 0.031 (0.065) | 0.120 (0.144) |

- **The main thread's work falls to about a third** in idle, spin and walk: a frame tests the 1,400 to 2,400 meshes of reached sections against the view instead of 4,000 to 4,500, and records half the draws. Its P99 is 0.14 to 0.16 ms (0.24 to 0.31).
- **The GPU's time falls by a quarter to a third** in idle, spin and walk and by a sixth in flight, where less is hidden. These scenes are still bound by it: with the GPU waited for, a frame is the main thread's work and then the GPU's.
- **The walk** reaches about 8,000 of the 15,288 loaded sections at the spawn and 10,700 at the fly route's end; most are air. It is not on a frame's path. Over the 60 s of walk its thread answered 400 times, 72 of them with a walk from the camera, in 0.16 s altogether; in flight 2,966 times, 163 of them from the camera, in 0.47 s. The longest walk from the camera took 4.4 ms.

### Meshing, uploads and latency

Over the 60 s capture, with the frustum alone in brackets:

| Scene | Meshes installed | Of them first meshes | MiB uploaded | Meshing, ms a section | First mesh, P50 / P99 | A change from the server, P50 / P99 |
|---|---:|---:|---:|---:|---:|---:|
| idle | 0 (1,337) | 0 (0) | 0 (29) | | | none (1.7 / 3.2 ms) |
| walk | 1,714 (18,548) | 849 (5,160) | 85 (258) | 0.23 (0.13) | 1.8 / 5.8 ms, 4 / 15 frames (12.7 / 41 ms, 20 / 62 frames) | 2.0 / 6.2 ms, 5 / 17 frames (9.9 / 45 ms, 16 / 73 frames) |
| fly | 9,401 (56,125) | 8,571 (48,944) | 408 (711) | 0.22 (0.10) | 1.5 / 5.1 ms, 4 / 12 frames (7.1 / 22 ms, 19 / 46 frames) | 1.2 / 2.8 ms, 4 / 9 frames (1.7 / 6.0 ms, 4 / 13 frames) |

- **The idle scene is idle now.** The 22 sections a second that were meshed again there are all sections nothing can be seen of. A change in such a section waits, and is meshed when the section is reached.
- **A tenth to a sixth of the meshing, and a third to three fifths of the uploads.** The sections left out are the cheap ones, which is why a section now takes 0.22 ms to mesh on average and not 0.10.
- **A section is on screen sooner**, because the workers are no longer busy with what is hidden. With culling a first mesh, and a change in a section that was not reached, count from the moment the section is reached, so the two columns do not wait for quite the same thing.
- **Staging** on the render thread: P99 0.013 ms in flight (0.053), P99.9 0.15 ms (0.18), the worst frame of any run 0.85 ms (0.98).

### Memory

| Scene | Sections with a mesh | Buffer pages, MiB | Of it meshes | GPU memory of the process, MiB | Process RSS at most, MiB |
|---|---:|---:|---:|---:|---:|
| idle, spin | 1,409 (4,037) | 64 (128) | 61 (124) | 140 (204) | 369 (379) |
| walk | 2,001 (4,499) | 92 (152) | 86 (141) | 168 (228) | 372 (382) |
| fly, at its end | 2,976 (4,515) | 156 (196) | 151 (183) | 236 at most (272) | 374 (379) |

- **Inside the 108 MiB planned for the terrain at the spawn and on the walk, not over the fly route's densest stretch**, where the meshes take 151 MiB: seen from the air, a jungle is that much at 64 bytes a quad. No mesh was refused in any run.
- **Meshes of sections that are reached no longer are kept**, as vanilla keeps them, and not drawn: 523 of the 2,001 at the walk's end and 566 of the 2,976 at the fly route's.
- **Fragmentation is as before**: 24 to 102 allocations found no room in a walk run and 78 to 110 in a fly run, each with enough free bytes in total; in flight the pool added 19 or 20 pages and released 4.

### A viewer on the streamed desktop

In 17 of the 18 runs left out, a video encoding session was open before and after the run, and about 40 times a second (33 to 49) a frame waited 2.9 ms for the GPU, which took 0.2 ms for its own work: an eighth of the time. (In the eighteenth the session ended during the run, which had no such waits.) Average FPS was 11 to 15% lower and the 1% low was 330 to 357 FPS in idle, spin and walk whatever the variant. That is what the two unexplained runs of the first measurement showed. A session alone does not tell: the first measurement's runs with one had a wait of that length once in ten seconds. A run's own frame intervals do, and a result should only be taken from runs without such waits.

## Against vanilla

The first measurement, without cave culling. The first set: every scene, with the options as they are by default, with the sections drawn by multi-draw indirect, and with the GPU allowed one frame of work in hand when the next frame samples its input.

| Scene | Vanilla avg / 1% low / 0.1% low | Default | × vanilla | Indirect draws | × vanilla | One frame ahead | × vanilla |
|---|---:|---:|---:|---:|---:|---:|---:|
| idle | 1456 / 896 / 638 | 1828 / 1142 / 944 | 1.26 / 1.27 / 1.48 | 1907 / 1275 / 1038 | 1.31 / 1.42 / 1.63 | 2514 / 1370 / 1180 | 1.73 / 1.53 / 1.85 |
| spin | 1532 / 903 / 633 | 2128 / 1358 / 1120 | 1.39 / 1.50 / 1.77 | 2133 / 1192 / 889 | 1.39 / 1.32 / 1.40 | 2864 / 1252 / 880 | 1.87 / 1.39 / 1.39 |
| walk | 1410 / 807 / 512 | 1794 / 1087 / 847 | 1.27 / 1.35 / 1.65 | 1845 / 1094 / 847 | 1.31 / 1.36 / 1.65 | 2537 / 1183 / 876 | 1.80 / 1.47 / 1.71 |
| fly | 1569 / 802 / 404 | 3209 / 1367 / 984 | 2.04 / 1.70 / 2.43 | 3386 / 1525 / 1085 | 2.16 / 1.90 / 2.68 | 4555 / 1675 / 1166 | 2.90 / 2.09 / 2.88 |

The second set, with no viewer connected: the default again, the sections drawn a buffer page at a time, and every upload on the graphics queue.

| Scene | Default | × vanilla | Paged draws | Graphics queue only |
|---|---:|---:|---:|---:|
| idle | 1849 / 1247 / 1022 | 1.27 / 1.39 / 1.60 | 1866 / 1267 / 1028 | 1853 / 1255 / 1025 |
| fly | 3358 / 1567 / 1147 | 2.14 / 1.95 / 2.84 | 3394 / 1594 / 1141 | 3365 / 1573 / 1121 |

**Run to run.** In the second set the three repetitions of a variant agree within 0.8% on average FPS and within 3.5% on the lows. In the first set, where a viewer came and went, average FPS spreads up to 7%, the 1% lows up to 19% and the 0.1% lows up to a third.

**Against the targets** of 1.5 times vanilla's average and twice its lows, with the scene as incomplete as it is:

| | Default | One frame ahead |
|---|---|---|
| Average, 1.5 × | fly only (2.04 to 2.14); idle, spin and walk reach 1.26 to 1.39 | every scene (1.73 to 2.90) |
| 1% low, 2 × | none (fly 1.70 to 1.95, the others 1.27 to 1.50) | fly only (2.09); the others 1.39 to 1.53 |
| 0.1% low, 2 × | fly only (2.43 to 2.84) | fly only (2.88) |

Idle, spin and walk are bound by the GPU: it takes 0.26 to 0.31 ms for the half million quads in view, of which nothing is culled for being hidden behind other terrain. That, and not submission, is what stands between these scenes and the targets.

## What Ruthenium draws, and what it does not

A ratio above compares two different pictures.

- **Left out in the measurements above:** water and lava, translucent blocks, block entities, entities and the player's hand, the sky, clouds, fog and the fade-in of new chunks, smooth lighting and ambient occlusion, biome colours, animated textures, the time of day, particles and every part of the HUD but the crosshair. Vanilla draws all of them in its runs.
- **Drawn since these runs:** smooth lighting with ambient occlusion, and the biomes' colours on grass, leaves and the like, both worked out when a section is meshed, as vanilla does. The vertices stay as they were (a colour and two lights each), so a frame draws the same quads at the same cost: one run each with and without smooth lighting, with a viewer on the streamed desktop, gave 2697 and 2683 FPS on average in idle and 3687 and 3695 in fly, the GPU's 0.206 and 0.120 ms in both. Meshing a section took 0.31 to 0.38 ms on a worker at the spawn in three joins, for 0.24 ms in one join lit flat and untinted. The medians above were not measured again.
- **Drawn since, at a cost:** the terrain as vanilla's shader draws it (its texture filtering, both fogs, the fade-in of new sections), the sky, and the time of day in the light, the fog and the sky. [The picture against vanilla's](#the-picture-against-vanillas) has what that looks like and what it costs.
- **Also drawn since those runs.** Water and lava, including fluid in waterlogged blocks, and translucent block faces such as stained glass and ice.
- **Block entities since 2026-11-25:** [chests, shulker boxes, skulls and banners](#block-entities).
- **Also drawn since then.** Players and mannequins, the first-person hand, the targeted block outline and breaking stages.
- **The HUD since 2026-11-25:** [the crosshair, the hotbar and its items, hearts, armour, food, air, experience, effect icons and the vignette](#the-hud), and [its text](#the-huds-text): the selected item's name, chat, titles, the action bar, the sidebar, boss bars and the player list.
- **Still left out:** the other block entities, most other entities, clouds, particles, the weather's rain and snow, the overlays of a burning player and of a player inside a block, the effects on the player (blindness, darkness, night vision, a conduit's), typing in the chat, subtitles and F3.
- **Drawn in excess, in the first measurement:** every loaded section that had anything to draw got a mesh, and every one of those inside the view's frustum was drawn, hidden or not: at the spawn 925 sections and 522,119 quads in view, of 4,037 with a mesh among 15,288 loaded. Vanilla builds and draws only what its occlusion graph reaches, and with [cave culling](#cave-culling) so does Ruthenium, by the same rules: 405 sections and 291,723 quads, of 1,409 with a mesh. One difference remains: Ruthenium meshes what the walk reaches behind the camera too, where vanilla builds a section when it is first inside the view.
- **The same pixels whichever way the sections are drawn.** The frame after the route's end, captured with `--screenshot`, is the same file byte for byte with direct, paged and indirect draws, at the spawn and at the end of the fly route.

## The picture against vanilla's

Since the measurements above, the terrain is drawn as vanilla's terrain shader draws it, and the air is vanilla's:

- **Texture filtering.** Texels keep hard edges with a blend one screen pixel wide between two of them, and with the Fancy preset's "RGSS" filtering, the default here too, a texel smaller than a pixel is the mean of four samples at each of two mip levels. `--terrain-shader nearest` is the Fast preset's.
- **Fog.** The stronger of the fog of the place (by the distance in a straight line) and the fog at the edge of the view (sideways or in height, whichever is more, over the last tenth of the render distance), both worked out for every corner and carried across the faces, in the colour vanilla gives the fog: the dimension's, the sky's mixed in, a sunset's glow where the camera looks at it, darker towards the void.
- **Fade.** A section comes out of the fog's colour over 0.75 s from its first mesh, beyond 16 blocks from the camera (`--chunk-fade`).
- **Sky.** The dome in the sky's colour fading to the fog's, the glow of sunrise and sunset, the sun, the moon in its phase, the stars, the dark half below the horizon; the End's sky; none in the Nether.
- **The time of day, the weather and the biomes** move the sky's and the fog's colours, the sun and the light map through the dimension's environment attributes and timelines, read once a tick around the camera and blended between ticks. Block light flickers.

Not there yet: clouds, rain and snow themselves, what a camera sees from inside water, lava or powder snow, the effects on the player (blindness, darkness, night vision) and a boss's fog.

### The two pictures

`cargo xtask bench dedicated --client vanilla,ruthenium --scene idle --bare-picture` saves one frame of each client at the idle scene's place. This comparison hides the HUD, hand, block outline, vignette and clouds and uses `--light-flicker off` on Ruthenium. `--time` sets the time of day. Of the 2,073,600 pixels of a frame:

| Time of day | The same byte for byte | Within one step of 255 | More than 8 steps apart | The upper 400 rows (sky and treetops) the same |
|---|---:|---:|---:|---:|
| noon (6000) | 95.1% | 99.4% | 0.55% | 99.8% |
| sunset (12600) | 98.6% | 99.4% | 0.54% | 100.0% |
| midnight (18000) | 98.9% | 99.5% | 0.50% | 100.0% |

In those shader-only captures, the differences over 8 steps included the then-missing water at the frame's right edge (8,345 of the 11,466 such pixels at noon), a handful of animals, and its own crosshair. The pixels one step apart are on leaves and plants, in both directions; the cause was not looked for. With vanilla's HUD on, its vignette alone darkens the frame by up to 4% towards the edges.

So the sky's and the fog's colours, the sun's glow, the stars, the light by day and by night, the smooth lighting and the biomes' colours are vanilla's at this place at these three times, with the time standing still. Not compared: a picture in motion or with time passing, rain, another biome's fog, the Nether and the End (looked at, not held against vanilla), and distances at which most texels are smaller than a pixel.

### The same picture, however it is drawn

Vanilla draws the sky first and the world over it. Ruthenium draws it after the world's opaque layers, only where they left nothing (`--sky late`, the default; `first` is vanilla's order). Vanilla's shader takes nine samples of the atlas for every pixel with RGSS. Ruthenium's leaves out the ones whose weight is exactly nothing, which is eight of them wherever texels are larger than pixels (`--terrain-shader rgss`, the default; `rgss-reference` takes them all). The frame after each route's end is the same file byte for byte with `late` and `first` and with `rgss` and `rgss-reference`, at the spawn and at the fly route's end, in both repetitions, and again in the three repetitions of every route below.

`--exercise-terrain` now draws its scene with each of the four shaders in turn and in five fogs, and compares every pixel with the scene as a small rasteriser in the client draws it by the shaders' arithmetic: no pixel wrong in 1,587 frames on the RTX 4060 and 1,481 on llvmpipe. 27 deliberate breakages of the shaders and of what is written for them each fail it or a test, and so do 59 of the rules of the fog, the fade and the sky in the client (two of them only after tests were added for them). Its atlas is 4096 by 8 texels, where vanilla's rule counts every texel as smaller than a pixel: of RGSS it shows the averaging, and the screenshots above the rest.

### What it costs

On the RTX 4060 as before, 24 runs, medians of 3, each with the vanilla runs' position on every tick; average FPS / 1% low / 0.1% low, then against vanilla's. "Before" is the same build told to draw as it did (`--terrain-shader plain --sky off`).

| Scene | Before | With vanilla's shader and sky | × vanilla |
|---|---|---|---|
| idle | 3098 / 1384 / 1018 | 2417 / 1286 / 939 | 1.66 / 1.44 / 1.47 |
| spin | 3272 / 1406 / 1041 | 2443 / 1302 / 958 | 1.59 / 1.44 / 1.51 |
| walk | 3096 / 1376 / 1011 | 2465 / 1265 / 940 | 1.75 / 1.57 / 1.83 |
| fly | 4181 / 1490 / 1119 | 3355 / 1361 / 1017 | 2.14 / 1.70 / 2.52 |

A fifth to a quarter of the frames went to it, and the 1% lows' target of twice vanilla's is missed on every route in these runs. **A video encoding session was open during 17 of the 24 runs**, without the 2.9 ms waits a viewer gave [before](#a-viewer-on-the-streamed-desktop) (at most 0.65 intervals over 2 ms a second in any run), but with lower lows: the first repetition of idle, spin and walk ran without one, at 2518 / 1558 / 1243, 2558 / 1550 / 1254 and 2573 / 1460 / 1186, which is 1.73 to 1.82 times vanilla's average and 1.71 to 1.81 times its 1% low. The "before" column is under the same sessions, and below the [cave culling](#cave-culling) runs' medians, which had none.

Where the GPU's time goes, from one run of 20 s for each way of drawing (the frame's GPU time at the median, in milliseconds; fly is the route's first 20 s):

| Drawn with | idle | fly | What it adds |
|---|---:|---:|---|
| `--terrain-shader plain` | 0.202 | 0.125 | the shader as it was, and the sky |
| `--terrain-shader nearest` | 0.212 | 0.132 | fog and fade: 0.010 and 0.007 |
| `--terrain-shader rgss` (the default) | 0.257 | 0.162 | RGSS: 0.045 and 0.030 |
| `--terrain-shader rgss-reference` | 0.363 | 0.200 | all nine samples for every pixel: 0.106 and 0.038 more |
| `--sky first` | 0.254 | 0.161 | the order makes no difference here |
| `--sky off` | 0.249 | 0.154 | the sky costs 0.008 |

- **RGSS is most of it**, and leaving out the samples that count for nothing saves 29% of the frame's GPU time in idle against taking them all.
- **Where a fragment's constants come from mattered more than what is done with them.** Read from the frame's data for every pixel, the fog's colour and ranges and the atlas's size made fog and fade cost 0.051 ms instead of 0.010 in idle, and the default 0.298 ms instead of 0.270 in runs of the same day under one condition (a viewer on the stream). They are push constants now, and the terrain's and the sky's fragment stages read nothing of the frame's data.
- **The sky's order** was expected to matter on a GPU bound by its pixels. On this one the sky is 3% of a frame either way; the Intel iGPU is where it has to be measured.
- The main thread's work is 0.049 to 0.075 ms a frame, of which working out the fog, the sky and the light map is part of the terrain's stage, 0.001 ms at the median as before. The rise in idle from 0.054 is in recording the frame (0.046 to 0.064 ms), with the same draws; it comes with the longer wait for the GPU and was not looked into.

## Water, lava and translucent blocks

The mesher now emits sloped fluid surfaces and the translucent faces of block models. With the game's textures, lava draws with solid terrain. Water, stained glass and ice draw after the opaque terrain and sky. Sections draw farthest first, and workers update each section's quad order as the camera changes position. When the pictures and times below were taken, the camera used atmospheric fog beneath a fluid surface and fluid textures showed one animation frame. Both have [come since](#animated-textures-and-the-camera-inside-fluids).

### Pictures at rest

Six pairs from the 26.3 dedicated server at 1920 × 1080. Each percentage counts whole pixels by the largest absolute difference across their three RGB channels. Both clients recorded the same position and chunk checksums in each pair. Vanilla animates its atlas during the capture.

| Scene | Identical | Within 1 of 255 | More than 8 apart |
|---|---:|---:|---:|
| Spawn at noon | 95.32% | 99.78% | 0.16% |
| Ocean from above | 52.81% | 73.30% | 2.31% |
| Stained glass, ice and flowing water | 74.94% | 89.89% | 2.28% |
| Pools beside open and glass-covered pits | 96.11% | 98.82% | 0.49% |
| Pools over different ground | 93.68% | 96.94% | 1.57% |
| Lava fall, water, glass and ice | 77.47% | 79.93% | 12.71% |

In the last pair, the overlapping red and blue glass agrees within one step in all 34,000 pixels of the rectangle from (808, 300) to (978, 500), upper bounds excluded. The ice does too across (1090, 450) to (1210, 565). The flowing lava and the surface of the pool differ visibly. The four Ruthenium PNGs of this scene are byte-identical with direct, paged and indirect draws and with frustum-only culling. These are stationary views. They leave the visible delay of a resort during camera motion unmeasured.

### Cost of drawing the translucent layer

One on/off pair per route on the RTX 4060, 5 s warm-up and 20 s capture, direct draws, view distance 12. Each pair matched its position and chunk checksums. A 60 FPS video encoding session was active before and after all six runs. Both settings mesh the fluids and build the initial indices. `--translucent off` skips the translucent draws and subsequent resorts. Lava still draws in the solid layer.

| Route | Off, FPS avg / 1% / 0.1% low | On, FPS avg / 1% / 0.1% low | GPU median ms, off → on | Translucent quads / sections at capture end | Resorts installed during capture |
|---|---:|---:|---:|---:|---:|
| Idle at spawn | 2147 / 344 / 319 | 1917 / 339 / 318 | 0.257 → 0.275 | 54,071 / 208 | 0 |
| Fly | 3067 / 362 / 328 | 2434 / 350 / 322 | 0.162 → 0.214 | 79,653 / 211 | 5,816 |
| Fly over the ocean | 1426 / 317 / 298 | 1288 / 311 / 293 | 0.394 → 0.452 | 57,256 / 246 | 3,534 |

The median GPU cost rose by 0.018 to 0.058 ms in these pairs. The main thread's median work rose by 0.015 to 0.021 ms. Sampling each variant once with the desktop streamed limits what these lows say about normal frame pacing. The separate ocean flight with synchronization validation installed 5,015 resorts by exit and logged no validation errors. Raw run records and the screenshot pairs are kept locally under `cache/bench/results/fluids-2026-11-24-rtx4060/`.

### Mutation checks

All 32 selected mutations were detected. The render tests caught 12 changes to fluid geometry and shading, nine to sorting and index encoding, and seven to the software rasteriser. The RTX 4060 fixture caught all four GPU changes with both direct and indirect draws. The unmodified fixture had zero mismatches across 59 probe regions in 684 direct frames and 675 indirect frames.

One blend error initially passed. Using source alpha for the destination weight gives nearly the right byte value at half alpha, and over clear the destination contributes zero. A new probe draws a 26/255-alpha texel over solid terrain, where the two weights give visibly different results. Two other probes exercise a later draw behind a pane and behind a discarded texel. The sort tests also check that quad 16,384 still uses 16-bit indices and quad 16,385 switches to 32 bits.

### Reproducing the checks

```sh
cargo test -p ruthenium-render --lib
python3 tools/test-render-mutations.py
python3 tools/test-render-mutations.py --gpu     # display and multi-draw indirect support
cargo xtask bench dedicated --accept-eula --client ruthenium --scene idle,fly \
    --width 960 --height 540 --warmup 5 --duration 20 --reps 1 \
    --variant ruthenium --variant 'opaque=--translucent off'
```

For the ocean flight use `--scene fly --at -120,82,-100,0,30`. For the aerial picture use `--client vanilla,ruthenium --scene idle --variant 'ruthenium=--light-flicker off' --bare-picture --at 4.5,116,2.5,90,35`. The window dimensions above assume the machine's 2× display scale. The harness also accepts a repeated `--setup` for server commands that build a scene after teleporting the player.

## Animated textures and the camera inside fluids

The block atlas's 56 animated sprites play as in 26.3: once in a frame in which the game has ticked, not while it is frozen, with every mip level written from its own frame and interpolated sprites mixed every tick. The camera's medium sets the fog and, in lava and powder snow, hides the sky. In water and lava the field of view shrinks to six sevenths, and with the player's eyes in water the picture of water lies over the frame. The eyes get used to water over 30 seconds. `--animations off` and `--fov-effects off` turn off what the names say; pictures that are to be the same file need the first.

### Pictures from inside

Pairs at 1920 × 1080 with `--bare-picture`, which turns vanilla's FOV Effects off, so Ruthenium ran with `--fov-effects off`; its narrowing is tested by rule only. Positions and chunk checksums agree in each pair.

| Scene | Identical | Within 1 of 255 | More than 8 apart | What differs |
|---|---:|---:|---:|---|
| A lit tank of water, after 30 s in it | 86.38% | 94.84% | 1.22% | Vanilla's particles, our crosshair, the surface's animation frame |
| In powder snow | 99.99% | 99.99% | 0.01% | Our crosshair |
| In lava | 59.65% | 59.65% | 40.35% | Vanilla's burning overlay, over plain fog of the same colour in both |
| A kelp forest in the ocean | 24.67% to 84.32% | | | The plants' animation frames, right in front of the camera |

The two clients' animations have no common phase, so a picture of moving textures is not a comparison of them. Their timing is checked instead against the game's own animation states: 2,000 made-up animations over 64 ticks each, every frame, tick count and decision to write equal. A debug run inside the tank under synchronization validation logged no message.

### Cost

A tick writes about 42 sprites, 0.5 MiB with padding and mips. Written in one frame, that took 0.75 ms of the main thread at P99.9 in idle and cost 18% of the 1% low and 35% of the 0.1% low in a single pair. Two changes followed: a row of a frame is mixed once and copied over the padding, and frames shorter than 8 ms share a tick's sprites among them. Single 20 s runs with desktop encoding active, after both changes:

| Route | Off, FPS avg / 1% / 0.1% low | On, FPS avg / 1% / 0.1% low | GPU median ms, off → on |
|---|---:|---:|---:|
| Idle | 2204 / 1433 / 1169 | 2176 / 1410 / 1146 | 0.278 → 0.278 |
| Idle, again | 2197 / 1406 / 1173 | 2182 / 1410 / 1165 | 0.278 → 0.278 |
| Fly | 2818 / 1584 / 1211 | 2804 / 1574 / 1254 | 0.214 → 0.214 |

An earlier idle run with animations on, the first of its set, gave 2061 / 1123 / 810 and is not in the table; the two later pairs with the order reversed did not repeat it. No full campaign was run.

## Block entities

Chests, shulker boxes, skulls and banners, from the working tree on top of `80d267f`. (Beds and sign boards are block models in 26.3 and were drawn already.) The mesh workers list a section's block entities; each frame those of the sections in view within 64 blocks are drawn as instances of model parts, one draw for all instances of a part in a layer. Needs `ruthenium prepare` again, for the models; `--block-entities off` leaves them out.

### Pictures

Pairs at 1920 × 1080 with `--bare-picture`, `--light-flicker off --fov-effects off` on our side. Of the boxes around each model in the scene by day: the chests, shulker boxes and skulls are within one step of 255 in 99.5% to 100% of their pixels (the lowest is the ender chest's box, which vanilla's particles cross). Faces turned by 90 degrees are often exactly one step off; the likely reason is that vanilla cuts a normal to whole 127ths, which was not tried. Banners cannot be compared pixel by pixel: the two clients' cloths were at different points of their wave (77.6% to 96.7% within one step for three, 14.7% for the orange one).

From behind, at midnight and without a wall, 92.7% of the whole picture is within one step and 0.33% more than 8 apart, where the water and our crosshair are.

**Vanilla's picture is sometimes darker, and it is vanilla's client that is wrong.** In some runs vanilla draws the outer faces of a freshly built wall or platform darker than we do (107 to 110 against 130 to 134 by day), as if sky light came only from above and the sides. In fifteen vanilla runs that built such a scene with a wall live, 2026-11-25, five were dark and ten were bright, of the same scene and of different ones, so it is a matter of timing and not of the blocks. For the dark run whose world was kept, the server's saved light, read by joining that world, is 15 everywhere beside the wall, as in a bright run, and a headless join that watched the scene being built has the same levels as the server sends on a rejoin. A run that loads the finished scene is bright. What goes wrong in vanilla's client was not traced. Before taking a brightness difference in freshly built scenes for ours, run vanilla again.

### Cost

One pair of 20 s in release on the RTX 4060, with the desktop being encoded: 420 block entities in view (1,140 parts in 9 draws) against `--block-entities off`. The GPU's median frame is 0.270 ms for 0.185, the main thread's work 0.202 ms for 0.071, of which gathering them is 0.087. A frame without block entities in range gathers nothing. The run with them had one interval of 135 ms, in which the terrain's stage was not slow (0.45 ms at most); it was not explained.

Not there: lids seen against vanilla in motion (their movement is tested by rule, their poses against the game's own models), a head's or a jaw's movement when powered, patterns of data packs whose picture is not named after their id, bells, conduits and the other animated ones, and the damage picture of a block being broken. Christmas chests go by UTC. No GPU fixture draws the models; the pictures above and a run under synchronisation validation without a message are what there is.

## Sign text

The text on standing, wall and hanging signs, from the working tree on top of `ccfd840`; results in `cache/bench/results/signs-2026-11-25-rtx4060` (not committed). It is drawn in the game's own fonts, read in place from the imported files (bitmap, space and Unifont providers; TrueType providers of resource packs are left out). Each glyph is one record of the frame's data; what surrounds glowing text is the same records drawn eight times by the vertex stage.

**Checks.** The game's own `Font` and line splitter give the same corners, depths and colours for every quad of 2,000 made-up lines (bold, italic, underlined, struck, coloured, `§` codes, line breaks, glowing), and its sign transforms are ours for all 80 of them. 40 deliberate breakages were tried: 39 were caught (four only after a test was added). 33 of them ran before the outline moved to the vertex stage and were not run again after it. The one not caught, that a picture is used a frame after it was staged, has no test and was not tried in a picture either. A debug run under synchronisation validation with 209 signs, one of them obfuscated and one changed by the server, gave no message; the glyph writing and the font's index were changed a little after it, and the pictures below are older than the second of those changes. GPU fixtures pass; none draws text.

**Pictures against vanilla** at 1920 × 1080. Close up at midnight: of five signs two are identical in every pixel, the others within one step of 255 in 99.94 to 100%; the whole picture 99.99%, what is left being the crosshair and a few pixels by the boards that were not looked into. Farther off by day, thirteen signs: 98.4 to 100% within one step in twelve boxes, 97.6% in the one whose line holds an Arabic ligature, which vanilla reorders from right to left and we do not. In each box 0.01 to 0.14% of the pixels differ by more than 8 steps; these were not looked into.

**Cost.** One pair of 20 s, clean (1 and 4 intervals over 2 ms): 209 signs in view, 104 of them glowing (10,219 glyphs in 2 draws). The main thread's work goes from 0.065 to 0.232 ms; in a split run with 208 signs that is 43 µs to gather them and 92 µs to write their records. The GPU's median frame goes from 0.217 to 0.282 ms in that 10 s split run. Glyphs are written anew every frame; a cached copy on the GPU would save that.

Not there: right-to-left text, a style's `font` and `shadow_color`, the forced Unicode and Japanese variants, the text filter's lines, the wider outline through a spyglass. `§r` inside a styled component is taken into account for drawing but not when a line's width is measured, where vanilla resets the style to none. Obfuscated text is drawn with at most 512 characters of each width, not all of them.

## Player heads, skins and players

From the working tree on top of `bd882dc`; results in `cache/bench/results/players-2026-11-25-rtx4060` (not committed). Heads with an owner, other players and mannequins are drawn with their skins, wide and thin arms, second layers and capes, in the game's poses: walking, sneaking, swimming, gliding, sleeping, dying, spinning, riding, striking, hurt, and the arms for what they hold or use. A skin is loaded on threads of its own: from the skin cache, else from `textures.minecraft.net` over HTTPS; what is missing shows the id's default skin meanwhile. As in the game, a player's skin counts only if Mojang's services signed it, a head's does not need to be, and a head or mannequin that gives only a name or an id is looked up. The bodies are the models the block entities are drawn from, through the same draws.

**Checks.** The game's own player model stands exactly so for 2,000 made-up states (every limb's place and turn, bit for bit), its 240 transforms are ours within 0.00002 and its capes within 0.0006 (the game goes by way of angles there), its 60 default skins and the three made-up pictures it turns into skins are ours. 58 deliberate breakages: 45 were caught at once, the other 13 after tests were added. A debug run under synchronisation validation with 124 figures gave no message. GPU fixtures pass; none draws a figure. **Not tried:** a download from the texture server, the services' real keys and a look-up (they are tested against stand-ins), and a signed skin on a live server.

**Pictures against vanilla** at 1920 × 1080, by day. A frozen scene of twelve mannequins (one of them invisible, and so not drawn) and five heads: the boxes around four figures without a shadow behind them and all five heads are within one step of 255 in 100% of their pixels. What differs by more in the others is the shadow on the ground behind them and the crosshair. A running scene of 24 mannequins with all 18 default skins, and one with another skin and a cape: the heads on blocks and the row of heads behind are within one step in 100% too; the rest was compared by eye, as arms are never still and not at the same moment in two clients. Live players (headless clients that walk, sprint, sneak, draw a bow, hold up a shield, strike and are hurt) were looked at, three at a time, beside the game's picture of three of them: they cannot be compared pixel by pixel.

**Cost.** One pair of 20 s with a video encoding session open on the GPU in both, so not a clean one: 100 mannequins in view, half of them with capes (1,250 parts in 25 draws). The main thread's work goes from 0.079 to 0.254 ms, 1.75 µs each; the GPU's median frame from 0.206 to 0.244 ms.

Not there (names, shadows, armour and what is held came next, below): elytra, heads on heads, arrows, parrots, fire and the glow's outline; the local player in third person, asleep, and the hand; a stab's arm, quick charge's shorter loading, the body of a horse or other animal ridden (its yaw is taken), a sleeping player put on its bed by the client; a pack's own skins for mannequins beyond its pictures. The order of two see-through figures is by their middles.

## Name tags, shadows, armour and held items

From the working tree on top of `4dfe421`; results in `cache/bench/results/equipment-2026-11-25-rtx4060` (not committed). Players and mannequins have their names over their heads, a shadow under them, their armour on and their items in their hands.

**Names.** A player's name is its team's prefix, its name and the team's suffix in the team's colour; a mannequin shows a custom name only if it is told to, with its description below. The score of the objective in the below-name slot stands under a name for those within the below-name distance. Team rules decide who sees a name (always, never, own team, other teams); the invisible, and one who carries another, show none. A name is drawn twice, as in the game: dimmed and through walls, then whole where nothing is in front. A sneaking player's is only the dimmed one, hidden by walls, within 32 blocks.

**Shadows.** One piece for every block under a figure that is seen and whole to stand on, as wide as the block's outline, darker in brighter light, fading with the height over the ground and gone from 16 blocks away. `--entity-shadows off` leaves them out.

**Armour.** From the game's equipment files: each layer's picture, the dye of leather, trims coloured by their material's palette (the darker one on armour of the same material), and the enchantment glint, which moves over a piece in the shader.

**Held items.** The item definitions of the game are read as it reads them (models, conditions, selections, ranges, composites, tints): 1,658 definitions bake into 2,109 models of 102,706 quads with no warning. A model is uploaded when a figure first holds it. Flat items get the game's generated sides; blocks use the block atlas; shields (with their patterns) and tridents are drawn from the game's own entity models. An item sits in the hand as its model's display entry says, mirrored in a left hand.

### Pictures

Pairs at 1920 × 1080 with `--bare-picture`, the game frozen unless said otherwise. Vanilla hides the HUD for such a picture and then shows names only of entities on a team, so every figure was put on one.

| Scene | Within 1 of 255 | What differs |
|---|---:|---|
| Eleven mannequins with names, on stone, glass, a barrier and leaves, one behind a pillar | every name and shadow | Two figures on glass and a barrier set that instant: vanilla lit them darker (its light after a block change, see sign text); not so with the game running |
| Live players: a team's prefix and colour, a score below the name | 10,500 of 10,500 pixels of the tags equal | |
| A sneaking live player | 10,491 of 10,500 equal, 9 one step off | |
| Ten in armour: diamond, dyed leather, trims (gold on iron, gold on gold, redstone on netherite), mixed pieces, one sneaking | 99.84% | The crosshair; a carved pumpkin on a head, which is not drawn |
| Fourteen holding, in both hands, swords, a bow, blocks, stairs, a trident, a crossbow, glass, a pane, shields, a mace, a rod, a lantern, a door, a spyglass | 99.70% | The crosshair; the arm of one holding a spear, raised since |

A name behind a pillar shows through it dimmed in both clients, and a sneaking player's does not. The glint moves by the clock in both and has no common phase: enchanted armour, items, a trident and trimmed armour were compared by eye. A shield with patterns matches; its glint is not drawn. A live player drawing a bow in a chestplate matches.

### Checks

880 tests in the workspace pass. 40 deliberate breakages of the new code (names, shadows, equipment, item models, records, packets): 34 were caught at once, the other 6 after tests were added. A debug run under synchronisation validation with 13 equipped and named mannequins (313 parts, 126 glyphs, 52 shadow pieces) gave no message; GPU fixtures pass, and none draws a figure. After the per-frame allocations were taken out (below), the armour scene was the same picture bit for bit, and the item scene too but for the spear's arm.

### Cost

One pass of 20 s for each, a video encoding session open on the GPU in all, so not clean: 100 mannequins in view, each with a name, four pieces of armour (iron, trimmed diamond, enchanted netherite or dyed leather), a shield, and a sword, bow or stairs. 3,000 parts, 1,508 glyphs and 256 shadow pieces in 30 draws.

| | Main thread ms | GPU median ms |
|---|---:|---:|
| No figures (`--players off`) | 0.120 | 0.264 |
| All of it | 0.782 | 0.390 |

That is 6.6 µs a figure on the main thread. Split by a second set of runs (before the allocations below were removed, when the whole was 0.872 ms): bare figures with shadows 0.176 ms over none, the names 0.078 ms, armour and items 0.49 ms; shadows alone 0.034 ms and 0.011 ms on the GPU. A part costs about 0.1 µs to place and 0.1 µs to sort and write, worn or not: nothing of a figure is kept between frames.

Taking string building and lists out of the per-frame path (layer pictures looked up once for an asset, a trim's picture found by the trim itself, leaves handed to a function, no list in placing a model) took gathering from 0.405 to 0.326 ms.

### Not there

Things worn that are not armour (a carved pumpkin, a head, a banner on the head; elytra); the spear's use and stab, a spyglass at the eye, quick charge; in the hand, the game's special models other than shield and trident (heads, banners, chests, shulker boxes, pots, conduits, copper golem statues) and what compasses, clocks, a cast rod, custom model data and component properties choose (their fallback is drawn); firework, map and team tints (the default), potions made only of their own effects (no colour mixed); a patterned shield's glint, decal trims, equipment and trims that are not the game's own; names of other entities, of what the crosshair is on, and the hide-HUD rule; shadows of anything else. Items named `_spear` stand in for the `spears` tag. All item models are baked at the join (about 9 MiB), and the models held so far share one 4 MiB vertex page that is never given back.

## First-person hand, block outline and breaking

Compared on 2026-11-25 with the 26.3 Vulkan client at 1920 × 1080 on the RTX 4060 Laptop. The bench's `--hand-picture` keeps the hand and outline, hides the HUD including the crosshair, and writes both clients' end-of-route PNGs beside their JSON. Results are under `cache/bench/results/hand-2026-11-25`.

| Scene | What the pictures show |
|---|---|
| Bare arm over existing terrain at `4.5,72,2.5,32,72` | All 35,530 pixels in an interior crop of the right arm agreed within one 8-bit channel step. Around the targeted leaves, 71,977 of 72,000 pixels agreed within one step. |
| Bare arm facing a newly placed smooth quartz block at `4.5,118,2.5,0,0` | The arm and block edges lined up. The arm and the front of the quartz were brighter in Ruthenium. Vanilla's client had sky light 11 at the eyes here, versus 15 in the terrain scene. Its own server stores 15 everywhere around the block: checked in the saved worlds of three of five repeated vanilla runs and of one Ruthenium run, and the sampled patches of all five vanilla pictures have the same darker averages. Ruthenium draws the server's light. |
| Diamond sword facing the same block, a second player mining | The sword silhouettes lined up. Comparing the front face with the unmined capture in each client, the same 1,463 pixels changed by more than 22 channel steps in both. This checks the placement and depth of a stage-zero crack on this solid face. |
| North-facing chest at the same position, a second player mining | The renderer drew 18 crack quads over the base, lid and latch. Against an unmined chest capture, both clients changed the same 1,425 pixels by more than eight channel steps in a 266 × 245 crop of the front. |

The numerical tests compare both arms' transforms, swing and use poses, five moments of spear use, and all six crack texture orientations with values produced by the installed vanilla classes. Six isolated mutations to the bare arm offset, spear pivot, west-face decal coordinates, line width, Quick Charge time and shared breaking stage each failed the corresponding test. The GPU fixtures and the live Ruthenium picture runs passed with Vulkan validation enabled.

In three consecutive 20-second idle captures over existing terrain, the two controls disabled the hand and outline. The middle run drew two arm parts and 12 outline edges. Timings are milliseconds per frame.

| Median | Before | Hand and outline | After |
|---|---:|---:|---:|
| Main-thread work | 0.0454 | 0.0497 | 0.0473 |
| GPU | 0.1499 | 0.1560 | 0.1499 |
| Frame interval | 0.2254 | 0.2369 | 0.2299 |

The median GPU difference was 6.1 µs. Frame interval P99 was 0.68–0.69 ms throughout. These measurements cover one idle view on this machine.

A second comparison kept the chest, hand, outline and invisible remote player in all three runs. Only the middle run had the remote player mine. The capture began and ended with 18 crack quads there and zero in both controls. The GPU medians were 0.2369, 0.2393 and 0.2369 ms. Main-thread work was 0.0958, 0.0987 and 0.0977 ms. The six-second samples kept the unchanging stage within its 20-second lifetime. A longer trial lost the stage during capture and was excluded.

Breaking overlays use the posed geometry of chests, shulker boxes, skulls and banners as well as terrain models. Spectators get an outline for menu providers and portals, including the lid checks on both halves of a chest. Filled maps still use the ordinary held-item path. The live pictures cover idle arms, a sword, stationary outlines and a remote miner. The animated hand poses have numerical checks.

## The HUD

Compared on 2026-11-25 with the 26.3 Vulkan client at 1920 × 1080 (GUI scale 4) on the RTX 4060 Laptop. The bench's `--hud-picture` begins the idle route in survival mode on the ground, leaves vanilla's HUD on (clouds off, FOV effects 0) and writes both clients' end-of-route PNGs beside their JSON; `--setup` commands stage the player. The warm-up was 40 seconds, so that the vignette, which follows the light a hundredth a tick, had settled in both clients. Results are under `cache/bench/results/hud-2026-11-25-rtx4060`.

Staged in every picture: nine stacks in the hotbar (a sword, 64 stone, 5 golden apples, a bow, 16 ender pearls, a worn pickaxe, 7 stairs, 3 glass, 12 torches), a shield in the off hand, armour, level 30 with 60 points more.

| Scene | Hotbar and its items | Hearts, armour, food, experience | Effect icons | Crosshair | Whole frame |
|---|---:|---:|---:|---:|---:|
| Looking up at noon; full health, four absorption hearts, hunger, three armour points, four effects | 100% (97.79% equal) | 100% (99.97% equal) | 100% (99.999% equal) | equal | 99.9994%, 12 pixels more than one step apart |
| The same view; 13 of 20 health, three and a half armour points, three effects | 100% (97.79% equal) | 100% (99.93% equal) | 100% (99.999% equal) | equal | 98.01% |
| The same view at midnight, no effects | 100% (97.79% equal) | equal | (none) | equal | 99.9996% (98.68% equal), 8 pixels more |
| That midnight frame without Ruthenium's vignette (`--vignette off`) | | | | | 75.91% |

Each figure is the share of a region's pixels within one step of 255 in every channel: the hotbar's is 976 × 96 pixels, the bars' 736 × 152, the icons' 320 × 216, the crosshair's 80 × 80. What is left of the second frame is vanilla's particles of the player's own effects, which Ruthenium does not draw. A first pair over the terrain at the spawn had the hotbar within one step in 99.96% of its pixels; the 36 others were single pixels of the grass seen through it.

- **Items** are not copied from an atlas as in vanilla, which draws each into a square of 16 GUI units and blits that: Ruthenium draws their models straight into the frame, through the same orthographic view and with the two lights vanilla's GUI has for flat items and for blocks. The pictures above are what says the two agree.
- **What moves** is checked by rule, against values the game's own classes give: the shaking of hearts, food and air draws `java.util.Random` as vanilla seeds it (thirty draws from the JVM), and the blinking, the regeneration wave, the bursting bubbles, the order and the flashing of effect icons, the bulge of a stack that grew and the cooldown's cover follow `Hud.java` in tests. 52 mutations of the new code each fail a test, 45 at once and 7 after their tests were made to tell (an indicator step, the wear bar's width, the second the lost hearts show, the experience bar's last unit, an icon's flashing, a stack replaced by one as large, the vignette across levels).
- **Cost.** With the second picture's HUD drawn (ten items, 61 quads of crosshair, hotbar, bars and icons, 28 glyphs) against `--hud off`, one 20-second pair at the spawn with a video encoding session open: main-thread work 0.124 against 0.109 ms a frame at the median, GPU 0.326 against 0.295 ms. That includes the vignette's quad over the whole frame, which is drawn for as long as it darkens anything (in daylight, about the first half minute in a level) and left out once it has faded. No full campaign was run.
- **Not drawn yet:** (The text has [come since](#the-huds-text).) A mount's hearts and jump bar, the locator bar; a spectator's hotbar and its crosshair on what it can open; subtitles and F3; the scope of a spyglass, a carved pumpkin's overlay, powder snow's frost, the portal's and nausea's overlays, the sleep fade, and the world border's red in the vignette. The HUD is drawn only while no screen is open (vanilla keeps it under one). The glint of an enchanted item in a slot is drawn as on a held one and was not compared (it moves with the clock). A bundle's bar, items larger than their slot (vanilla clips them), and the bubbles' popping sound are missing. The attack indicator can stand beside the hotbar in the layout, but no setting chooses it or the GUI scale yet.

## The HUD's text

From the working tree on top of `aec76a6`, 2026-11-25, on the same machine and with the same `--hud-picture` runs (1920 × 1080, GUI scale 4, looking up, 40 seconds of warm-up); results in `cache/bench/results/hud-text-2026-11-25-rtx4060` (not committed). Two bench options are new: `--tick <command>` has a data pack run a command every tick, so that the chat, the action bar and a title are always new when the picture is taken, and `--player-list` shows the player list in both clients without its key.

| Scene | Whole frame within one step of 255 | Equal |
|---|---:|---:|
| Three boss bars, a sidebar with a team's prefix and suffix and three ways of writing a score, a title with its subtitle, the action bar with a key's name, ten lines of chat (wrapped, two-line, `§` codes, Unifont, a translation, `say`, shadows of their own) | 100% | 97.93% |
| The same with a selected item's name, renewed every tick | 100% outside the hand | |
| The player list with a number for two of three players, a team's name and a spectator, over a boss bar | 99.293% | 97.23% |
| The player list with hearts (13, 27 and none), otherwise the same | 99.279% | 97.21% |

In the second scene the hand rises and sinks with every renewed item, in both clients and never in step; every pixel that differs is of it. In the last two the rest is vanilla's locator bar, which Ruthenium does not draw; nothing else differs. The first picture of all had 132 pixels of the bold, slanted title one column off: vanilla samples a glyph a hundredth of a texel inside its edge, and the HUD's glyphs do so now.

- **Checked by rule:** where lines break, against the game's own splitter (600 made-up texts with styles, codes and line breaks, 12 hashes); the readers of chat, player list, titles and level events, against bytes the game's own codecs wrote; the fading of chat, titles and the action bar, the boss bars' filling, the order of the sidebar and the list, the list's blinking hearts, item names and rarities, in tests after the game's rules. 81 mutations: 65 failed a test at once (one of them by never ending), 12 after tests were added, and four change nothing that can be seen. The sidebar's tests were among the 12: they had been left out of their file by mistake, which only the mutations showed.
- **The chat's protocol.** Signed messages are counted and the server told of them as vanilla does (after every 65th), their signatures kept so that others can refer to them and the server can have them taken away, and a message out of order ends the connection. The signatures themselves are **not verified**: a message with one counts as its sender's. No live signed message has been read.
- **Cost.** The first scene's text with the player list showing, five messages of chat arriving every tick, against `--hud off`, one 20-second pair over the terrain at the spawn: main-thread work 0.148 against 0.107 ms a frame at the median, GPU 0.332 against 0.283 ms; all of the HUD, not its text alone. No full campaign was run.
- **Not there:** typing in the chat, its scrolling, links and hover texts; text that runs from right to left, fonts other than the usual one, pictures within text; the options (chat scale, width, opacity and delay, backgrounds behind all text, notification time, GUI scale); blocked players; subtitles, toasts and F3; a boss bar's darkened sky, fog and music. Faces in the player list show on servers that check accounts and were not compared (the test server does not). The chat and the player list ask for memory every frame they show.

## The clouds

From the working tree on top of `c21bc81`, 2026-11-25, with the 26.3 Vulkan client at 1920 × 1080 on the RTX 4060 Laptop (display scale 2, `--width 960 --height 540`). `--clouds fancy@1000` (or `fast@1000`) has both clients draw their clouds where they are at game time 1000 and no part of a tick: vanilla's bench mod fixes the time `CloudRenderer.prepare` is given, Ruthenium takes `--clouds` and `--cloud-time`. Otherwise the bench keeps both clients' clouds off in pictures, and gives Ruthenium vanilla's Cloud Distance of its options file (64 chunks) in every run. Both clients' PNGs are under `cache/bench/results/clouds-2026-11-25-rtx4060`.

| View (`--at`) | Clouds | Equal |
|---|---|---:|
| From below, looking up (`4.5,110,2.5,0,-40`) | Fancy | 100% |
| The same | Fast | 100% |
| The same, at dusk (`--time 12800`, looking west, game time 51234) | Fancy | 100% |
| Inside the layer, in a gap (`4.5,192.5,2.5,30,-5`) | Fancy | 100% |
| Inside a cloud's cell (`54.5,192.5,-17.5,30,-5`) | Fancy | 100% |
| From above, looking down (`4.5,205,2.5,0,35`) | Fancy | 85.43%; every cloud pixel equal, the rest is the sea's animated water |

- **Checked by rule:** the cells and faces, against the game's own `CloudRenderer` run headless on 3000 made-up pictures (8 to 64 cells wide, alpha around the cut of 10, every camera position and both kinds, radii 1 to 30): the number and order of faces and an FNV-1a hash of them. Where the layer is, how it drifts and wraps, the colour's steps and the fog's end are unit tests.
- **When lists are made:** only when the camera's cell, its place below, inside or above the layer, or the options change, as vanilla's rule; a worker thread makes them (1.5 ms at the largest distance in a release build), and until a new list has arrived the old one is drawn from its own cell.
- **Not there:** the Improved Transparency (order-independent) path; clouds through the darkness and blindness fogs and the boss bar's fog, which the rest of the world does not have either.

## Where a frame's time goes

Medians of the runs' medians, in milliseconds, at the default options. Work is the main thread's own: events, the game's frame, the GUI, recording and submitting. The governor is the wait for the GPU to finish the frame before, which is what one frame ahead removes.

| Scene | Work | Work P99 | Events + game + GUI | Terrain: staging | Staging P99 / P99.9 / worst | Terrain: draws | Governor | Present | GPU | GPU P99 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| idle | 0.141 | 0.283 | 0.012 | 0.003 | 0.011 / 0.036 / 0.45 | 0.101 | 0.349 | 0.020 | 0.313 | 0.346 |
| spin | 0.129 | 0.262 | 0.003 | 0.001 | 0.008 / 0.031 / 0.27 | 0.100 | 0.292 | 0.020 | 0.262 | 0.339 |
| walk | 0.153 | 0.323 | 0.012 | 0.004 | 0.049 / 0.105 / 0.89 | 0.116 | 0.339 | 0.020 | 0.304 | 0.374 |
| fly | 0.080 | 0.247 | 0.003 | 0.001 | 0.054 / 0.185 / 0.95 | 0.064 | 0.151 | 0.019 | 0.144 | 0.342 |

- **The terrain's draws are most of the main thread's work**: testing some 4,000 meshes against the view, ordering those in it and recording 780 to 1,430 draws.
- **Everything that is not rendering is small.** Events, the game's frame (handing input to the game thread, the route's step) and the GUI take 3 to 12 µs a frame together at the median, and their P99s add up to 28 to 57 µs. A render thread of its own would have little to take off the main thread here.
- **Staging** is the render thread's part of getting meshes to the GPU: taking in what changed, taking finished meshes from the workers, reserving staging, placing them in pages and recording their copies. No mesh byte is copied on the render thread. While chunks stream in, it stays under 0.06 ms in 99 frames of 100 and under 0.19 ms in 999 of 1000; its worst frame in any run here took 1.8 ms.
- **With the GPU waited for**, a frame is the main thread's work and then the GPU's, one after the other: 0.53 ms in idle. One frame ahead lets them overlap, and the interval falls to whichever is longer: 0.375 ms in idle, 0.19 ms in fly. The price is that a frame's input is up to one GPU frame older when it is shown, which at these rates is 0.15 to 0.3 ms and on a slow GPU is a whole frame. The default stays at none ahead without VSync.

### Ways to draw the sections

| | Draw calls / vertex buffer binds (idle) | Work, idle / fly | Average FPS against the default |
|---|---:|---:|---:|
| Direct, nearest first (default) | 1425 / 1096 | 0.141 / 0.080 | |
| Direct, a page at a time | 1425 / 64 | 0.137 / 0.075 | +0.9% idle, +1.1% fly |
| Indirect: one call a page and layer | 64 / 64 | 0.122 / 0.071 | +4.3% idle, +5.5% fly, +2.8% walk, +0.2% spin |

Indirect draws take 10 to 13% off the main thread's work and gave more frames in 11 of 12 paired runs, by 3% on average: little, because the GPU is what bounds these scenes. GPU time is the same in all three, though the page-wise order is not strictly front to back. Direct draws remain the default until other GPUs have been measured; `--terrain-draws` chooses.

**Uploads on the graphics queue alone** (`--no-transfer-queue`) change nothing that these runs can show: within 0.3% on average FPS and 1% on the lows, with the same staging times.

## Meshing, uploads and latency

Over the 60 s capture, at the default options:

| Scene | Meshes installed | Of them first meshes | MiB uploaded | Meshing, ms a section | First mesh, P50 / P99 | A change from the server, P50 / P99 |
|---|---:|---:|---:|---:|---:|---:|
| idle | 1,337 | 0 | 29 | 0.22 | | 1.7 / 3.5 ms (3 / 5 frames) |
| walk | 18,563 | 5,160 | 258 | 0.12 | 13 / 41 ms (20 / 62 frames) | 9.6 / 44 ms (15 / 73 frames) |
| fly | 56,237 | 49,033 | 711 | 0.10 | 7.0 / 22 ms (18 / 45 frames) | 1.7 / 6.1 ms (4 / 13 frames) |

Latency is from the frame in which the render thread learns of a change to the first frame that draws the mesh made of it; a first mesh counts from when the chunks around its section were there. No reservation of staging was refused and the per-frame upload budget refused nothing in any run. The main thread never had to wait for the game thread: at most 57 frames of input waited at once, the oldest for 11.8 ms.

**The player's own changes.** The `dig` scene, which only Ruthenium plays, holds the attack key for twelve seconds with the player looking straight down in creative mode: 23 blocks broken, each one a change the client makes before the server answers. The 21 meshes they caused were on screen 2 frames later at the median and 3 at most, which was 0.8 ms and 1.7 ms. The windowed runs and a headless run recorded the same position on every tick.

**In the idle scene nothing is idle.** Through the whole capture about 22 sections a second are meshed again: 6 for a block that changed in them, the rest for light or for a change next to them.

## Memory

| Scene | Buffer pages, MiB | Of it meshes | Free (largest piece) | Allocations that found no room | Process RSS, MiB | GPU memory of the process, MiB |
|---|---:|---:|---:|---:|---:|---:|
| idle, spin | 128 | 124 | 4.2 (2.3) | 0 | 351 | 204 |
| walk | 152 | 142 | 10.4 (2.2) | 149 | 357 | 228 |
| fly, at its end | 196 | 183 | 12.9 (3.6) | 96 | 356 | 272 at most |

- **The terrain takes more than the 108 MiB planned for it** in every scene, and 183 MiB over the densest stretch of the fly route (it takes 117 MiB over the sparsest). The cause is the one above: sections that nothing can be seen of are meshed too. The pool's limit is twice the plan for now, and no mesh was refused.
- **Fragmentation.** Every allocation that found no room did so with enough free bytes in total, spread over pages in pieces too small for it, and each made the pool add a page. In flight the pool added 55 pages of 4 MiB and released 6. Ranges are never moved.
- **Against the targets** of 600 MB of RSS and 300 MB of GPU memory at this view distance: 357 MiB and 272 MiB at most. The vanilla client's runs took 4.9 GiB and 783 MiB.

## What the measurements changed

Three things were found with the per-frame log (`--frame-log`) on the fly route and fixed before the runs above. Before them, the default options gave about 2,300 FPS there with 1% lows near 370.

- **Sections were meshed again when a chunk next to theirs left.** In flight that is every section of the row behind the player, once more, just before it leaves too: twice the meshing and uploads for nothing. A mesh now stays as it is when a neighbouring chunk goes, as in vanilla, and is made again if that chunk comes back.
- **A frame went through every loaded section to find the ones to draw**, and each section's record held the state of the 27 sections its mesh depends on: some 15 MB read every frame. The meshes to draw are now kept together, and that state apart. The main thread's work in flight fell from 0.188 to 0.080 ms.
- **A chunk arriving or leaving looked at every loaded section**, and judged each of its neighbours' sections with 27 look-ups. Chunks now go by their keys, and a chunk's sections are judged together from one reading of the level around them. The worst staging frame fell from 8.4 ms to 1.8 ms.

Two runs before those fixes also showed frames waiting 2.5 ms for the GPU about 26 times a second while its own work took 0.15 ms; later runs did not. The cause was not found. It may be a viewer on the streamed desktop, which is why the task records the encoding sessions now.

## Not measured yet

- **The Intel iGPU, AMD and Macs.** Every number here is one discrete NVIDIA GPU's. Which way of drawing is fastest, and whether a frame ahead pays, may differ where the GPU is slow and memory is shared.
- **The whole picture.** Everything [still left out](#what-ruthenium-draws-and-what-it-does-not) has yet to be drawn, and no ratio here is a result until it is. Stationary comparisons cover three times of day at spawn and six views with fluids and glass. [Time of day](#the-two-pictures) · [Fluids and glass](#pictures-at-rest).
- **The shader's variants and the sky's order on the Intel iGPU**, where a frame is bound by its pixels: what RGSS, the shorter RGSS, fog and fade and the sky drawn late each cost there is what the options of [what it costs](#what-it-costs) are for.
- **Whether vanilla draws the same sections.** Cave culling follows vanilla's rules, but the vanilla runs did not record how many sections they drew, and the walk's order of work differs once meshes arrive.
- **The picture in motion with cave culling.** The pictures compared are of scenes at rest. What a moving camera can miss is a section between being reached and having its mesh, 5 to 6 ms at P99 on these routes.
- **The walk on a slow processor**, where its 4 ms would be longer, and whether meshing what is inside the view first would show.
- **Releasing the meshes of sections that are no longer reached**, which the fly route's 151 MiB may need.
- **Input-to-photon latency**, and so what a frame ahead costs on screen. It needs a hardware rig.
- **Pacing with VSync and with a frame limiter** in the world, and the per-frame upload budget derived from the frame's pace, which the client does not report to the upload ring yet.
- **The same runs with nothing else on the GPU**, and what a viewer on the streamed desktop costs the vanilla client.
- **Render-thread heap allocations** while chunks stream in.
- **Edits at the scale of play**: bridging while turning, placing at a section corner, a resource pack reload. The `dig` scene is one column of 23 blocks.
- **Relocating ranges** in the buffer pages, which the fragmentation above may or may not need once fewer sections have a mesh.

## Reproducing

```sh
cargo xtask bench setup          # once: creates cache/bench/world
cargo xtask bench dedicated --accept-eula --client vanilla   # the vanilla runs to compare with, about 25 minutes
cargo xtask bench dedicated --accept-eula --client ruthenium \
    --variant ruthenium --variant 'mdi=--terrain-draws mdi' --variant 'fa1=--frames-ahead 1' \
    --baseline cache/bench/results/<the vanilla runs>       # 4 scenes × 3 variants × 3, about 50 minutes
cargo xtask bench dedicated --accept-eula --client ruthenium \
    --variant graph --variant 'frustum=--terrain-culling frustum' --variant 'fa1=--frames-ahead 1' \
    --baseline cache/bench/results/<the vanilla runs>       # cave culling against the frustum alone
cargo xtask bench dedicated --accept-eula --client ruthenium,headless --scene dig
cargo xtask bench report <folder> --baseline <folder>       # the tables again
```

```sh
# The terrain's shader and the sky: the default against the build drawing as before, then each part
cargo xtask bench dedicated --accept-eula --client ruthenium \
    --variant rgss --variant 'before=--terrain-shader plain --sky off' \
    --baseline cache/bench/results/<the vanilla runs> --screenshot
cargo xtask bench dedicated --accept-eula --client ruthenium --scene idle,fly --reps 1 --warmup 5 --duration 20 \
    --variant rgss --variant 'ref=--terrain-shader rgss-reference' --variant 'nearest=--terrain-shader nearest' \
    --variant 'plain=--terrain-shader plain' --variant 'skyfirst=--sky first' --variant 'nosky=--sky off'
# First-person view over existing terrain. Both PNGs are in the results directory.
cargo xtask bench dedicated --accept-eula --client vanilla,ruthenium --variant 'ruthenium=--light-flicker off' \
    --scene idle --at 4.5,72,2.5,32,72 --reps 1 --warmup 5 --duration 6 --hand-picture
# The HUD in survival mode over the sky, both clients' PNGs in the results directory
cargo xtask bench dedicated --accept-eula --client vanilla,ruthenium --variant 'ruthenium=--light-flicker off' \
    --scene idle --at 4.5,71,2.5,0,-90 --reps 1 --warmup 40 --duration 5 --hud-picture \
    --setup 'give @a stone 64' --setup 'xp set @a 30 levels'
# The HUD's text: what would fade is sent again every tick
cargo xtask bench dedicated --accept-eula --client vanilla,ruthenium \
    --variant 'ruthenium=--light-flicker off' --scene idle --at 4.5,71,2.5,0,-90 \
    --reps 1 --warmup 40 --duration 5 --width 960 --height 540 --hud-picture \
    --tick 'title @a title {text:"Title"}' --tick 'tellraw @a {text:"a message"}'
# The clouds from below, Fancy, at a fixed game time
cargo xtask bench dedicated --accept-eula --client vanilla,ruthenium \
    --variant 'ruthenium=--light-flicker off --fov-effects off' --scene idle --reps 1 --warmup 5 --duration 10 \
    --bare-picture --clouds fancy@1000 --at 4.5,110,2.5,0,-40
# One frame of each client, vanilla's without HUD, hand, vignette and clouds (its PNG is
# cache/bench/game/screenshots/ruthenium-bench-idle.png, Ruthenium's in the results folder)
cargo xtask bench dedicated --accept-eula --client vanilla,ruthenium --variant 'ruthenium=--light-flicker off' \
    --scene idle --reps 1 --warmup 5 --duration 10 --bare-picture --time 12600
```

At a display scale of 2, add `--width 960 --height 540`. `--screenshot` saves the frame after each route's end next to the run's file. A variant's options are the client's own (`ruthenium --help`); add `--frame-log <file>` to one for a line per frame with where its time went. Each run's file holds the frame intervals, the route's record and Ruthenium's own summary (phases, terrain, uploads, latencies) under `ruthenium`; trial folders with the client's and the server's logs are under `cache/bench/dedicated`.

Nothing else may build or play while the runs go: check the power profile, and the table of runs for a video encoding session.
