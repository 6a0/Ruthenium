# Ruthenium

> **NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.**

Ruthenium is an open-source client for **Minecraft: Java Edition** (currently targeting 26.3), written in Rust with a Vulkan renderer. It is aimed at PvP players and built around three goals:

- **Smooth frames.** Consistent frame pacing and strong 1% / 0.1% lows on weak and strong hardware alike.
- **Vanilla behaviour.** Ticks, physics and outgoing packets match the vanilla client exactly, checked against recorded traces, so servers and anticheats see a normal client.
- **Fair-play features only.** Ruthenium has no cheats and no features that give information or control vanilla doesn't, such as minimaps, radar, reach displays or freelook. Optional HUD features follow each server's rules.

## Status

**Pre-alpha. Nothing is playable yet.** Work is at Stage 0: workspace, CI, game-file import and code generation.

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

## Game files

Install and play Minecraft 26.3 once in your usual launcher, then run:

```sh
cargo run -p ruthenium -- locate    # list the launcher folders searched
cargo run -p ruthenium -- prepare   # import the game files and run Mojang's data generator
```

`prepare` accepts a client jar only if its SHA-1 matches the official 26.3 release. It checks the asset index and every library and asset file the same way. Verified files are hard-linked (or copied) into Ruthenium's data folder. Pass `--from <folder>` for a launcher folder that isn't found automatically.

## Publisher and contact

Ruthenium is published by **6a0**. Contact: **[sixazero@outlook.com](mailto:sixazero@outlook.com)**. Report security issues as described in [SECURITY.md](SECURITY.md).

## Licence

The application is licensed under [GPL-3.0-or-later](LICENSE). Standalone libraries (for example `ruthenium-javamath`) are licensed MIT OR Apache-2.0, as stated in each crate. Contributions require a DCO sign-off; see [CONTRIBUTING.md](CONTRIBUTING.md).

Minecraft is a trademark of Mojang Synergies AB. Ruthenium is not affiliated with or endorsed by Mojang or Microsoft.
