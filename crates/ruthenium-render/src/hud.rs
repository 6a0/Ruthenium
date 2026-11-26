//! The HUD over the world: `ruthenium-ui` lays it out from what a frame of the game
//! thread says of the player, and it is drawn here in vanilla's order: the crosshair, the
//! quads under the hotbar's items, the items (models, seen straight on with the lights of
//! a GUI), and the quads over them with the texts between them.

use std::sync::Arc;

use ruthenium_assets::atlas::Atlas;
use ruthenium_gpu::{
    Arrival, FrameData, Pass, SamplerId, TextureDesc, TextureId, TextureTable, UploadRing, vk,
};
use ruthenium_protocol::text::{BoundKeys, WithKeys};
use ruthenium_sim::hud::OFF_HAND;
use ruthenium_sim::scene::Frame;
use ruthenium_ui::hud::{
    ChatInput, ChatShown, HudHealth, HudLayout, HudMemory, HudOptions, HudSprites, HudView,
};
use ruthenium_ui::{GuiScale, Quad, Sprites};

use crate::block_entity::renderer::BlockEntityRenderer;
use crate::gui;
use crate::pipelines::PipelineId;
use crate::sky::SkyImage;

/// How much of the way to the darkness around the eyes the vignette goes in a tick.
const VIGNETTE_STEP: f32 = 0.01;
/// Ticks at most that a frame takes the vignette on by: after a pause it does not catch
/// up for long.
const VIGNETTE_TICKS: u64 = 200;

/// `Lightmap.getBrightness`: how bright light of `level` (0 to 15) is in a dimension
/// with that ambient light.
fn brightness(ambient: f32, level: i32) -> f32 {
    let v = level as f32 / 15.0;
    let curved = v / (4.0 - 3.0 * v);
    curved + ambient * (1.0 - curved)
}

/// The light around the eyes, which the vignette follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EyeLight {
    /// `getMaxLocalRawBrightness` at the eyes: the block's light or the sky's less what
    /// the sky lacks now, whichever is more.
    pub level: i32,
    /// The dimension's ambient light.
    pub ambient: f32,
}

/// The vignette (`Hud.updateVignetteBrightness`, `extractVignette`): how dark it is, and
/// the ticks it has taken in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Vignette {
    darkness: f32,
    /// The level and its tick the darkness is of.
    at: Option<(u64, u64)>,
}

impl Vignette {
    /// Takes the vignette on to tick `ticks` of level `epoch`, in light `light`: a
    /// hundredth of the way to the darkness there with every tick.
    fn update(&mut self, epoch: u64, ticks: u64, light: EyeLight) {
        let elapsed = match self.at {
            Some((level, last)) if level == epoch => ticks.saturating_sub(last),
            _ => 0,
        };
        let target = (1.0 - brightness(light.ambient, light.level)).clamp(0.0, 1.0);
        for _ in 0..elapsed.min(VIGNETTE_TICKS) {
            self.darkness += (target - self.darkness) * VIGNETTE_STEP;
        }
        self.at = Some((epoch, ticks));
    }

    /// The colour its picture is drawn in: what it takes from what is behind it
    /// (`ARGB.colorFromFloat(1, b, b, b)`).
    fn colour(&self) -> u32 {
        let b = (self.darkness.clamp(0.0, 1.0) * 255.0).floor() as u32;
        0xFF00_0000 | b << 16 | b << 8 | b
    }
}

/// How a frame's HUD is to be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudParams {
    /// The GUI Scale option: the largest scale taken, or 0 for the largest that fits.
    pub gui_scale: u32,
    pub options: HudOptions,
    /// The Vignette option: whether the frame's corners darken in the dark.
    pub vignette: bool,
    /// A clock in milliseconds that only goes on (`Util.getMillis`).
    pub millis: i64,
    /// The time itself.
    pub now: std::time::Instant,
}

