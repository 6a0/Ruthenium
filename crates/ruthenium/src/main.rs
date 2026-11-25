//! Ruthenium: composition root.

#![forbid(unsafe_code)]
// The run summaries are large `json!` literals.
#![recursion_limit = "256"]

mod account;
mod assets_cmd;
mod auth_cmd;
mod client;
mod connect_cmd;
mod controls;
mod ingame;
mod input_script;
mod join;
mod menu;
mod prepare_cmd;
mod route;
mod servers;
mod settings;
mod skins;
mod telemetry;

use std::process::ExitCode;
use std::time::Instant;

/// Printed by `ruthenium version` and at start-up, and for the first-run and About screens
/// (Minecraft Usage Guidelines). Not on the title screen: no launcher puts it there.
pub const DISCLAIMER: &str =
    "NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.";

const USAGE: &str = "\
usage: ruthenium [command]

With no command, opens the game window. Options:
  --vsync | --no-vsync   VSync on or off (tearing, else mailbox) for this run
  --fps-cap <fps>        frame limiter (sleep, then spin, to exact deadlines) for
                         this run, until Max Frame Rate changes
  --gpu <name>           prefer a GPU whose name contains <name> (or RUTHENIUM_GPU)
  --validation | --no-validation
                         Vulkan validation layer (default: on in debug builds if
                         installed; RUTHENIUM_VALIDATION=1 requires it)
  --stats                print frame-time statistics every second
  --overlay              start with the frame-stats overlay shown (F3 toggles it)
  --bench-out <file>     write a JSON summary of the run at exit (cargo xtask bench);
                         runs on the default settings
  --frame-log <file>     write a CSV line per frame at exit: its number, when input
                         was sampled, where its main-thread time went, its acquire
                         and its GPU time
  --frames-ahead <0|1>   frames the GPU may still be working on when the next one
                         samples input (default: 0 without VSync, 1 with it)
  --empty-window         draw only the background, no menus (benchmarks)
  --quit-after <s>       close after <s> seconds (for benchmarks and tests)
  --acquire-wait <ms|pace>
                         how long a frame waits for a swapchain image before it goes
                         on without presenting: pace (default: until two display or
                         frame-limiter intervals after its input), or <ms> at most
  --swapchain-maintenance <auto|khr|ext|none>
                         use only this swapchain maintenance extension, or none
                         (default auto: KHR, else EXT, else none)
  --exercise-swapchain   resize, toggle VSync, replace the swapchain and capture
                         frames every few frames (development)
  --pretend-srgb-surface <start|recreate>
                         treat the window's surface as offering only sRGB formats,
                         from the start or from the second swapchain (development)
  --exercise-minimize    minimise the window and bring it back every 1.5 s
                         (development; Wayland cannot bring it back)
  --no-pipeline-cache    don't load or save the pipeline cache
  --no-transfer-queue    upload on the graphics queue even if there is a copy queue
  --exercise-uploads     check data pushed through the upload ring every frame
                         (development; a mismatch fails the run)
  --exercise-streaming   stream a texture larger than the upload ring in tiles, with
                         deferred, repeated and cancelled batches, and worker-written
                         blocks through pooled buffer ranges; read everything back
                         (development; a wrong byte or a broken rule fails the run)
  --texture-slots <n>    slots of the texture table to ask the GPU for (default 1024,
                         at least 256)
  --texture-table <auto|ordinary|update-after-bind>
                         make the texture table of this kind of descriptor set only
                         (default auto: ordinary, update-after-bind for a larger table)
  --exercise-textures    fill and reload the texture table and read every slot back
                         through the GUI pipeline (development; a wrong slot fails the
                         run)
  --adapter-report <file>
                         write what the GPU reported (limits, optional features, the
                         portability subset, vertex layouts) and what was chosen as JSON
  --exercise-colour      draw a known grey and a translucent swatch over it, off
                         screen and in the window's corner, and read the bytes back
                         (development; an sRGB decode or encode fails the run)
  --exercise-terrain     draw a made-up scene with the terrain pipelines, off screen,
                         and read every pixel back (development; a wrong one fails the
                         run)
  --screenshot <file>    write the window to a PNG, then quit (development); runs
                         on the default settings unless --data-dir is given
  --screenshot-resize <WxH>
                         with --screenshot: resize the window first, and capture the
                         first frame at the new size
  --data-dir <folder>    Ruthenium's data folder (accounts, servers, settings in
                         config/client.toml); default: the per-user one
  --join <address>       join this server at once (development)
  --mesh-workers <n>     threads that mesh the world (default: by the machine's size)
  --rejoin-after <s>     with --join: leave after this long and join again (development)
  --terrain-draws <direct|paged|mdi>
                         how the world's sections are drawn: one draw each, nearest
                         first (default); one draw each, a buffer page at a time; or
                         a page's draws in one call that reads them as commands
                         (multi-draw indirect, where the GPU has it)
  --terrain-culling <graph|frustum>
                         what of the world is meshed and drawn: what can be seen
                         from the camera through the sections' open faces (graph, the
                         default), or everything loaded inside the view (frustum)
  --smooth-lighting <on|off>
                         whether corners are lit and darkened by the blocks around
                         them (on, the default) or every face has one light (off)
  --biome-blend <0..7>   over how many columns each way grass, leaves and water take
                         the mean of the biomes' colours (default 2: 5 x 5)
  --terrain-shader <rgss|rgss-reference|nearest|plain>
                         how the terrain's texels are filtered: far ones averaged
                         as vanilla's RGSS does it (rgss, the default; rgss-reference
                         takes every sample the game's shader takes, for the same
                         picture), or all with hard edges (nearest); plain is
                         nearest without fog or fade, to measure those against
  --fog <on|off>         whether the terrain fades into the fog's colour with
                         distance (on, the default)
  --chunk-fade <seconds> how long a section takes to fade in once it has a mesh,
                         0 to 1.5 (default 0.75; 0: it is there at once)
  --sky <late|first|off> when the sky is drawn: after the world's opaque layers,
                         only where they left nothing (late, the default), before
                         the world as vanilla does it (first), or not at all
  --translucent <on|off> whether the world's translucent layer (water, stained glass,
                         ice) is drawn (on, the default); off, to measure it against
  --block-entities <on|off>
                         whether chests, shulker boxes, skulls and banners are drawn
                         (on, the default); off, to measure it against
  --players <on|off>     whether other players and mannequins are drawn (on, the
                         default); off, to measure it against
  --entity-shadows <on|off>
                         whether players cast shadows (on, the default)
  --hand <on|off>        whether the player's own hands and what they hold are drawn
                         (on, the default)
  --block-outline <on|off>
                         whether the block the crosshair is on is outlined (on, the
                         default)
  --view-bobbing <on|off>
                         whether the view sways with the player's steps (on, the
                         default), as vanilla's View Bobbing
  --crosshair <on|off>   whether the crosshair is drawn (on, the default)
  --vignette <on|off>    whether the frame's corners darken in the dark (on, the
                         default), as vanilla's Vignette
  --player-list <on|off> whether the player list shows without its key held (off, the
                         default)
  --hud <on|off>         whether the HUD is drawn over the world (on, the default);
                         off, for a picture to hold against another client's without
                         its HUD
  --animations <on|off>  whether animated textures (water, lava, fire) play (on, the
                         default); off, they show their first frame
  --fov-effects <on|off> whether the field of view widens and narrows with speed,
                         flight, bows and fluids (on, the default), as vanilla's FOV
                         Effects at 100% or at 0%
  --clouds <off|fast|fancy>
                         the clouds, as vanilla's Clouds option (fancy, the default)
  --weather <on|off>     whether rain and snow are drawn (on, the default)
  --weather-time <ticks> scroll rain and snow to that game time, for a picture to hold
                         against another client's
  --cloud-distance <chunks>
                         how far out clouds are drawn, 2 to 128 (128, the default),
                         as vanilla's Cloud Distance
  --cloud-time <ticks>   draw the clouds where they are at that game time, for a
                         picture to hold against another client's
  --light-flicker <on|off>
                         whether block light flickers (on, the default); off, two
                         frames of one place and time are the same picture
  --route <file>         with --join: play a route file from cargo xtask bench
                         dedicated once in the world, measure the frames of its
                         capture (for --bench-out, with every frame interval and what
                         the terrain did), and quit when it ends; the run fails unless
                         every tick is the route's. The keyboard and the mouse do
                         nothing in the world meanwhile. With --screenshot, the
                         picture is of the frame after the route's end
  --route-out <file>     write the route's record (positions, chunks, checksums)
  --expect-positions <checksum> | --expect-footprint <checksum>
                         fail unless the route's record ends with these checksums
  --input '<steps>'      once in the world, press keys and mouse buttons as if by hand
                         (development): steps like key.keyboard.w+@500 (down, 500 ms
                         after the step before), key.mouse.4-@50 (up), shot@1000 (the
                         --screenshot); keys go through the key bindings
