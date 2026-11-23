# Ruthenium menu baseline

What Ruthenium's own window and menus cost before a world is drawn, and how the frame loop behaves at the window system's edges. It is the reference for later changes to the frame loop and the swapchain, measured with the tools in [Reproducing](#reproducing). The vanilla numbers it is compared with are in [baseline.md](baseline.md).

Recorded on 2026-11-23 from the working tree on top of `cc75339`, in release.

## Machine and conditions

The laptop of the vanilla baseline, in Discrete GPU mode: NVIDIA GeForce RTX 4060 Laptop GPU, driver 620.70.06, KDE Plasma on Wayland, 2560 × 1600 at 240 Hz with a display scale of 2.

**These numbers are provisional.** The conditions were not the vanilla baseline's:

- The firmware platform profile was `balanced`, not `performance`.
- The desktop was in use: Sunshine and Discord were running, and the panel's variable refresh rate was set to Always.
- Round-to-round spread was 20–30% on average FPS and more on the lows (see [Throughput](#throughput-without-vsync)). Small differences mean nothing here.

The runs were repeated later the same day at the `performance` profile with variable refresh rate off: see [At the performance profile](#at-the-performance-profile). The tables before that section are from the conditions above.

The Intel iGPU needs the laptop's Hybrid mode and is not measured yet.

## Launch

Milliseconds after the process starts; medians of 5 launches, with the files in the page cache.

| Pipelines | Window | Device | Renderer | First frame | Menus faded in |
|---|---:|---:|---:|---:|---:|
| compiled (no cache) | 145 | 250 | 274 | 282 | 850 |
| from the cache | 121 | 220 | 232 | 238 | 807 |

Vanilla's first frame comes after 4.53 s and its menu after 7.33 s. The six menu pipelines compile in 12 ms and load from the cache in 1 ms, so the cache changes little yet. The glyph atlas and the logo are baked on another thread in about 140 ms; the last column is the end of the menus' fade-in, not the first text on screen.

## Memory

A 1920 × 1080 pixel window with three swapchain images, VSync on, 30 s each. RSS and GPU memory are sampled from outside the process; the allocated columns are what Ruthenium itself asked the driver for.

| Scene | RSS MiB | GPU MiB (`nvidia-smi`) | Textures and targets MiB | Staging MiB | Per-frame data MiB | Swapchain MiB (estimate) | Driver's heap usage MiB (device / host) |
|---|---:|---:|---:|---:|---:|---:|---:|
| empty window | 208 | 39 | 1.0 | 16.0 | 2.0 | 23.7 | 33.4 / 20.0 |
| menus | 215 | 56 | 16.2 | 16.0 | 2.0 | 23.7 | 50.3 / 20.0 |

Vanilla's Vulkan title screen holds 301 MiB of GPU memory and 2940 MiB resident.

- **GPU memory.** The swapchain, the per-frame data and the textures account for 26.7 of the empty window's 39 MiB and 41.9 of the menus' 56 MiB. The other 12–14 MiB is the driver's own. The staging ring is host memory on a discrete GPU and is not in the GPU figure.
- **The menus' 16.2 MiB** is the glass backdrop (a window-sized image and two quarter-sized ones, 8.9 MiB at this size), the starfield (1.0 MiB), and the font atlas and logo.
- **Resident memory** is mostly not Ruthenium's. Of the empty window's 213 MiB, 69 MiB is anonymous memory, 28 MiB is the driver's shared mapping, and 116 MiB is file-backed: the NVIDIA libraries (about 50 MiB), and `libLLVM` with Mesa's software drivers (about 28 MiB), which the Vulkan loader maps while it lists the GPUs even though Ruthenium never uses them. The executable is 7 MiB and the heap 10 MiB.

## Throughput without VSync

The menus at 1708 × 960 pixels, IMMEDIATE, no frame limiter; medians of 4 rounds of 15 s on each side of an A/A run (the same build twice).

| Metric | Median | Spread between rounds |
|---|---:|---:|
| Average FPS | 9440 | 32% |
| 1% low FPS | 1260 | 110% |
| 0.1% low FPS | 590 | 76% |
| Frame time, median | 0.078 ms | 5% |
| Main-thread work, median / P99 | 0.018 / 0.083 ms | 5% / 39% |
| Governor wait, median | 0.026 ms | 6% |
| Acquire, P99 | 0.67 ms | 160% |
| Present, P99 | 0.082 ms | 29% |
| GPU time, median | 0.020 ms | 3% |

The medians repeat within about 5%; the average and the lows do not. The tail is the acquire: `vkAcquireNextImageKHR` usually returns in about 15 µs, but its P99 is 0.7 ms, which is also the P99 of the whole frame's CPU time.

### Frames ahead

By default, without VSync, a frame samples input only once the GPU has finished the frame before it (0 frames ahead). `--frames-ahead 1` lets one frame overlap, as vanilla does. On the menus:

| Metric | 0 ahead | 1 ahead | Change | Verdict |
|---|---:|---:|---:|---|
| Frame time, median (ms) | 0.077 | 0.069 | −10% | better |
| Input sampled to present returned, median (ms) | 0.049 | 0.068 | +38% | worse |
| Governor wait, median (ms) | 0.026 | 0.001 | −98% | better |
| Average FPS | 9340 | 8610 | −8% | within the spread |
| 1% low FPS | 1240 | 1010 | −19% | within the spread |
| 0.1% low FPS | 519 | 374 | −28% | within the spread |

One frame of overlap buys a tenth off the median frame time and pays for it in input age; the lows did not improve. The menus keep the GPU busy for 0.02 ms a frame, so this says little about a world. The default stays at 0 until the comparison is repeated with terrain.

## Pacing with VSync

FIFO at 240 Hz, 1920 × 1080 pixels. The menu's gate is P99.9 frame time at most 1.5 times the median.

| Run | Frames | Median ms | P99 ms | P99.9 ms | P99.9 / median | Acquires over 10 ms |
|---|---:|---:|---:|---:|---:|---:|
| menus, 13 s | 3052 | 4.16 | 4.78 | 7.56 | 1.82 | 0 |
| menus, 13 s | 3045 | 4.16 | 5.03 | 7.71 | 1.85 | 1 |
| menus, 13 s | 2925 | 4.20 | 24.5 | 35.3 | 8.41 | 74 |
| menus, 13 s | 2518 | 4.16 | 34.6 | 35.3 | 8.49 | 90 |
| menus, 30 s | 5896 | 4.20 | 34.6 | 36.0 | 8.58 | – |
| empty window, 30 s | 7136 | 4.16 | 6.07 | 30.7 | 7.39 | – |

Two kinds of run came out of the same command. In a quiet one, about one frame in a thousand takes two refresh intervals, and the gate is missed at 1.8. In a disturbed one, the acquire blocks for about 34.5 ms in bursts of three, with nothing waiting on the CPU or the GPU: the main thread's work is 0.1 ms and the GPU's 0.05 ms a frame. Identical runs back to back were quiet or disturbed, so the bursts come from the desktop (the compositor's variable refresh rate or the screen capture are the suspects), not from Ruthenium. They need a controlled session before the gate is judged.

## Swapchain recreation and minimising

`--exercise-swapchain` resizes the window every 25 frames and switches VSync every 40, for 8 s under the validation layer:

| | |
|---|---|
| Swapchains created | 135 |
| Recreation, median / maximum | 0.93 / 2.83 ms |
| Acquire, median / P99 / maximum | 0.14 / 27.6 / 107 ms |
| Acquire timeouts (100 ms each, no frame presented) | 3, two of them back to back: 205 ms without a frame |
| Acquires over 20 ms | 40, none of them the first from a new swapchain (that one takes about 10 µs) |
| Validation messages | 0 |

`--exercise-minimize` minimises the window and brings it back every 1.5 s. Under XWayland (`SDL_VIDEODRIVER=x11`) each return made a new swapchain in about 2.5 ms, with no timeout and no skipped frame; the longest acquire was 60 ms. Wayland has no request to bring a window back, so there the window stays minimised and the loop idles at 10 Hz.

## After the swapchain changes

Measured the same day, on top of `859d243`, in the same conditions. Since the baseline above:

- **A replaced swapchain is destroyed when its presents are over**, not a fixed number of frames later. With `VK_KHR_swapchain_maintenance1` (or the EXT one) every present carries a fence that says so. Without either, the fence of the acquire that next returns the same image does, and a replaced swapchain waits for a present to a later one to finish. `--swapchain-maintenance khr|ext|none` picks the path.
- **The swapchain is replaced at the start of the frame**, before anything is recorded for its size or format, so a frame never draws glass without its backdrop, and the first frame at a new size is complete.
- **An acquire waits two display (or frame limiter) intervals after the frame sampled its input**, and at least 8 ms, instead of up to 100 ms. Then the frame is submitted without an image and the loop goes back to its events. `--acquire-wait 100` is the old wait.
- **A surface with no 8-bit UNORM format in the sRGB colour space is refused** with what it offers instead, at start-up and when a swapchain is replaced. Before, its first format was used, whatever it was.

### Recreation

`--exercise-swapchain` now also replaces the swapchain in the first frame, in three frames running every 60, and twice in one frame every 97, and captures a swapchain's first frame every 60. 8 s under the validation layer, in release:

| | Present fences (KHR) | Present fences, `--acquire-wait 100` | Acquire fences (`none`) |
|---|---:|---:|---:|
| Swapchains created | 242 | 230 | 233 |
| Destroyed while running | 241 | 229 | 230 |
| Most waiting to be destroyed at once | 2 | 2 | 4 |
| Recreation, median / maximum ms | 1.05 / 12.4 | 1.07 / 9.1 | 1.04 / 8.1 |
| Acquire, median / P99 / maximum ms | 0.17 / 10.0 / 11.0 | 0.16 / 27.0 / 36.8 | 0.16 / 9.7 / 10.5 |
| Acquires given up (frames without a present) | 142, 8.3 ms each | 0 | 127 |
| From one event poll to the next, P99 / maximum ms | 11.3 / 15.2 | 28.2 / 35.9 | 11.3 / 13.6 |
| Captures of a swapchain's first frame | 37 | 35 | 35 |
| Frames that drew glass without its backdrop | 0 | 0 | 0 |
| Validation messages | 0 | 0 | 0 |

The long acquires are the compositor's, after a resize; another pair of runs had them at 50 to 60 ms, where the fixed wait polled events every 56 ms at P99 and the shorter wait every 12 ms. The frames presented are no closer together with the shorter wait (their P99 interval is 27 ms either way): what it buys is the loop returning to its events, and the next frame being drawn from newer input.

The same exercise passes with the EXT extension, with `--exercise-uploads` (every chunk intact; textures destroyed before their upload arrives included), under XWayland, where a resize makes the acquire report out of date (45 times in 8 s), and on Mesa's llvmpipe, whose present fences are Mesa's own.

A screenshot of the first frame after a resize (`--screenshot-resize`) is identical, pixel for pixel, to one of a window started at that size (2200 × 1240 pixels, the server list with its glass shapes).

### The rest, against the baseline

| | Baseline | Now |
|---|---:|---:|
| First frame, pipelines compiled / cached (ms) | 282 / 238 | 275 / 277 |
| Menus faded in (ms) | 850 / 807 | 844 / 846 |
| GPU memory, empty window / menus (MiB) | 39 / 56 | 39 / 56 |
| Uncapped menu, frame time median (ms), A/B of 4 rounds | 0.074 | 0.075 |
| VSync pacing, quiet runs: P99.9 / median | 1.82 to 1.85 | 1.64 to 1.85 |
| Minimise and restore under XWayland: longest acquire (ms) | 60 | 11 |

With VSync, three runs of 13 s with each wait, in turns, came out the same: a median of 4.17 to 4.18 ms, P99 of 4.9 to 5.3 ms and P99.9 of 7.4 to 7.7 ms either way. No run was disturbed, so the 34.5 ms bursts of the baseline were not seen and the shorter wait is not measured against them. At start-up it gives up two to five acquires while the compositor maps the window, each about 8 ms, before the measured part of a run.

The uncapped runs showed something about the machine, not the change. It has two states, with a frame time median of 0.075 ms or of 0.128 ms (GPU time 0.020 or 0.026 ms a frame), and moves between them within a run or stays in one for minutes. In 28 runs of 6 s taken in turns, the baseline's build was in the slow state in 3 of 7 and this one in 3 of 7, 2 of 7 and 4 of 7 for its three settings. An A/B here needs the median per second of each run looked at before its averages are believed.

## At the performance profile

The firmware platform profile `performance` (the vanilla baseline's), variable refresh rate set to Never, the build with the swapchain changes. Sunshine was still capturing the desktop: the machine was being used through it.

| | `balanced`, VRR Always | `performance`, VRR Never |
|---|---:|---:|
| Window / first frame, pipelines cached (ms) | 140 / 277 | 84 / 189 |
| Menus faded in (ms) | 846 | 755 |
| GPU memory, empty window / menus (MiB) | 39 / 56 | 39 / 56 |
| Uncapped menu: average FPS | 9400 to 11000 | 12900 to 13500 |
| Uncapped menu: frame time median (ms) | 0.075 or 0.128 | 0.058 to 0.059 |
| Uncapped menu: main-thread work, median / P99 (ms) | 0.018 / 0.05 | 0.012 / 0.042 |
| VSync, menus: P99 / P99.9 (ms) | 5.4 / 6.8 | 4.6 / 6.0 |
| VSync, menus: P99.9 / median | 1.64 to 1.85 | 1.42 to 1.50 |
| Swapchain recreation, median (ms) | 1.05 | 0.52 |

- **The two states were the `balanced` profile's.** At `performance`, 14 runs of 6 s taken in turns with the baseline's build had no half second in the slow state for either build (medians of 0.058 to 0.065 ms). Right before, at `balanced` with variable refresh rate already off, the slow state was still there, so the refresh setting is not what causes it.
- **The pacing gate is met, narrowly:** 1.42 to 1.50 in four runs of 13 s and two of 30 s, with the capture running.
- **The change costs nothing measurable here either:** the A/B against the baseline's build has a frame time median of 0.058 against 0.059 ms, within the spread, and the same memory.
- The 34.5 ms acquire bursts did not come back in any run of the day after the baseline's, at either setting, so what caused them is still open.

## Packaging the menu

Recorded on 2026-11-23 from the working tree on top of `92ab409`, in release, at the `performance` profile with variable refresh rate set to Never and Sunshine capturing the desktop.

### The texture table

The table had 16,384 slots in update-after-bind descriptor sets. It now asks for 1,024 in ordinary sets, takes update-after-bind sets only when they hold a larger table than ordinary ones can, and needs at least 256 (`--texture-slots`, `--texture-table`). Both GPUs here allow about a million textures either way, so both get ordinary sets.

| Slots, sets | Table made in (ms), RTX 4060 | llvmpipe |
|---|---:|---:|
| 16,384, update-after-bind (before) | 0.31 | 0.84 |
| 16,384, ordinary | 0.29 | 0.90 |
| 2,048, ordinary | 0.16 | 0.12 |
| 1,024, ordinary (default) | 0.14 | 0.07 |
| 256, ordinary | 0.13 | 0.04 |

Medians of seven launches on the RTX 4060 and three on llvmpipe, taken in turns.

- **The smaller table saves a fraction of a millisecond at start-up and no memory that shows:** resident memory was 190.3 to 190.6 MiB at every size on the RTX 4060, and 220 to 225 MiB with no order on llvmpipe.
- **A frame costs the same.** An A/B of the default against the old table (`--b-args '--texture-slots 16384 --texture-table update-after-bind'`, six rounds of 5 s) has a frame time median of 0.058 ms on both sides and every metric inside the spread.
- **The menus of a profile with no account or server take 8 slots at most** (7 textures and one on its way out). The client prints the peak at exit, and `--bench-out` records it, for the working sets to come: resource packs, skins, maps.
- 1,024 is kept as the default and 2,048 is not needed yet. A table that is full refuses new textures and says whether slots are on their way back; the menus' own uploads then wait for a slot instead of giving up.

`--exercise-textures` fills the table until a texture is refused, holds it, destroys half of the textures (every third time, all) and fills up again, while one pixel per slot is drawn with the GUI pipeline each frame and read back. In 6 s at 256 slots under the validation layer:

| | RTX 4060, ordinary | update-after-bind | llvmpipe, ordinary | update-after-bind |
|---|---:|---:|---:|---:|
| Slots compared, wrong | 232,522, 0 | 233,519, 0 | 270,378, 0 | 272,878, 0 |
| Times the table was full | 91 | 90 | 75 | 75 |
| Creations that waited for a freed slot | 242 | 239 | 148 | 150 |

It also passes at 1,024 and 2,048 slots, with `--exercise-uploads`, and at 16,384 slots, where the check stops at 2,048 textures of its own and the table is never full. With `--exercise-swapchain` the glass backdrop of a resized window sometimes waits for a slot (77 frames in 8 s drew the flat tint instead), which that exercise would count as a failure and does not when the table is being filled on purpose.

### The pipeline cache

The cache file is named after the GPU, the driver's cache id and a hash of the compiled SPIR-V, each pipeline's state and the interface the pipelines share with the renderer (push-constant range, the texture table's bindings, size and samplers, the frame data's binding). A build with other shaders starts cold in a file of its own and the same build always finds its own: six launches with a new hash each built the six pipelines in 35 to 46 ms, every launch of a build that had run before in 0.8 to 0.9 ms. The current file and the three other ones used last are kept (an A/B of two builds stays warm on both sides); older ones are deleted. `--bench-out` records `cold`, `warm` or `off`.

## Against the targets

| Target | At `balanced` | At `performance` |
|---|---|---|
| Window visible within 300 ms | Window 121–145 ms, first frame 238–282 ms | Window 82–84 ms, first frame 184–189 ms |
| Main menu ready within 1.5 s warm | 0.81–0.85 s, the fade-in included | 0.75 s |
| Menu pacing: P99.9 at most 1.5 × median | 1.6 to 1.8 in quiet runs, about 8.5 in disturbed ones: not met | 1.42 to 1.50: met |
| GPU memory at most 300 MB in a world | 39 MiB before any world data, 56 MiB with a menu open | the same |
| RSS at most 600 MB in a world | 208–215 MiB before any world data, most of it shared libraries | the same |

## Not measured yet

- **The Intel iGPU** (Hybrid mode), where memory is shared and the DRM client statistics replace `nvidia-smi`.
- **The same runs with no capture running.** The desktop was streamed by Sunshine in every run here, also those at the `performance` profile, so the lows and the VSync pacing may still carry its cost.
- **A cold start** with the files out of the page cache.
- **Occlusion, moving between displays and a stalled compositor**, which need a person at the machine. They are also where the shorter acquire wait has yet to be measured, together with the CPU an idle, covered window takes with it.
- **A surface that really offers only sRGB formats.** `--pretend-srgb-surface` hides the others to show that it is refused; presenting to one through UNORM views of a mutable-format swapchain is not built.
- **A GPU whose ordinary descriptor limits are below the table.** Both GPUs here allow a million sampled images in ordinary sets, so the update-after-bind path was only run forced.

## Reproducing

```sh
cargo xtask bench menu                          # launch, memory and VSync pacing, about 2 minutes
cargo xtask bench ab --a work --b work          # A/A: the noise of the uncapped menu
cargo xtask bench ab --a work --b work --b-args '--frames-ahead 1'
cargo xtask bench menu --gpu intel              # the iGPU of a hybrid laptop
```

Every run writes its summary as JSON under `target/bench-menu` or `target/bench-ab`. For a line per frame, run the client itself:

```sh
ruthenium --vsync --quit-after 15 --frame-log frames.csv
ruthenium --validation --exercise-swapchain --quit-after 8 --frame-log resize.csv
ruthenium --validation --exercise-swapchain --exercise-uploads --swapchain-maintenance none --quit-after 8
ruthenium --screenshot first.png --screenshot-resize 1100x620 --screenshot-after 0.5 --screen servers
ruthenium --screenshot steady.png --window-size 1100x620 --screenshot-after 0.5 --screen servers
```

`--exercise-swapchain` fails the run if a replaced swapchain is not destroyed while the client runs, if a frame draws glass without its backdrop, or on any validation message.

The renderer's checks:

```sh
ruthenium --validation --exercise-textures --texture-slots 256 --texture-table update-after-bind --quit-after 6
cargo xtask bench ab --a work --b work --b-args '--texture-slots 16384 --texture-table update-after-bind'
```

Each line has the frame's number, when its input was sampled, the time in each part of the main thread's frame (limiter, governor, events, game, GUI, recording, and the acquire, submit and present calls), how the acquire ended, and the frame's GPU time.