/// What the last frame's HUD was made of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HudStats {
    pub quads: u32,
    pub items: u32,
    pub glyphs: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: EyeLight = EyeLight {
        level: 15,
        ambient: 0.0,
    };

    #[test]
    fn light_is_as_bright_as_the_games_curve_says() {
        assert_eq!(brightness(0.0, 15), 1.0);
        assert_eq!(brightness(0.0, 0), 0.0);
        // 4 of 15: v / (4 - 3v).
        assert!((brightness(0.0, 4) - 1.0 / 12.0).abs() < 1e-6);
        // The Nether's ambient light lifts the dark a tenth of the way.
        assert_eq!(brightness(0.1, 0), 0.1);
        assert_eq!(brightness(0.1, 15), 1.0);
    }

    #[test]
    fn the_vignette_follows_the_light_a_hundredth_a_tick() {
        let mut v = Vignette {
            darkness: 1.0,
            at: None,
        };
        // The first frame of a level only notes its tick.
        v.update(1, 100, DAY);
        assert_eq!(v.darkness, 1.0);
        assert_eq!(v.colour(), 0xFFFF_FFFF);
        // In daylight it fades: a hundredth of what is left with each tick.
        v.update(1, 101, DAY);
        assert_eq!(v.darkness, 0.99);
        assert_eq!(v.colour(), 0xFFFC_FCFC);
        v.update(1, 103, DAY);
        assert!((v.darkness - 0.99f32.powi(3)).abs() < 1e-6);
        // Frames within a tick change nothing.
        let before = v;
        v.update(1, 103, DAY);
        assert_eq!(v, before);
        // In the dark it comes back, towards what the light there leaves.
        let night = EyeLight {
            level: 4,
            ambient: 0.0,
        };
        for tick in 104..2000 {
            v.update(1, tick, night);
        }
        assert!((v.darkness - 11.0 / 12.0).abs() < 1e-4);
        assert_eq!(v.colour(), 0xFFE9_E9E9);
        // Another level's ticks are not counted from this one's, and a long pause is not
        // caught up with for ever.
        let before = v.darkness;
        v.update(2, 5000, DAY);
        assert_eq!(v.darkness, before);
        v.update(2, 5000 + 1_000_000, DAY);
        assert!((v.darkness - before * 0.99f32.powi(200)).abs() < 1e-4);
    }
}

/// See the module docs.
#[derive(Debug)]
pub(crate) struct Hud {
    /// The GUI atlas until its upload has been started, then its texture and where the
    /// HUD's sprites are in it.
    waiting: Option<Atlas>,
    atlas: Option<(TextureId, HudSprites)>,
    /// The vignette's picture, until it is staged and after, and how dark it is.
    vignette_waiting: Option<SkyImage>,
    vignette_picture: Option<TextureId>,
    vignette: Vignette,
    /// The frame's vignette, over the whole GUI.
    vignette_quad: Option<Quad>,
    layout: HudLayout,
    health: HudHealth,
    memory: HudMemory,
    /// The key bindings the kept lines were laid out with.
    keys_seen: Option<Arc<BoundKeys>>,
    /// The quads over the items, with the texts and faces between them, and how many of
    /// them are glyphs'.
    over: Vec<Quad>,
    glyphs: u32,
    /// The GUI scale the frame is laid out for; 0 while there is nothing to draw.
    scale: u32,
    stats: HudStats,
}

impl Hud {
    /// A HUD of the sprites of the GUI's atlas, which [`Self::stage`] sends to the GPU.
    pub fn new(atlas: Atlas, vignette: Option<SkyImage>) -> Hud {
        Hud {
            waiting: Some(atlas),
            atlas: None,
            vignette_waiting: vignette,
            vignette_picture: None,
            // As dark as it gets, until the first ticks in a level.
            vignette: Vignette {
                darkness: 1.0,
                at: None,
            },
            vignette_quad: None,
            layout: HudLayout::new(),
            health: HudHealth::default(),
            memory: HudMemory::default(),
            keys_seen: None,
            over: Vec::new(),
            glyphs: 0,
            scale: 0,
            stats: HudStats::default(),
        }
    }

