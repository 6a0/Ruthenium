//! In the world: the mouse grabbed and turning the camera, the movement, hotbar and
//! swap-hands keys, the use button and the scroll wheel handed to the game thread; where
//! the world is seen from; and a readout of where the player is and what the terrain is
//! doing (always while the world can't be drawn, else with the frame statistics).
//!
//! Vanilla's rules: keys count only while no screen is open (`KeyMapping.releaseAll`
//! when one opens); the mouse's motion is summed over a frame and turns the camera once
//! per frame, after the frame's events (`handleAccumulatedMovement`), the first motion
//! after a grab dropped; the game pauses (opens the Game Menu) once the window has been
//! out of focus for half a second (`pauseIfInactive`, `pauseOnLostFocus` on).

use std::time::{Duration, Instant};

use std::sync::Arc;

use ruthenium_data::v26_3::registries::MobEffect;
use ruthenium_render::{CameraPose, WorldStats};
use ruthenium_sim::environment::Environment;
use ruthenium_sim::input::{self, HeldKeys, KeyBinds, MouseMotion, MouseOptions, ScrollWheel};
use ruthenium_sim::player::Pose;
use ruthenium_sim::scene::Scene;
use ruthenium_sim::session::{PlayerView, Session};
use ruthenium_ui::modern::{Rect, ShapeList, UiScale};
use ruthenium_ui::text::{Fonts, GlyphAtlas, TextList, TextStyle, Weight};

/// How long the window may be out of focus before the game pauses.
const PAUSE_AFTER: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct InGame {
    motion: MouseMotion,
    held: HeldKeys,
    wheel: ScrollWheel,
    binds: KeyBinds,
    focused: bool,
    /// Since when the window has been out of focus.
    unfocused_since: Option<Instant>,
}

impl Default for InGame {
    fn default() -> InGame {
        InGame {
            motion: MouseMotion::new(),
            held: HeldKeys::default(),
            wheel: ScrollWheel::default(),
            binds: KeyBinds::DEFAULT,
            focused: true,
            unfocused_since: None,
        }
    }
}

impl InGame {
    /// The screen closed and the mouse is grabbed again (`grabMouse`).
    pub fn enter(&mut self, session: Option<&Session>) {
        self.motion = MouseMotion::new();
        self.release(session);
    }

    /// A screen opened (`KeyMapping.releaseAll`).
    pub fn release(&mut self, session: Option<&Session>) {
        self.held.release_all();
        if let Some(s) = session {
            s.set_keys(Default::default());
            s.set_use(false);
            s.set_attack(false);
        }
    }

