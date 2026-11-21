# Vanilla 26.3 baseline

These are the numbers later stages are judged against (plan §3): vanilla 26.3 on the same machine, in the same scenes, measured the same way. They were recorded on 2026-11-21 with `cargo xtask bench run` and summarised with `cargo xtask bench report`.

## Machine

| | |
|---|---|
| Device | Lenovo laptop: Intel Core i5-13500HX (20 threads), 64 GB RAM, NVIDIA GeForce RTX 4060 Laptop GPU (8 GB) |
| Software | Fedora Linux 45 (kernel 7.3.4), KDE Plasma on Wayland; NVIDIA driver 620.70.06 |
| Power | On AC. Firmware platform profile `performance`, CPU governor `performance`, Lenovo power mode "Balanced" |
| Display | 2560 × 1600 at 240 Hz; game window 1920 × 1080, windowed, with SDL's native Wayland driver |
| Java | Mojang's runtime from Prism Launcher: OpenJDK 25.0.1+8 (Microsoft build) |

The laptop has a GPU switch. The NVIDIA results were measured in its Discrete GPU mode; the [Intel iGPU results](#second-device-the-laptops-intel-igpu) in Hybrid mode, the only mode in which the iGPU is visible. The rest of the plan's device lab (other Intel and AMD iGPUs, Arc, Snapdragon X, a Linux ARM board, Apple Silicon) is still to come.

## Method

- **The game.** The player's own Prism install, launched by `cargo xtask bench` with the official launcher's JVM arguments (`-Xms2G -Xmx4G -XX:+UseCompactObjectHeaders -XX:+AlwaysPreTouch -XX:+UseStringDeduplication -XX:+UseZGC`, from the version JSON).
- **What runs on top of vanilla.** Fabric Loader and `tools/java/bench`: one hook at the end of each frame that drives the scene and stores the frame interval in a preallocated array. Fabric API is not loaded, so vanilla's renderer is untouched.
- **Settings.** Vanilla's default graphics preset (Fancy), except render distance 12 instead of Fancy's 16, to match the plan's gates. Simulation distance 12. VSync off, frame limit unlimited, sound muted, no pause or throttling when the window loses focus.
- **World.** Seed 20261121, default world type, created by `cargo xtask bench setup`. Spawn is at the edge of a jungle by water. Time is fixed at noon, weather is clear, and mob spawning is off. Every world run starts from a fresh copy of the same template, loaded with `--quickPlaySingleplayer`.
- **Scenes.** Each world scene waits for the world to load and all queued chunk sections to be built, then warms up for 10 s and measures for 60 s.
  - `menu`: the title screen. Vanilla caps menus without a world at 60 FPS, so this measures pacing at 60 FPS, not throughput.
  - `idle`: standing at spawn, looking north.
  - `spin`: turning on the spot at 36° per second, which exercises culling and the visible-section set.
  - `fly`: creative sprint-flight south at about 21 blocks per second, 40 blocks above spawn, over terrain that setup had already generated. This exercises chunk loading and section meshing.
- **Frame time** is the interval between the ends of consecutive frames, after submit, present and the frame limiter. **1% / 0.1% low** = 1000 / P99 and 1000 / P99.9 of frame time in milliseconds (nearest rank, the PresentMon / CapFrameX convention). Present-to-display timing is not measured.
- **Memory.** Resident memory comes from `/proc/<pid>/status` (VmRSS), sampled every 0.5 s during the measured 60 s. GPU memory comes from `nvidia-smi`'s per-process table on NVIDIA, and from the kernel's per-client DRM statistics (`drm-resident-*` in `/proc/<pid>/fdinfo`) on Intel.
- **Temperatures and clocks.** CPU package temperature, mean CPU clock, GPU temperature, GPU clock and NVIDIA throttle reasons were sampled the same way.
- **Repetitions.** 3 for every scene and backend; tables show the median.

## Results

Medians of 3 runs, 60 s each.

| Backend | Scene | Avg FPS | Median ms | 1% low FPS | 0.1% low FPS | Max ms | RSS MiB (mean / max) | VRAM MiB (mean / max) |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| OpenGL | menu | 60 | 16.67 | 60 | 60 | 16.7 | 2862 / 2867 | 156 / 156 |
| OpenGL | idle | 1064 | 0.87 | 660 | 414 | 5.0 | 4652 / 4942 | 599 / 599 |
| OpenGL | spin | 1100 | 0.85 | 671 | 436 | 4.5 | 4668 / 4955 | 598 / 598 |
| OpenGL | fly | 1123 | 0.79 | 578 | 371 | 8.6 | 4884 / 5108 | 608 / 727 |
| Vulkan | menu | 60 | 16.67 | 60 | 60 | 16.8 | 2940 / 2961 | 301 / 301 |
| Vulkan | idle | 1217 | 0.77 | 801 | 606 | 5.0 | 4726 / 4977 | 653 / 653 |
| Vulkan | spin | 1294 | 0.72 | 829 | 640 | 4.1 | 4805 / 5026 | 653 / 653 |
| Vulkan | fly | 1384 | 0.64 | 714 | 392 | 9.2 | 4959 / 5256 | 664 / 781 |

**Launch.** Medians of 6 runs:

| Milestone | Time after JVM start |
|---|---:|
| First frame | 4.53 s |
| Main menu ready (title screen up, resource loading finished) | 7.33 s |
| Quick-play world ready (median of 18 runs) | 8.43 s |

These times include Fabric Loader's start-up (mod discovery and Mixin setup for one mod). Pure vanilla starts slightly sooner. Measured cold the first time after the world template was created, but warm in practice: the files were in the page cache.

**Run to run.** The three repetitions agree within about 1% on average FPS. The exception is one OpenGL `spin` run with a single 50 ms frame, which pulled its 0.1% low to 259. That run is kept as measured; the medians above are unaffected.

**Thermals.** The GPU held about 2490 MHz with no thermal or hardware slowdown flagged in any sample. The CPU package peaked at 92–100 °C in the world scenes and was at 95 °C or above in up to 7% of the samples in `fly`. Its all-core mean clock was about 3.0 GHz. Repeat on a cooler machine before reading small differences into these numbers.

## Second device: the laptop's Intel iGPU

The same laptop with its GPU switch in Hybrid mode, the game pinned to the iGPU with `cargo xtask bench run --gpu intel`. Measured on 2026-11-21.

| | |
|---|---|
| GPU | Intel UHD Graphics (ADL-S GT0.5): the 16-execution-unit Xe iGPU of the i5-13500HX, max 1500 MHz, sharing system memory |
| Driver | i915, Mesa 26.2.7 (OpenGL 4.6, Vulkan 1.4.354) |
| Display | In Hybrid mode the iGPU also drives the laptop panel and composites the desktop |

Everything else (machine, power settings, Java, world, scenes, settings) is as above, with two differences in method:

- **GPU samples.** GPU memory comes from the DRM statistics. With no dedicated VRAM, it is the game's GPU buffers in shared system memory, resident in RAM; some of it can also count in RSS. The GPU clock and throttle reasons come from i915's sysfs files. The iGPU has no temperature sensor of its own; it shares the CPU package's.
- **Input.** The bench mod now drops all keyboard and mouse input and doesn't capture the cursor, so the desktop can't disturb a run. This was added after the NVIDIA runs; it doesn't change what is rendered.

Vulkan and the title screen: medians of 3 runs. OpenGL world scenes: medians of 5 runs (see *Run to run* below). 60 s each.

| Backend | Scene | Avg FPS | Median ms | 1% low FPS | 0.1% low FPS | Max ms | RSS MiB (mean / max) | GPU memory MiB (mean / max) |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| OpenGL | menu | 60 | 16.67 | 60 | 60 | 16.9 | 2764 / 2780 | 184 / 194 |
| OpenGL | idle | 82 | 11.54 | 48 | 42 | 28.5 | 4000 / 4003 | 615 / 625 |
| OpenGL | spin | 80 | 11.86 | 47 | 41 | 28.3 | 4022 / 4239 | 616 / 625 |
| OpenGL | fly | 75 | 12.18 | 44 | 37 | 29.5 | 4573 / 5038 | 630 / 759 |
| Vulkan | menu | 60 | 16.67 | 60 | 60 | 16.8 | 2715 / 2723 | 691 / 692 |
| Vulkan | idle | 73 | 14.54 | 48 | 43 | 25.8 | 3893 / 3894 | 874 / 874 |
| Vulkan | spin | 72 | 14.53 | 46 | 43 | 24.7 | 3327 / 3350 | 874 / 874 |
| Vulkan | fly | 66 | 15.41 | 39 | 35 | 29.4 | 4489 / 4992 | 896 / 1132 |

**Launch.** Medians of 6 runs: first frame 4.31 s, main menu ready 7.10 s, quick-play world ready 8.36 s (median of 18 runs). These are within a few percent of the NVIDIA numbers: launch is CPU- and disk-bound here, and this laptop's CPU, RAM and SSD are far stronger than a typical iGPU laptop's. They say nothing about the plan's "≤ 3 s on the iGPU laptop" gate, which needs a real thin-and-light machine.

**Where the time goes.** The GPU is the bottleneck: it held its 1500 MHz maximum through every world scene, with no thermal or hardware slowdown flagged. The only throttle reason seen was the occasional PL4 power limit, in under 15% of samples. The CPU package peaked at 84–89 °C.

**OpenGL is faster than Vulkan here**, by 11–14% in average FPS, with similar lows. On the RTX 4060 it was the other way round. Vanilla's Vulkan backend also uses about 260 MiB more GPU memory on this device.

**Resident memory is about 0.6–0.8 GiB lower than on the NVIDIA GPU in the same scenes** (about 3.3–4.6 against 4.7–5.0 GiB): NVIDIA's driver allocates in the game process, Mesa's mostly doesn't. Compare RSS within one GPU only.

**Run to run.** Vulkan's three runs agree within 3%. The first OpenGL round didn't: its second set of world runs was up to 26% slower and its first set 7–8% faster than the rest, each run steady for its whole minute. Another Minecraft client was probably running on the same desktop (it was during a later test), competing for the same small GPU. That round's OpenGL world runs are discarded. The OpenGL world scenes were re-measured 5 times with nothing else running, and agree within about 1%.

## What this says against the plan's targets (§3)

| Target | Vanilla 26.3 here | Ruthenium target |
|---|---|---|
| Process RSS at RD 12 | about 4.7–5.0 GiB in a world, 2.9 GiB at the title screen | ≤ 600 MB |
| VRAM at RD 12 | about 600–660 MiB (OpenGL / Vulkan) | ≤ 300 MB |
| Main menu (warm) | 7.3 s (first frame 4.5 s) | ≤ 1.5 s warm |
| Avg FPS | 1100–1400 | ≥ 1.5× vanilla Vulkan: about 1830 (idle) to 2080 (fly) |
| 1% / 0.1% lows | Vulkan: 801 / 606 idle, 714 / 392 fly | ≥ 2× vanilla |

The plan estimated vanilla's RSS at 1.5–3 GB. Measured with the official launcher's arguments, it is higher. `-Xms2G` with `-XX:+AlwaysPreTouch` commits 2 GiB of heap at start-up, and ZGC grows the committed heap to nearly the 4 GiB maximum once a world is loaded (about 3.9 GiB committed at the end of every world run). Most of vanilla's footprint is therefore its heap configuration, not its live data.

Vanilla's Vulkan backend is already 15–25% faster than its OpenGL backend here, with better lows. Ruthenium is compared against Vulkan, the faster of the two, as the plan says.

## Not measured yet

- **Input-to-photon latency.** It needs a hardware rig (PresentMon on Windows, NVIDIA Reflex Latency Analyzer, or a photodiode).
- **The standard PvP benchmark** (spawn + sprint + bridge + 8-player fight replay). It needs a server-side replay; the golden-trace and packet-replay tools come first.
- **Other devices** in the lab, a pure-vanilla launch time without Fabric Loader, and NVIDIA in Hybrid mode (where its frames are copied to the iGPU-driven panel).

## Reproducing

```sh
cargo xtask bench setup          # once: creates cache/bench/world
cargo xtask bench run            # 3 × {opengl, vulkan} × {menu, idle, spin, fly}, about 35 minutes
cargo xtask bench run --gpu intel   # the same on the iGPU of a hybrid laptop (auto, nvidia, intel)
cargo xtask bench report         # Markdown tables for the latest results folder
```

Close other games first, plug in, and check the power profile. Each run writes its frame intervals and samples to `cache/bench/results/<run>/<backend>-<scene>-<rep>.json`, with `machine.json` alongside. `--screenshot` saves one screenshot per scene into `cache/bench/game/screenshots`, to confirm what was measured.