    /// Starts the GUI atlas on its way to the GPU once the ring has a stream to spare
    /// for it (it is too large to stage at once), and looks the HUD's sprites up in it:
    /// before the frame's `UploadRing::record`.
    pub fn stage(&mut self, textures: &mut TextureTable, uploads: &mut UploadRing) {
        if let Some(image) = self.vignette_waiting.take() {
            let whole = image.rgba.len() == image.width as usize * image.height as usize * 4;
            match textures.create_with(
                uploads,
                TextureDesc::rgba8(image.width, image.height),
                &[&image.rgba],
                Arrival::NextFrame,
            ) {
                Ok(id) if whole => self.vignette_picture = Some(id),
                Ok(id) => textures.destroy(id),
                // The ring is full for now.
                Err(_) if whole => self.vignette_waiting = Some(image),
                Err(_) => {}
            }
        }
        let Some(atlas) = self.waiting.take() else {
            return;
        };
        let Some(level) = atlas.levels.first() else {
            return;
        };
        let Ok(id) = textures.create(TextureDesc::rgba8(atlas.width, atlas.height)) else {
            self.waiting = Some(atlas);
            return;
        };
        if textures
            .stream(uploads, id, vec![level.to_rgba8()])
            .is_err()
        {
            textures.destroy(id);
            self.waiting = Some(atlas);
            return;
        }
        let sprites = HudSprites::new(&Sprites::from_atlas(&atlas, id.index()));
        self.atlas = Some((id, sprites));
    }

    pub fn stats(&self) -> HudStats {
        self.stats
    }

    /// How the chat screen's chat was last drawn.
    pub fn chat_shown(&self) -> Option<&ChatShown> {
        self.layout.chat_shown.as_ref()
    }

    /// No HUD this frame, unless [`Self::gather`] says otherwise.
    pub fn begin(&mut self) {
        self.scale = 0;
        self.vignette_quad = None;
        self.stats = HudStats::default();
    }

    /// Lays out the frame's HUD for a target of `size` pixels and hands its items and
    /// its text to `models`, whose font the text is in; text that names a key binding
    /// shows its key in `keys`.
    #[allow(clippy::too_many_arguments)]
    pub fn gather(
        &mut self,
        frame: &Frame,
        partial: f32,
        size: [u32; 2],
        params: &HudParams,
        keys: &Arc<BoundKeys>,
        light: EyeLight,
        models: &mut BlockEntityRenderer,
        block_atlas: TextureId,
        textures: &TextureTable,
        chat: Option<&ChatInput>,
    ) {
        // (Whether or not it is drawn: `Hud.tick`.)
        self.vignette.update(frame.epoch, frame.ticks, light);
        let Some((_, sprites)) = self.atlas.as_ref().filter(|a| textures.is_ready(a.0)) else {
            return;
        };
        let scale = GuiScale::new(size[0], size[1], params.gui_scale, false);
        let view = HudView {
            hud: &frame.hud,
            partial,
            width: scale.width as i32,
            height: scale.height as i32,
            millis: params.millis,
            now: params.now,
            options: params.options,
            chat,
        };
        if !self
            .keys_seen
            .as_ref()
            .is_some_and(|k| Arc::ptr_eq(k, keys))
        {
            self.memory.forget_lines();
            self.keys_seen = Some(keys.clone());
        }
        let lang = WithKeys {
            lang: models.lang(),
            keys,
        };
        self.layout.build(
            sprites,
            models.font(),
            &lang,
            &view,
            &mut self.health,
            &mut self.memory,
        );
        // An item in use shows it in its slot too: the selected one, or the other hand's.
        let r = &frame.player.first_person;
        let using = r.using.map(|off_hand| {
            let slot = if off_hand {
                OFF_HAND
            } else {
                usize::try_from(frame.hud.selected).unwrap_or(0)
            };
            let ticks = r.use_duration[usize::from(off_hand)] - r.use_remaining;
            (slot, ticks, r.use_remaining)
        });
        models.gather_gui_items(
            &self.layout.items,
            &frame.hud.slots,
            scale.scale as f32,
            using,
            block_atlas,
            textures,
        );
        self.over.clear();
        self.glyphs = models.gui_over(&self.layout, textures, &mut self.over);
        self.vignette_quad = self
            .vignette_picture
            // (Fully faded, it would darken nothing.)
            .filter(|t| {
                params.vignette
                    && self.vignette.colour() & 0x00FF_FFFF != 0
                    && textures.is_ready(*t)
            })
            .map(|texture| Quad {
                rect: [0.0, 0.0, scale.width as f32, scale.height as f32],
                uv: [0.0, 0.0, 1.0, 1.0],
                color: self.vignette.colour(),
                texture: texture.index(),
                slant: [0.0; 2],
            });
        self.scale = scale.scale;
    }