The options above that change a setting override it for the run; it is saved with
them if a setting changes. In the window, F3 toggles the frame-stats overlay and F11
fullscreen (or the keys bound to the debug screen and to fullscreen); Escape always
opens the menu. Key bindings live in config/client.toml's [keys]: an empty [keys]
resets them all, and without one they are imported from vanilla's options.txt again.

commands:
  version
      Print the version and the disclaimer.
  locate  [--from <folder>]
      List the launcher folders searched for a verified game install.
  prepare [--from <folder>] [--data-dir <folder>] [--java <java>] [--exporters <jar>]
      Import the game files from your launcher into Ruthenium's data folder,
      then run Mojang's data generator and Ruthenium's exporters on them. Runs
      only what is out of date.
  assets  [--data-dir <folder>] [--pack <zip or folder>]... [--mip-levels <0-4>]
          [--dump <folder>]
      Build the sprite atlases from the imported game files and the given
      resource packs (lowest first), print their sizes, and optionally write
      every atlas level as PNG with a sprite index.
  auth login [--device] | list | check [<name>] [--certificates] | logout <name>
      Sign in with a Microsoft account (in the browser, or with a code to enter
      on another device), list the accounts signed in, check one by signing in
      again from its saved sign-in, or sign one out. Saved sign-ins live in the
      system's credential store.
  connect <address> [--seconds <s>] [--data-dir <folder>] [--script '<steps>']
          [--light-out <file>] [--light-against <file>]
          [--route <file> [--route-out <file>] [--expect-positions <checksum>]
           [--expect-footprint <checksum>]]
      Join a server without a window, print each step, and disconnect after
      <s> seconds (default 10). Joins as the signed-in account; development
      builds with the dev-offline feature join offline-mode servers on this
      machine or the local network as an offline player. --script plays timed
      input. --light-out writes every loaded chunk's light before leaving, and
      --light-against compares it with such a file and fails if a level differs. --route plays a tick-keyed route file from cargo xtask bench
      dedicated instead, leaves when it ends, and fails unless the start, every
      tick and the loaded chunks are what the route and the checksums say.";

fn main() -> ExitCode {
    let started = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("version") => {
            println!("Ruthenium {}", env!("CARGO_PKG_VERSION"));
            println!("{DISCLAIMER}");
            ExitCode::SUCCESS
        }
        Some("locate") => prepare_cmd::locate(&args[1..]),
        Some("prepare") => prepare_cmd::prepare(&args[1..]),
        Some("assets") => assets_cmd::run(&args[1..]),
        Some("auth") => auth_cmd::run(&args[1..]),
        Some("connect") => connect_cmd::run(&args[1..]),
        Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        a if a.is_none_or(|a| a.starts_with("--")) => match client::Options::parse(&args) {
            Ok(opts) => {
                println!("Ruthenium {} - {DISCLAIMER}", env!("CARGO_PKG_VERSION"));
                client::run(&opts, started)
            }
            Err(e) => {
                eprintln!("ruthenium: {e}\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}
