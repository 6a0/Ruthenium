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
| Hosts a server names for its resource packs | When you join a server that offers resource packs and you accept them (the prompt's Yes or Proceed, or the server list's Server Resource Packs set to Enabled) | A download of each pack (HTTP or HTTPS only), with the identifying headers the vanilla client sends: your Minecraft username and UUID (`X-Minecraft-Username`, `X-Minecraft-UUID`), the game version and resource pack format (`X-Minecraft-Version`, `X-Minecraft-Version-ID`, `X-Minecraft-Pack-Format`) and the user agent `Minecraft Java/26.3`. Like any web server, the host also sees your IP address. Hosts on your local network are only contacted when the server you joined is on it too. Declining (No, or the setting Disabled) downloads nothing. |

## What is stored on your device

- Microsoft and Minecraft tokens, in your operating system's credential store (Windows DPAPI, or Secret Service on Linux).
- Settings, the server list and caches built from your installed game, in Ruthenium's data folder.
- Resource packs downloaded from servers, in the `server-packs` folder of Ruthenium's cache folder (`~/.cache/ruthenium` on Linux, `~/Library/Caches/Ruthenium` on macOS, `%LOCALAPPDATA%\Ruthenium\cache` on Windows), one folder per pack. They are only ever read as resource packs: nothing in them is run. Delete the folder to clear them; a server's pack is downloaded again the next time you accept it.

## What Ruthenium reads on your device

To find your installed game, Ruthenium looks in your launchers' folders for the 26.3 client jar, its libraries, its asset index and asset files, and Java runtimes the launcher downloaded. It does not read launcher account files, instance worlds, screenshots or settings.

Delete Ruthenium's data folder and its credential-store entries to remove everything.

## Crash reports (future)

If crash reporting is added, it will be **off by default**. Ruthenium will show you exactly what would be sent before anything is sent.

## Contact

Questions about privacy: [sixazero@outlook.com](mailto:sixazero@outlook.com).