    /// Draws the frame's HUD into `pass`, which has a depth attachment, on a target of
    /// `size` pixels. `shared` is the light map and the quads' indices, which the items
    /// are drawn with.
    ///
    /// # Safety
    /// As [`BlockEntityRenderer::draw`]; [`Self::gather`] has run for this frame, and
    /// the frame's uploads have been recorded.
    pub unsafe fn draw(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        size: [f32; 2],
        models: &mut BlockEntityRenderer,
        shared: Option<(TextureId, (vk::Buffer, u64))>,
    ) {
        if self.scale == 0 {
            return;
        }
        let scale = self.scale as f32;
        let mut drawn = 0;
        let mut quads = |pass: &mut Pass<'_>,
                         frame_data: &mut FrameData,
                         pipeline: PipelineId,
                         quads: &[Quad]| {
            if quads.is_empty() {
                return;
            }
            let Some((first, bytes)) = frame_data.alloc(quads.len() as u32, gui::QUAD_BYTES) else {
                return;
            };
            gui::write_quads(quads, bytes);
            let mut push = [0u8; 16];
            push[0..4].copy_from_slice(&size[0].to_ne_bytes());
            push[4..8].copy_from_slice(&size[1].to_ne_bytes());
            push[8..12].copy_from_slice(&scale.to_ne_bytes());
            push[12..16].copy_from_slice(&first.to_ne_bytes());
            // SAFETY: the pipelines are those of passes with a depth attachment, their
            // push constants are laid out as gui.vert declares them, and the quads were
            // just written; their textures (the GUI atlas, the glyphs' pages) have
            // arrived.
            unsafe { pass.draw(pipeline.index(), &push, 6, quads.len() as u32) };
            drawn += quads.len() as u32;
        };
        // Under everything else of the HUD: its picture is smooth, and darkens what is
        // behind it.
        if let Some(quad) = self.vignette_quad
            && let Some((first, bytes)) = frame_data.alloc(1, gui::QUAD_BYTES)
        {
            gui::write_quad_with(quad, SamplerId::LinearRepeat, bytes);
            let mut push = [0u8; 16];
            push[0..4].copy_from_slice(&size[0].to_ne_bytes());
            push[4..8].copy_from_slice(&size[1].to_ne_bytes());
            push[8..12].copy_from_slice(&scale.to_ne_bytes());
            push[12..16].copy_from_slice(&first.to_ne_bytes());
            // SAFETY: as the quads below; its picture has arrived.
            unsafe { pass.draw(PipelineId::Vignette.index(), &push, 6, 1) };
        }
        quads(
            pass,
            frame_data,
            PipelineId::Crosshair,
            self.layout.crosshair.quads(),
        );
        quads(
            pass,
            frame_data,
            PipelineId::WorldGui,
            self.layout.under.quads(),
        );
        if let Some((lightmap, indices)) = shared.filter(|_| !self.layout.items.is_empty()) {
            // Among themselves the items' faces hide each other; nothing of the world
            // hides them.
            pass.clear_depth();
            // SAFETY: the caller's.
            unsafe { models.draw_gui_items(pass, frame_data, size, lightmap, indices) };
        }
        quads(pass, frame_data, PipelineId::WorldGui, &self.over);
        self.stats = HudStats {
            quads: drawn.saturating_sub(self.glyphs),
            items: self.layout.items.len() as u32,
            glyphs: self.glyphs,
        };
    }
}