    /// A mouse button went down or up in the world (SDL's numbering: 1 left, 3 right):
    /// the left one attacks, the right one uses.
    pub fn mouse_button(&mut self, button: u8, down: bool, session: Option<&Session>) {
        let Some(s) = session else { return };
        match button {
            1 => s.set_attack(down),
            3 => s.set_use(down),
            _ => {}
        }
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn set_focus(&mut self, on: bool, session: Option<&Session>) {
        self.focused = on;
        self.unfocused_since = if on { None } else { Some(Instant::now()) };
        if !on {
            self.release(session);
        }
    }

    /// Whether the game should pause: out of focus long enough.
    pub fn should_pause(&self) -> bool {
        self.unfocused_since
            .is_some_and(|t| t.elapsed() > PAUSE_AFTER)
    }

    /// A key went down or up while in the world (not Escape).
    pub fn key(&mut self, scancode: u32, down: bool, session: Option<&Session>) {
        self.held.set(scancode, down);
        // `KeyMapping.click`: every press, repeats too, as vanilla counts them.
        if down
            && let Some(slot) = self.binds.hotbar_slot(scancode)
            && let Some(s) = session
        {
            s.hotbar_key(slot);
        }
        if down
            && scancode == self.binds.swap_offhand
            && let Some(s) = session
        {
            s.swap_hands_key();
        }
        if let Some(s) = session {
            s.set_keys(self.held.keys(&self.binds));
        }
    }

    /// Relative mouse motion (SDL's `xrel`, `yrel`).
    pub fn mouse_moved(&mut self, dx: f32, dy: f32) {
        self.motion.moved(dx, dy, self.focused);
    }

    /// The wheel, as SDL reports it (no natural-scrolling flip, as vanilla reads it).
    pub fn wheel(&mut self, x: f32, y: f32, session: Option<&Session>) {
        if let Some(w) = self.wheel.scrolled(f64::from(x), f64::from(y))
            && let Some(s) = session
        {
            s.scroll(w);
        }
    }

    /// The frame's turn of the camera, after its events.
    pub fn frame(&mut self, mouse: &MouseOptions, session: Option<&Session>) {
        let (dx, dy) = self.motion.take();
        if !self.focused || (dx == 0.0 && dy == 0.0) {
            return;
        }
        let (xo, yo) = input::turn_amount(dx, dy, mouse);
        if let Some(s) = session {
            s.turn(xo, yo);
        }
    }
}

/// The field of view from top to bottom, in degrees: the FOV option's default.
const FOV: f32 = 70.0;

/// What a frame draws of the world: what the game thread published, and where it is
/// seen from.
#[derive(Debug, Clone)]
pub struct WorldView {
    pub scene: Scene,
    pub camera: CameraPose,
    pub environment: Arc<Environment>,
}

/// The world as the session's latest frame has it, seen from the player's eyes: the
/// player's place between its last two ticks (`now` says how far), looking where the
/// mouse has turned to since.
pub fn world_view(session: &Session, now: Instant) -> Option<WorldView> {
    let camera = session.camera()?;
    let f = &camera.frame;
    let t = f64::from(f.partial_tick(now));
    let p = &f.player;
    let mut position: [f64; 3] =
        std::array::from_fn(|i| p.old_position[i] + (p.position[i] - p.old_position[i]) * t);
    position[1] += f64::from(p.eye_height);
    Some(WorldView {
        scene: session.scene().clone(),
        camera: CameraPose {
            position,
            y_rot: camera.y_rot,
            x_rot: camera.x_rot,
            fov: FOV,
        },
        environment: f.environment.clone(),
    })
}

/// What the terrain is doing, as lines for the readout.
pub fn terrain_lines(s: &WorldStats) -> String {
    let mib = |bytes: u64| bytes as f64 / f64::from(1 << 20);
    let average = if s.tracker.jobs == 0 {
        0.0
    } else {
        s.mesh_micros as f64 / s.tracker.jobs as f64
    };
    format!(
        "Terrain: {} sections with a mesh of {} ({} waiting, {} jobs out, {} on their way); drawn {} of {} tested, {} quads in {} draws with {} buffer binds\n\
         Meshed {} times ({} for the player's changes, {} stale, {} waits for neighbours), {:.2} ms a section, slowest {:.2} ms; to the screen in at most {:.1} ms\n\
         Pages {:.1} MiB holding {:.1} MiB ({:.0} planned); left out: {} translucent quads, {} meshes for memory, {} sections a frame's data had no room for",
        s.meshed,
        s.sections,
        s.dirty,
        s.jobs_out,
        s.pending,
        s.visible,
        s.tested,
        s.quads,
        s.draws,
        s.binds,
        s.tracker.jobs,
        s.tracker.urgent_jobs,
        s.tracker.stale,
        s.tracker.waits_for_neighbours,
        average / 1000.0,
        s.slowest_mesh_micros as f64 / 1000.0,
        s.slowest_install_ms,
        mib(s.pool.reserved_bytes),
        mib(s.pool.payload_bytes),
        mib(ruthenium_render::terrain::POOL_ENVELOPE),
        s.translucent_quads,
        s.dropped,
        s.overflowed,
    )
}

/// `Mth.wrapDegrees` for floats.
fn wrap_degrees(v: f32) -> f32 {
    let mut w = v % 360.0;
    if w >= 180.0 {
        w -= 360.0;
    }
    if w < -180.0 {
        w += 360.0;
    }
    w
}

/// The readout: where the player is and what it's doing, a crosshair, and how to get
/// to the menu.
/// What the game thread has published for drawing and how the handoff to it fares, as
/// a line for the HUD while there is no world to look at: the camera as drawing would
/// take it (the latest frame, turned by the mouse's moves the game thread has yet to
/// take in), the published chunks, and the queues' worst moments.
pub fn scene_line(session: &Session) -> Option<String> {
    let camera = session.camera()?;
    let f = &camera.frame;
    let scene = session.scene();
    let feet = f.player.position.map(|v| v.floor() as i32);
    let (chunks, light) = scene.with_terrain(|t| (t.len(), t.probe(feet)));
    let light = light.map_or_else(
        || "no chunk here".to_owned(),
        |p| format!("light {} sky, {} block", p.sky_light, p.block_light),
    );
    let dirty = scene.queue_stats();
    let handoff = session.handoff_stats();
    // From the main thread handing the frame's input over to the game thread having
    // published a frame with it.
    let input_to_frame = f.input_at.map_or(0.0, |at| {
        f.published.saturating_duration_since(at).as_secs_f64() * 1000.0
    });
    Some(format!(
        "Published: {chunks} chunks ({light}), level {}, frame {} with input {} ({} turns ahead, camera {:.1} / {:.1}), {} entities\n\
         Input to frame {input_to_frame:.2} ms; input queue at most {} frames, oldest {:.1} ms, {} waits; dirty queue at most {}, {} rescans",
        f.epoch,
        f.sequence,
        f.input_sequence,
        camera.turns_ahead,
        wrap_degrees(camera.y_rot),
        camera.x_rot,
        f.entities.len(),
        handoff.high_water,
        handoff.oldest_age.as_secs_f64() * 1000.0,
        handoff.stalls,
        dirty.high_water,
        dirty.overflows,
    ))
}

/// What the HUD's text says: the player as of its last tick, the published scene
/// ([`scene_line`]), the terrain ([`terrain_lines`]) and the server's name.
#[derive(Debug, Clone, Copy)]
pub struct HudText<'a> {
    pub view: Option<&'a PlayerView>,
    pub world: Option<&'a str>,
    pub terrain: Option<&'a str>,
    pub server: &'a str,
    /// The world is drawn behind the HUD.
    pub drawn: bool,
    /// Show the readout over a drawn world too (with the frame statistics).
    pub debug: bool,
}

