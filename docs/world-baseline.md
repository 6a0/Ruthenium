# Ruthenium world baseline

What Ruthenium's world renderer costs on the routes the vanilla client was measured on, and where its frame time, memory and latency go. It is a first measurement of an unfinished renderer: read [what the scene leaves out](#what-ruthenium-draws-and-what-it-does-not) before any ratio. The vanilla numbers are the dedicated-server runs of [baseline.md](baseline.md); the menus' own cost is in [menu-baseline.md](menu-baseline.md).

Recorded on 2026-11-24 from the working tree on top of `d447d22`, in release.

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

## Against vanilla

The first set: every scene, with the options as they are by default, with the sections drawn by multi-draw indirect, and with the GPU allowed one frame of work in hand when the next frame samples its input.

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

- **Left out:** water and lava, translucent blocks, block entities, entities and the player's hand, the sky, clouds, fog and the fade-in of new chunks, smooth lighting and ambient occlusion, biome colours, animated textures, the time of day, particles and every part of the HUD but the crosshair. Vanilla draws all of them in its runs.
- **Drawn in excess:** every loaded section that has anything to draw gets a mesh, and every one of those inside the view's frustum is drawn, hidden or not. Vanilla builds and draws only what its occlusion graph reaches. At the spawn that is 925 sections and 522,119 quads in view, of 4,037 with a mesh among 15,288 loaded.
- **The same pixels whichever way the sections are drawn.** The frame after the route's end, captured with `--screenshot`, is the same file byte for byte with direct, paged and indirect draws, at the spawn and at the end of the fly route.

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
- **The whole picture.** Everything [left out](#what-ruthenium-draws-and-what-it-does-not) has yet to be drawn, and what is hidden has yet to be left out. No ratio here is a result until both are done; the comparison of images with vanilla's waits for the same.
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
cargo xtask bench dedicated --accept-eula --client ruthenium,headless --scene dig
cargo xtask bench report <folder> --baseline <folder>       # the tables again
```

At a display scale of 2, add `--width 960 --height 540`. `--screenshot` saves the frame after each route's end next to the run's file. A variant's options are the client's own (`ruthenium --help`); add `--frame-log <file>` to one for a line per frame with where its time went. Each run's file holds the frame intervals, the route's record and Ruthenium's own summary (phases, terrain, uploads, latencies) under `ruthenium`; trial folders with the client's and the server's logs are under `cache/bench/dedicated`.

Nothing else may build or play while the runs go: check the power profile, and the table of runs for a video encoding session.
