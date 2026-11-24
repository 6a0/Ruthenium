# Vanilla 26.3 baseline

These are the numbers later stages are judged against (plan §3): vanilla 26.3 on the same machine, in the same scenes, measured the same way. They were recorded on 2026-11-21 with `cargo xtask bench run` and summarised with `cargo xtask bench report`.

## Machine

| | |
|---|---|
| Device | Lenovo laptop: Intel Core i5-13500HX (20 threads), 64 GB RAM, NVIDIA GeForce RTX 4060 Laptop GPU (8 GB) |
| Software | Fedora Linux 45 (kernel 7.3.4), KDE Plasma on Wayland; NVIDIA driver 620.70.06 |
| Power | On AC. Firmware platform profile `performance` (set by the desktop's power mode), CPU governor `performance` |
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
  - `idle`: standing at spawn, looking south (yaw 0, as the world template saved the player).
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

## Dedicated-server runs: method

Ruthenium only joins servers, so its world renderer will be measured on a dedicated server. Comparing that with the numbers above would mix two protocols, so vanilla gets a second baseline, measured the same way Ruthenium will be: `cargo xtask bench dedicated`. The NVIDIA results are [below](#dedicated-server-runs-results); the Intel iGPU runs wait for the laptop's Hybrid mode (in Discrete mode, which it is in now, the iGPU is not visible). The tables above stay the published integrated-server numbers.

What is the same: the game and how it is launched, the options (Fancy, render and simulation distance 12, VSync off), the world template and its seed, noon, clear weather, no mob spawning, a 10 s warm-up and a 60 s capture, 3 repetitions, frame time as the interval between frame ends, and the memory, clock and temperature samples.

What differs, and why:

- **The server.** A vanilla 26.3 dedicated server (a `server.jar` supplied locally and checked by SHA-1, never downloaded by the task) runs as its own process on the same machine, with Mojang's Java runtime and `-Xms1G -Xmx2G`, in offline mode, bound to 127.0.0.1 on a free port, white-list off. The client joins with `--quickPlayMultiplayer`. Packets cross loopback TCP with the server's default compression instead of the in-memory connection of an integrated server, and the server no longer shares the client's heap and garbage collector. It still shares the CPU: its process's memory and CPU time are sampled, and its own tick-time query (`tick query`: average and percentiles of the last 100 ticks) is recorded every 5 s.
- **The world copy.** Every run gets a fresh copy of the template in a folder of its own under `cache/bench/dedicated`, identified by a SHA-256 over its files. The server is stopped after the run and the copy deleted; if the task fails, times out or is killed, the server is killed with it.
- **The controller.** The scene is set up from outside, over RCON on loopback: the same game rules, time and weather as the integrated scenes fix, the world spawn at the scene's spawn, and, once the player is on the server, the player on a team with collisions off, teleported to the route's start and put in creative mode (through spectator mode on the `fly` route, which leaves it flying). The team keeps wandering mobs from pushing the player off its route; the mobs themselves stay, as in the integrated scenes. One rule is added: random ticks are off (`random_tick_speed 0`), so nothing grows or decays while a route runs. With them on, one vanilla `walk` run of the first three met a vine that had grown into its path and climbed it, where the other two and Ruthenium walked through.
- **Routes keyed by client tick.** One route file per run is read by the vanilla bench mod and by Ruthenium alike. Tick 0 is the first tick after the player has stood at the start for a second with every chunk the server sends at that distance loaded (637 at distance 12) and, in vanilla, every section built for 3 s. The warm-up is 200 ticks and the capture the next 1200, so the measured window is 60 s of ticks, not 60 s of wall clock; a client that falls behind its ticks measures longer.
  - `idle` and `fly` are the integrated scenes. `fly` starts exactly 40 blocks above the spawn; the integrated scene had dropped 0.19 blocks before flight came on.
  - `spin` turns 1.8° before every tick (36° a second, as before). The integrated scene turned every frame; here the game interpolates the camera between ticks.
  - `walk` is new: forward with jump held and 0.6° a tick, a circle of about 21 blocks' radius where the ground allows, so the route stays near the spawn however long it runs. It has no integrated counterpart.
- **The screen.** The toast a server without signed chat brings up and the chat lines of the join are cleared when the route starts.
- **Checks.** Each client records its position and rotation after every tick and the loaded chunks at tick 0, every 100 ticks and at rest after the route. Two checksums come out of that: one over the positions, one over the chunks at the start and at rest. The task fails if a run's turns, marks or loaded chunks are not what the route says, if the drawable is not 1920 × 1080 pixels (a scaled desktop gives the window more), or if two runs of a scene differ. Ruthenium plays the same routes headless through its development join path (`ruthenium connect --route`) and has to come out with the same checksums as vanilla.

**Label every ratio.** A dedicated-server row and an integrated-server row differ in the server's place, the connection, how the camera is driven and what the capture counts. `bench report` prints the two next to each other with the server named in the first column. A ratio of a dedicated-server client to an integrated-server row has to say that it crosses the two. Ruthenium's world-renderer runs are to be paired with the dedicated rows of the same device; the integrated rows above stay the published numbers next to them.

## Dedicated-server runs: results

Recorded on 2026-11-24 with `cargo xtask bench dedicated --accept-eula --width 960 --height 540` on the RTX 4060 Laptop GPU (Discrete mode, driver 620.70.06), firmware profile `performance`, on AC. The desktop was at scale 2, so the window was 960 × 540 points, which is the 1920 × 1080 pixels of the runs above (the task checks the drawable). Variable refresh was off, and the desktop was being streamed by Sunshine (one NVENC session) throughout, as it usually is on this machine. Medians of 3 runs of 1200 ticks (60 s) each:

| Backend | Scene | Avg FPS | Median ms | 1% low FPS | 0.1% low FPS | Max ms | RSS MiB (mean / max) | VRAM MiB (mean / max) | CPU / GPU °C max |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Vulkan | idle | 1456 | 0.65 | 896 | 638 | 3.1 | 4713 / 4868 | 653 / 653 | 94 / 79 |
| Vulkan | spin | 1532 | 0.61 | 903 | 633 | 3.9 | 4792 / 4892 | 653 / 653 | 96 / 82 |
| Vulkan | walk | 1410 | 0.66 | 807 | 512 | 4.7 | 4778 / 4905 | 654 / 655 | 98 / 82 |
| Vulkan | fly | 1569 | 0.57 | 802 | 404 | 6.6 | 4751 / 4994 | 664 / 783 | 98 / 84 |

**Run to run.** Average FPS of the three repetitions agrees within 1% in every scene (idle 1451 to 1460, spin 1529 to 1533, walk 1409 to 1411, fly 1564 to 1574); the 0.1% lows spread more, most in `fly` (378 to 428).

**The same scene every time.** In each scene the three vanilla runs and Ruthenium's headless run of the same route recorded the same position on every one of the 1440 ticks and the same 637 chunks at the start and at rest:

| Scene | Route | Positions | Chunks at the start and at rest |
|---|---|---|---|
| idle | `58bb70b9655f6c8a` | `8dd4897dc82ac8e5` | `3ece8aa2b6b39475` |
| spin | `68e62776d02d19e7` | `4947001eb2c13686` | `3ece8aa2b6b39475` |
| walk | `56b6a6d9b0d66667` | `692b320eba9f2520` | `acfad72fca46ca6d` |
| fly | `4eccd90811443043` | `ba30c8310240c0a6` | `b4da7fbbf26ad726` |

The world copy is `fcc0c49f13fb9049…` (SHA-256 over the template's files) in every run. On the move, at most 27 chunks (`walk`) and 24 (`fly`) of what the server would send at the player's position had yet to arrive at a mark.

**The server.** It ticked in 1.0 to 1.4 ms on average (its own `tick query`, worst P99 of a 100-tick window 24 ms) and used 6 to 11% of one core in `idle`, `spin` and `walk` and 28 to 29% in `fly`, where it streams chunks. The scene was ready (every chunk loaded, every section built for 3 s) 13.2 to 13.4 s after the JVM started.

**Against the integrated-server runs above** (another protocol: the server inside the client, scenes driven by time, recorded three days earlier): the dedicated-server client averages 18 to 20% more frames in `idle` and `spin` and 13% more in `fly`, with 1% lows 9 to 12% higher. That is the server's work leaving the client's process, not a faster renderer, which is why Ruthenium's world runs are to be paired with these rows and not with those.

**A first recording was discarded** for one reason: with random ticks on, the first of its three `walk` runs diverged from the other two (and from Ruthenium's) at tick 625, where a vine had grown into the route and the player, with jump held, climbed it. The controller now switches random ticks off (see the method). The average FPS of that recording was within 1% of the one above in every scene.

**Thermals.** As in the integrated runs, the CPU package reached 94 to 98 °C in every scene.

## Not measured yet

- **Input-to-photon latency.** It needs a hardware rig (PresentMon on Windows, NVIDIA Reflex Latency Analyzer, or a photodiode).
- **The standard PvP benchmark** (spawn + sprint + bridge + 8-player fight replay). It needs a server-side replay; the golden-trace and packet-replay tools come first.
- **The dedicated-server baseline on the Intel iGPU** (method above): it needs the laptop's Hybrid mode.
- **Other devices** in the lab, a pure-vanilla launch time without Fabric Loader, and NVIDIA in Hybrid mode (where its frames are copied to the iGPU-driven panel).

## Reproducing

```sh
cargo xtask bench setup          # once: creates cache/bench/world
cargo xtask bench run            # 3 × {opengl, vulkan} × {menu, idle, spin, fly}, about 35 minutes
cargo xtask bench run --gpu intel   # the same on the iGPU of a hybrid laptop (auto, nvidia, intel)
cargo xtask bench report         # Markdown tables for the latest results folder
cargo xtask bench dedicated --accept-eula   # vulkan × {idle, spin, walk, fly} × 3 on a dedicated server, then Ruthenium's headless check of each route; about 25 minutes
cargo xtask bench dedicated --accept-eula --client ruthenium --baseline <folder>   # Ruthenium in its window on the same routes: see world-baseline.md
cargo xtask bench report <folder> --compare <folder>   # a dedicated-server folder next to an integrated-server one
```

`bench dedicated` needs the 26.3 `server.jar` (`--server-jar`, by default the local reference copy) and `--accept-eula`, which says you accept [Mojang's EULA](https://aka.ms/MinecraftEULA) for the server it starts; it refuses to run without it. `--gpu`, `--backend`, `--reps`, `--warmup` and `--duration` work as for `bench run`. Its runs go to the same results folders, with the route, the client's record of it and the server's side (world copy, controller commands, tick times, samples) in each file; trial folders and server logs are under `cache/bench/dedicated`.

Close other games first, plug in, and check the power profile. Each run writes its frame intervals and samples to `cache/bench/results/<run>/<backend>-<scene>-<rep>.json`, with `machine.json` alongside. `--screenshot` saves one screenshot per scene into `cache/bench/game/screenshots`, to confirm what was measured.