pub fn draw_hud(
    hud: HudText<'_>,
    scale: &UiScale,
    fonts: &Fonts,
    atlas: Option<&GlyphAtlas>,
    shapes: &mut ShapeList,
    text: &mut TextList,
) {
    let HudText {
        view,
        world,
        terrain,
        server,
        drawn,
        debug,
    } = hud;
    let (cx, cy) = (scale.width / 2.0, scale.height / 2.0);
    let white = [1.0, 1.0, 1.0, 0.9];
    shapes.rounded(
        scale,
        Rect {
            x: cx - 9.0,
            y: cy - 1.0,
            w: 18.0,
            h: 2.0,
        },
        1.0,
        white,
    );
    shapes.rounded(
        scale,
        Rect {
            x: cx - 1.0,
            y: cy - 9.0,
            w: 2.0,
            h: 18.0,
        },
        1.0,
        white,
    );
    let Some(atlas) = atlas else { return };
    // Over a drawn world there is only the crosshair, unless the statistics are on.
    if drawn && !debug {
        return;
    }
    let style = |size: f32, weight| TextStyle {
        size: size * scale.px,
        weight,
        tracking: 0.0,
    };
    let mut lines = vec![(server.to_owned(), style(20.0, Weight::SEMIBOLD))];
    let body = style(16.0, Weight::MEDIUM);
    match view {
        Some(v) => {
            let p = v.position;
            lines.push((
                format!("XYZ: {:.3} / {:.5} / {:.3}", p[0], p[1], p[2]),
                body,
            ));
            lines.push((
                format!("Facing: {:.1} / {:.1}", wrap_degrees(v.y_rot), v.x_rot),
                body,
            ));
            let mut state = vec![format!("Slot {}", v.selected_slot + 1)];
            if v.on_ground {
                state.push("on the ground".into());
            }
            if v.sprinting {
                state.push("sprinting".into());
            }
            if v.flying {
                state.push("flying".into());
            }
            if v.pose != Pose::Standing {
                state.push(format!("{:?}", v.pose).to_lowercase());
            }
            lines.push((state.join(", "), body));
            let mode = match v.game_type {
                0 => "Survival",
                1 => "Creative",
                2 => "Adventure",
                _ => "Spectator",
            };
            let mut status = vec![
                mode.to_owned(),
                format!("health {:.1}", v.health),
                format!("food {}", v.food),
            ];
            for &(id, amplifier, duration) in &v.effects {
                let name = u32::try_from(id)
                    .ok()
                    .and_then(MobEffect::from_id)
                    .map_or("unknown effect", |e| {
                        e.name().trim_start_matches("minecraft:")
                    });
                let time = if duration < 0 {
                    "∞".to_owned()
                } else {
                    format!("{}s", duration / 20)
                };
                status.push(format!("{name} {} ({time})", amplifier + 1));
            }
            lines.push((status.join(", "), body));
            let hand = |s: &str| {
                if s.is_empty() {
                    "empty".to_owned()
                } else {
                    s.trim_start_matches("minecraft:").replace('*', " x")
                }
            };
            let mut hands = format!(
                "Main hand: {}, off hand: {}",
                hand(&v.main_hand),
                hand(&v.off_hand)
            );
            if let Some((h, item, left)) = &v.using {
                let h = if *h == "OFF_HAND" { "off" } else { "main" };
                hands.push_str(&format!(
                    "; using {} ({h} hand, {left} ticks left)",
                    item.trim_start_matches("minecraft:")
                ));
            }
            hands.push_str(&format!(
                "; attack {:.0}%",
                f64::from(v.attack_strength) * 100.0
            ));
            if let Some(([x, y, z], progress)) = v.breaking {
                hands.push_str(&format!(
                    "; breaking {x} {y} {z} {:.0}%",
                    f64::from(progress.min(1.0)) * 100.0
                ));
            }
            lines.push((hands, body));
            lines.push((format!("{} ({} chunks)", v.dimension, v.chunks), body));
            for line in world.into_iter().chain(terrain).flat_map(str::lines) {
                lines.push((line.to_owned(), body));
            }
        }
        None => lines.push(("Joining...".into(), body)),
    }
    // Below the frame statistics, which sit in the same corner.
    let mut y = if debug {
        ruthenium_render::Overlay::BOTTOM + 28.0 * scale.px
    } else {
        36.0 * scale.px
    };
    for (s, st) in &lines {
        let line = fonts.shape(s, *st);
        text.line(atlas, &line, 24.0 * scale.px, y, white);
        y += st.size * 1.5;
    }
    if drawn {
        return;
    }
    let hint = fonts.shape(
        "The world isn't drawn yet. Esc for the menu.",
        style(15.0, Weight::MEDIUM),
    );
    text.line(
        atlas,
        &hint,
        (scale.width * scale.px - hint.width) / 2.0,
        (scale.height - 32.0) * scale.px,
        [1.0, 1.0, 1.0, 0.6],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_degrees_like_vanilla() {
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-180.0), -180.0);
        assert_eq!(wrap_degrees(180.0), -180.0);
        assert_eq!(wrap_degrees(-540.0), -180.0);
        assert_eq!(wrap_degrees(725.0), 5.0);
    }

    #[test]
    fn hotbar_and_movement_keys_reach_the_input() {
        let mut g = InGame::default();
        g.enter(None);
        g.key(KeyBinds::DEFAULT.forward, true, None);
        assert!(g.held.keys(&g.binds).forward);
        g.set_focus(false, None);
        assert!(
            !g.held.keys(&g.binds).forward,
            "focus loss lets go of the keys"
        );
        assert!(!g.should_pause(), "not before half a second");
        g.unfocused_since = Some(Instant::now() - Duration::from_secs(1));
        assert!(g.should_pause());
        g.set_focus(true, None);
        assert!(!g.should_pause());
    }
}
