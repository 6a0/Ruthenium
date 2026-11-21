# Privacy

Ruthenium is pre-alpha. This document describes how the client is designed to work. It will be updated before the first public release and whenever data handling changes.

## What Ruthenium does not do

- It does not collect telemetry or analytics.
- It has no Ruthenium accounts and runs no Ruthenium servers that receive player data.
- It does not sell or share data.

## What leaves your device, and why

| Destination | When | What |
|---|---|---|
| Microsoft identity (login.microsoftonline.com), Xbox Live, XSTS | Signing in | The standard OAuth sign-in flow for your Microsoft account |
| Mojang / Minecraft services (api.minecraftservices.com, sessionserver.mojang.com, and others listed in Mojang's discovery document) | Signing in, joining servers, loading skins, the server blocklist | The same requests the vanilla client makes |
| Minecraft servers you choose to join | While connected | Normal game traffic, including the client brand `ruthenium` |
| Skin and cape texture hosts allowed by Mojang | Rendering players | Texture downloads |

## What is stored on your device

- Microsoft and Minecraft tokens, in your operating system's credential store (Windows DPAPI, or Secret Service on Linux).
- Settings, the server list and caches built from your installed game, in Ruthenium's data folder.

Delete Ruthenium's data folder and its credential-store entries to remove everything.

## Crash reports (future)

If crash reporting is added, it will be **off by default**. Ruthenium will show you exactly what would be sent before anything is sent.

## Contact

Questions about privacy: [sixazero@outlook.com](mailto:sixazero@outlook.com).
