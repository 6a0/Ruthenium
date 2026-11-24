# Ruthenium

> **NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.**

Ruthenium is an open-source client for **Minecraft: Java Edition** (currently targeting 26.3), written in Rust with a Vulkan renderer. It is aimed at PvP players and built around three goals:

- **Smooth frames.** Consistent frame pacing and strong 1% / 0.1% lows on weak and strong hardware alike.
- **Vanilla behaviour.** Ticks, physics and outgoing packets match the vanilla client exactly, checked against recorded traces, so servers and anticheats see a normal client.
- **Fair-play features only.** Ruthenium has no cheats and no features that give information or control vanilla doesn't, such as minimaps, radar, reach displays or freelook. Optional HUD features follow each server's rules.

## Status

**Pre-alpha. Nothing is playable yet.** Stage 0 is done: workspace, CI, game-file import, code generation, golden traces of the vanilla client and its baseline measurements ([docs/baseline.md](docs/baseline.md)). Stage 1, the engine skeleton, has started: `cargo run -p ruthenium` opens an empty Vulkan window with a paced frame loop.

## How Ruthenium treats Mojang's content and accounts

- **No Mojang files are shipped or downloaded.** Ruthenium uses the copy of the game that the player has already installed through a launcher (official, Prism, MultiMC, Modrinth App, ATLauncher, GDLauncher) and checks it by hash. This repository contains no Minecraft textures, sounds, models, shaders, language files or decompiled code. CI enforces this with `cargo xtask check-content`.
- **A genuine Minecraft account is required.** Players sign in with Microsoft through Ruthenium's own registered Azure application. Release builds have no offline or cracked mode.
- **Accounts stay on the player's device.** Tokens are kept in the operating system's credential store. There is no Ruthenium account and no Ruthenium server. See [PRIVACY.md](PRIVACY.md).
- **Mojang's server blocklist is respected,** as it is in vanilla.

The project follows the [Minecraft EULA](https://www.minecraft.net/eula) and [Usage Guidelines](https://www.minecraft.net/usage-guidelines).

## Building

```sh
cargo build
cargo test --workspace
cargo xtask check-content   # make sure no Mojang content is in the tree
```

The window and input use SDL3, and rendering uses Vulkan (on macOS, Vulkan runs on Metal through MoltenVK). On Windows and macOS, SDL3 is built from source and linked statically, so you need CMake. On Linux, either install the SDL3 development package (Fedora `SDL3-devel`, Debian/Ubuntu `libsdl3-dev`), or build SDL3 from source with `--features ruthenium/sdl-static` (needs CMake and the X11 and Wayland development headers).

```sh
cargo run -p ruthenium -- --stats             # open the window; print frame-time statistics every second
cargo run -p ruthenium -- --no-vsync --stats  # uncapped; --fps-cap 240 adds the frame limiter
cargo run -p ruthenium -- --help              # F3 in the window shows the frame-stats overlay
cargo xtask bench ab                          # compare HEAD with the working tree (release, A B B A)
cargo xtask fixtures                          # the renderer's own checks, one short run each (needs a display)
```

On macOS, `cargo xtask bundle` builds `target/bundle/Ruthenium.app` with a pinned MoltenVK inside (downloaded once into the git-ignored `cache/`, checked by SHA-256), so the app needs no Vulkan SDK.

Shaders are GLSL in `crates/ruthenium-shaders/glsl`, compiled to SPIR-V that is committed, so building needs no shader compiler. After editing a shader, run `cargo xtask shaders`. It uses the `glslc` of a pinned Vulkan SDK release: on Linux x86-64 it is downloaded once into the git-ignored `cache/`; elsewhere, install that SDK and pass `--glslc <path>`. CI fails if the committed SPIR-V differs from what that compiler produces.

Debug builds turn on the Vulkan validation layer if it is installed (Fedora `vulkan-validation-layers`, Debian/Ubuntu `vulkan-validationlayers`). Set `RUTHENIUM_VALIDATION=1` to make it required.

To profile, build with `--features ruthenium/tracy` and connect the [Tracy](https://github.com/wolfpld/tracy) profiler, version **0.14.0** (it must match the Tracy client Ruthenium links). It shows CPU zones on every thread and GPU zones from timestamp queries on the graphics queue. `TRACY_NO_EXIT=1` keeps the process alive at exit until the profiler has all its data.

The per-version tables in `crates/ruthenium-data` (packet ids, registries, block states, entity-data serializer ids) are generated and committed, so building needs no game files. To regenerate them, run `cargo xtask codegen`. It needs JDK 25 and the official client jar (see below), and runs Mojang's data generator from that jar.

## Game files

Install and play Minecraft 26.3 once in your usual launcher, then run:

```sh
cargo run -p ruthenium -- locate    # list the launcher folders searched
cargo run -p ruthenium -- prepare   # import the game files and run Mojang's data generator
```

To also run Ruthenium's exporters (original Java code that reads data such as entity sizes from your installed game), build them first with `cargo xtask exporters`. This needs JDK 25 and a client jar to compile against: either the local reference copy, or the official jar fetched by `cargo xtask fetch-compile-deps` (compile-time only, kept in the git-ignored `cache/`).

`prepare` accepts a client jar only if its SHA-1 matches the official 26.3 release. It checks the asset index and every library and asset file the same way. Verified files are hard-linked (or copied) into Ruthenium's data folder. Pass `--from <folder>` for a launcher folder that isn't found automatically.

After `prepare`, `cargo run -p ruthenium -- assets` builds the game's sprite atlases from those files, read in place (add `--pack <zip or folder>` for resource packs, `--dump <folder>` to write them out as PNGs). The atlas tests in `crates/ruthenium-assets/tests/vanilla.rs` run against your import when `RUTHENIUM_GAME_DIR` points at it.

## Measuring vanilla

Two development tools run the vanilla 26.3 client from your own install (offline developer mode, in singleplayer or on a local server the tool starts itself; your account is never used):

```sh
cargo xtask trace          # record golden traces of scripted scenarios (docs/golden-traces.md)
cargo xtask bench setup    # create the benchmark world, then:
cargo xtask bench run      # frame times, memory and launch time on OpenGL and Vulkan (docs/baseline.md)
cargo xtask bench run --gpu intel   # on a hybrid laptop: pin the game to one GPU (auto, nvidia, intel)
cargo xtask bench dedicated --accept-eula   # the world scenes on a local dedicated server (needs the 26.3 server.jar; docs/baseline.md)
```

They load Fabric Loader and Ruthenium's own small development mods (`tools/java/`). They need JDK 25 to compile the mods and a desktop session, since a game window opens.

## Publisher and contact

Ruthenium is published by **6a0**. Contact: **[sixazero@outlook.com](mailto:sixazero@outlook.com)**. Report security issues as described in [SECURITY.md](SECURITY.md).

## Licence

The application is licensed under [GPL-3.0-or-later](LICENSE). Standalone libraries (for example `ruthenium-javamath`) are licensed MIT OR Apache-2.0, as stated in each crate. Contributions require a DCO sign-off; see [CONTRIBUTING.md](CONTRIBUTING.md).

The menus use [Nunito](https://github.com/googlefonts/nunito) by The Nunito Project Authors, bundled under the SIL Open Font License 1.1 (see [crates/ruthenium-ui/fonts](crates/ruthenium-ui/fonts)).

Minecraft is a trademark of Mojang Synergies AB. Ruthenium is not affiliated with or endorsed by Mojang or Microsoft.
