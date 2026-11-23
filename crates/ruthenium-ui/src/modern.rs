//! Ruthenium's own menus (title, server list, accounts, settings, connecting): a modern,
//! space-themed look, unrelated to vanilla's GUI. Layout is in UI units, scaled to the
//! window by [`UiScale`]; shapes and glyphs come out in window pixels.

use crate::Clip;
use crate::text::{Fonts, GlyphAtlas, Line, TextList, TextStyle, Weight};

pub mod accounts;
pub mod form;
pub mod list;
pub mod servers;
pub mod settings;

pub use accounts::{AccountView, AccountsScreen};
pub use form::{Field, FormButton, FormScreen, FormSpec, TextField};
pub use list::ScrollList;
pub use servers::{Ping, RowView, ServerListScreen};
pub use settings::{ControlSpec, SectionSpec, SettingSpec, SettingsEvent, SettingsScreen};

/// Window pixels per UI unit: the layout is designed for a window 900 units tall, so it
/// keeps its proportions at any size (never below 0.75 px per unit, so text stays
/// readable in small windows).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiScale {
    pub px: f32,
    /// Window size in UI units.
    pub width: f32,
    pub height: f32,
}

impl UiScale {
    const DESIGN_HEIGHT: f32 = 900.0;
    const MIN: f32 = 0.75;

    pub fn new(pixel_width: u32, pixel_height: u32) -> UiScale {
        Self::sized(pixel_width, pixel_height, 1.0)
    }

    /// Like [`Self::new`], with everything `factor` times as large (the Menu Size
    /// setting).
    pub fn sized(pixel_width: u32, pixel_height: u32, factor: f32) -> UiScale {
        let px = (pixel_height as f32 / Self::DESIGN_HEIGHT).max(Self::MIN) * factor;
        UiScale {
            px,
            width: pixel_width as f32 / px,
            height: pixel_height as f32 / px,
        }
    }

    /// Window coordinates (points) to UI units, for a window `window_size` points large
    /// and `pixel_size` pixels large.
    pub fn from_window(
        &self,
        x: f32,
        y: f32,
        window_size: (u32, u32),
        pixel_size: (u32, u32),
    ) -> (f32, f32) {
        let sx = pixel_size.0 as f32 / window_size.0.max(1) as f32;
        let sy = pixel_size.1 as f32 / window_size.1.max(1) as f32;
        (x * sx / self.px, y * sy / self.px)
    }
}

/// A rectangle in UI units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    /// Scaled by `k` about its centre.
    pub fn scaled(&self, k: f32) -> Rect {
        let (w, h) = (self.w * k, self.h * k);
        Rect {
            x: self.x + 0.5 * (self.w - w),
            y: self.y + 0.5 * (self.h - h),
            w,
            h,
        }
    }

    /// Grown by `d` on every side.
    pub fn outset(&self, d: f32) -> Rect {
        Rect {
            x: self.x - d,
            y: self.y - d,
            w: self.w + 2.0 * d,
            h: self.h + 2.0 * d,
        }
    }
}

/// One rounded shape for the renderer's shape pipeline (`shape.vert/frag`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// `x0, y0, x1, y1` in window pixels.
    pub rect: [f32; 4],
    /// Straight-alpha RGBA: the fill, or for glass the tint over the backdrop.
    pub fill: [f32; 4],
    /// Corner radius in pixels (half the height makes a pill).
    pub radius: f32,
    /// See-through frosted glass instead of a flat fill.
    pub glass: bool,
    /// Strength of the glass rim light (0 to 1).
    pub highlight: f32,
    /// How far, in pixels, the glass bends the backdrop at its rim.
    pub refraction: f32,
    /// Width in pixels of the glass's bevelled edge; the face inside it is flat.
    pub bevel: f32,
    /// Draw this texture (premultiplied alpha) instead of a fill, tinted by `fill`.
    pub image: Option<ShapeImage>,
    /// Flat shapes: keep only a band this many pixels wide inside the edge (0 fills).
    pub ring: f32,
    /// Set by [`ShapeList::push`] from the list's current clip.
    pub clip: Clip,
}

/// A texture shown by a [`Shape`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeImage {
    /// The renderer's texture index.
    pub texture: u32,
    /// `u0, v0, u1, v1`.
    pub uv: [f32; 4],
}

/// Shapes for one frame, back to front.
#[derive(Debug, Default)]
pub struct ShapeList {
    shapes: Vec<Shape>,
    clip: Clip,
}

impl ShapeList {
    pub fn new() -> ShapeList {
        ShapeList {
            shapes: Vec::with_capacity(64),
            clip: Clip::NONE,
        }
    }

    pub fn clear(&mut self) {
        self.shapes.clear();
        self.clip = Clip::NONE;
    }

    /// The clip for shapes added from now on.
    pub fn set_clip(&mut self, clip: Clip) {
        self.clip = clip;
    }

    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// Multiplies the opacity of every shape so far by `k`.
    pub fn fade_all(&mut self, k: f32) {
        for s in &mut self.shapes {
            s.clip.opacity *= k;
        }
    }

    /// Adds `shape` under the current clip.
    pub fn push(&mut self, shape: Shape) {
        self.shapes.push(Shape {
            clip: self.clip,
            ..shape
        });
    }

    /// Glass over `r` (UI units) with corners of `radius` units.
    pub fn glass_rect(
        &mut self,
        scale: &UiScale,
        r: Rect,
        radius: f32,
        tint: [f32; 4],
        highlight: f32,
    ) {
        let px = scale.px;
        let bevel = GLASS_BEVEL.min(radius).min(0.5 * r.h) * px;
        self.push(Shape {
            rect: px_rect(scale, r),
            fill: tint,
            radius: radius * px,
            glass: true,
            highlight,
            refraction: GLASS_REFRACTION * bevel,
            bevel,
            image: None,
            ring: 0.0,
            clip: Clip::NONE,
        });
    }

    /// A glass pill over `r` (UI units).
    pub fn glass_pill(&mut self, scale: &UiScale, r: Rect, tint: [f32; 4], highlight: f32) {
        self.glass_rect(scale, r, 0.5 * r.h, tint, highlight);
    }

    /// A flat rounded rectangle (UI units).
    pub fn rounded(&mut self, scale: &UiScale, r: Rect, radius: f32, color: [f32; 4]) {
        self.ring(scale, r, radius, 0.0, color);
    }

    /// A flat band `width` units wide just inside the edge of `r` (0 fills it).
    pub fn ring(&mut self, scale: &UiScale, r: Rect, radius: f32, width: f32, color: [f32; 4]) {
        let px = scale.px;
        self.push(Shape {
            rect: px_rect(scale, r),
            fill: color,
            radius: radius * px,
            glass: false,
            highlight: 0.0,
            refraction: 0.0,
            bevel: 0.0,
            image: None,
            ring: width * px,
            clip: Clip::NONE,
        });
    }

    /// A flat ring `width` units wide just inside the edge of the pill `r` (UI units).
    pub fn pill_ring(&mut self, scale: &UiScale, r: Rect, width: f32, color: [f32; 4]) {
        self.ring(scale, r, 0.5 * r.h, width, color);
    }

    /// A texture over `r` (UI units), premultiplied alpha.
    pub fn image(&mut self, scale: &UiScale, r: Rect, texture: u32) {
        self.image_rounded(scale, r, 0.0, texture);
    }

    /// A texture over `r` with rounded corners (UI units), premultiplied alpha.
    pub fn image_rounded(&mut self, scale: &UiScale, r: Rect, radius: f32, texture: u32) {
        self.push(Shape {
            rect: px_rect(scale, r),
            fill: [1.0; 4],
            radius: radius * scale.px,
            glass: false,
            highlight: 0.0,
            refraction: 0.0,
            bevel: 0.0,
            image: Some(ShapeImage {
                texture,
                uv: [0.0, 0.0, 1.0, 1.0],
            }),
            ring: 0.0,
            clip: Clip::NONE,
        });
    }
}

/// `r` (UI units) as `x0, y0, x1, y1` window pixels.
pub fn px_rect(scale: &UiScale, r: Rect) -> [f32; 4] {
    let px = scale.px;
    [r.x * px, r.y * px, (r.x + r.w) * px, (r.y + r.h) * px]
}

/// What a menu button asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Multiplayer,
    Settings,
    Quit,
    /// Back to the previous screen (also Escape).
    Back,
    /// Server list.
    Join,
    DirectConnect,
    AddServer,
    EditServer,
    DeleteServer,
    Refresh,
    /// Forms: the main button (Done, Join, Delete) and Cancel.
    Confirm,
    Cancel,
    /// The title's account chip: the Accounts screen.
    Accounts,
    /// Accounts: sign in another account, switch to the selected one, sign it out.
    AddAccount,
    UseAccount,
    SignOut,
    /// Signing in: switch to a device code, open the sign-in page again, copy the code.
    UseCode,
    OpenPage,
    CopyCode,
    /// Steps a choice button to its next value (the resource pack setting).
    Cycle,
}

/// A pill button.
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub rect: Rect,
    /// The label (Ruthenium's own text; translations come later).
    pub label: &'static str,
    pub action: Action,
}

/// Glass is a flat slab with a bevelled edge: the edge's width (UI units), the same
/// whatever the corner radius, and how far it bends the backdrop (fraction of the width).
const GLASS_BEVEL: f32 = 4.0;
const GLASS_REFRACTION: f32 = 0.6;

/// Glass tint at rest and lit (hovered or focused); a press adds `PRESS_TINT` alpha.
const TINT: [f32; 4] = [0.85, 0.88, 1.0, 0.04];
const TINT_HOVER: [f32; 4] = [0.90, 0.93, 1.0, 0.12];
const PRESS_TINT: f32 = 0.08;
const TINT_DISABLED: [f32; 4] = [0.85, 0.88, 1.0, 0.02];
const LABEL_DISABLED: [f32; 4] = [1.0, 1.0, 1.0, 0.32];
/// Size change of a hovered and of a pressed pill (fractions of its size).
const HOVER_GROW: f32 = 0.025;
const PRESS_SHRINK: f32 = 0.035;
/// Focus ring: its gap to the pill and its width (UI units), and its colour.
const FOCUS_GAP: f32 = 3.0;
const FOCUS_WIDTH: f32 = 2.0;
const FOCUS: [f32; 4] = [0.82, 0.87, 1.0, 0.9];

/// Time constants (seconds) of the pills' motion, towards lit and back: quick to answer,
/// slower to let go.
const HOVER_IN: f32 = 0.06;
const HOVER_OUT: f32 = 0.15;
const PRESS_IN: f32 = 0.03;
const PRESS_OUT: f32 = 0.12;
const FOCUS_IN: f32 = 0.05;
const FOCUS_OUT: f32 = 0.12;
/// A press shows for at least this long, so a quick click is still seen.
const MIN_PRESS: f32 = 0.08;

/// A value that eases towards its target, exponentially and at any frame rate.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ease {
    pub value: f32,
}

impl Ease {
    /// Moves towards `target` over `dt` seconds with the time constant `rise` when going
    /// up and `fall` when going down; snaps to it once within 1/500.
    pub fn step(&mut self, target: f32, dt: f32, rise: f32, fall: f32) {
        let tau = if target > self.value { rise } else { fall };
        self.value += (target - self.value) * (1.0 - (-dt / tau).exp());
        if (target - self.value).abs() < 0.002 {
            self.value = target;
        }
    }
}

/// Keys the menus understand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Down or Tab.
    Next,
    /// Up or Shift+Tab.
    Previous,
    /// Enter or Space.
    Activate,
}

/// How lit, pressed and focused one button is this frame, each from 0 to 1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ButtonLook {
    pub hover: f32,
    pub press: f32,
    pub focus: f32,
    /// Reduced motion: the button lights up but keeps its size.
    pub still: bool,
}

impl ButtonLook {
    /// How much larger (or, pressed, smaller) the button is drawn.
    pub fn size(&self) -> f32 {
        if self.still {
            1.0
        } else {
            1.0 + HOVER_GROW * self.hover - PRESS_SHRINK * self.press
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Motion {
    hover: Ease,
    press: Ease,
    focus: Ease,
    /// Seconds the press still shows after a quick click.
    hold: f32,
}

/// Pointer and keyboard state of a screen's buttons, by index, and their motion.
///
/// Like a browser: a click focuses the button without showing the focus ring; the
/// keyboard moves focus (wrapping) and shows the ring. Buttons act on release, of the
/// mouse over the button it went down on, or of Enter/Space on the button focused when
/// it went down; moving away first cancels. While the mouse holds a button, the others
/// do not light up.
#[derive(Debug, Clone, Default)]
pub struct ButtonStates {
    hovered: Option<usize>,
    focus: Option<usize>,
    focus_visible: bool,
    mouse_down: Option<usize>,
    key_down: Option<usize>,
    motion: Vec<Motion>,
    enabled: Vec<bool>,
    still: bool,
}

impl ButtonStates {
    pub fn new(buttons: usize) -> ButtonStates {
        ButtonStates {
            motion: vec![Motion::default(); buttons],
            enabled: vec![true; buttons],
            ..ButtonStates::default()
        }
    }

    /// Enables or disables button `i`. A disabled button does not light up, take
    /// presses or keyboard focus.
    pub fn set_enabled(&mut self, i: usize, on: bool) {
        if let Some(e) = self.enabled.get_mut(i) {
            *e = on;
        }
        if !on {
            if self.mouse_down == Some(i) {
                self.mouse_down = None;
            }
            if self.key_down == Some(i) {
                self.key_down = None;
            }
        }
    }

    pub fn enabled(&self, i: usize) -> bool {
        self.enabled.get(i).copied().unwrap_or(false)
    }

    /// The focused button, if any (whether or not its ring shows).
    pub fn focused(&self) -> Option<usize> {
        self.focus
    }

    /// Whether the focus ring shows (focus came from the keyboard).
    pub fn focus_visible(&self) -> bool {
        self.focus_visible
    }

    /// Moves focus, showing the ring or not.
    pub fn set_focus(&mut self, i: Option<usize>, visible: bool) {
        self.focus = i.filter(|&i| i < self.motion.len());
        self.focus_visible = visible;
        self.key_down = None;
    }

    /// The next enabled button after `from` going forwards or backwards, wrapping.
    fn step(&self, from: Option<usize>, forwards: bool) -> Option<usize> {
        let n = self.motion.len();
        let mut i = from;
        for _ in 0..n {
            let next = match (i, forwards) {
                (None, true) => 0,
                (None, false) => n - 1,
                (Some(i), true) => (i + 1) % n,
                (Some(i), false) => (i + n - 1) % n,
            };
            if self.enabled(next) {
                return Some(next);
            }
            i = Some(next);
        }
        None
    }

    /// How many buttons it tracks.
    pub fn len(&self) -> usize {
        self.motion.len()
    }

    pub fn is_empty(&self) -> bool {
        self.motion.is_empty()
    }

    /// The button under the pointer (`None` when off every button or out of the window).
    pub fn hover(&mut self, hit: Option<usize>) {
        self.hovered = hit;
    }

    /// The left mouse button went down over `hit`.
    pub fn mouse_down(&mut self, hit: Option<usize>) {
        self.hovered = hit;
        let hit = hit.filter(|&i| self.enabled(i));
        self.focus = hit;
        self.focus_visible = false;
        self.mouse_down = hit;
        self.key_down = None;
        if let Some(i) = hit {
            self.motion[i].hold = MIN_PRESS;
        }
    }

    /// The left mouse button went up over `hit`: the button it activates, if any.
    pub fn mouse_up(&mut self, hit: Option<usize>) -> Option<usize> {
        self.hovered = hit;
        self.mouse_down.take().filter(|&i| hit == Some(i))
    }

    /// A key went down or up: the button it activates, if any.
    pub fn key(&mut self, key: Key, down: bool) -> Option<usize> {
        let n = self.motion.len();
        if n == 0 || !down && key != Key::Activate {
            return None;
        }
        match (key, down) {
            (Key::Next | Key::Previous, _) => {
                self.focus = self.step(self.focus, key == Key::Next);
                self.focus_visible = true;
                self.key_down = None;
                None
            }
            (Key::Activate, true) => {
                self.focus_visible = true;
                match self.focus {
                    // Nothing focused yet: the first key only shows where focus starts.
                    None => self.focus = self.step(None, true),
                    Some(i)
                        if self.enabled(i)
                            && self.key_down.is_none()
                            && self.mouse_down.is_none() =>
                    {
                        self.key_down = Some(i);
                        self.motion[i].hold = MIN_PRESS;
                    }
                    Some(_) => {}
                }
                None
            }
            (Key::Activate, false) => self.key_down.take().filter(|&i| self.focus == Some(i)),
        }
    }

    /// Reduced motion: buttons light up and fade as usual, but no longer grow or shrink.
    pub fn set_reduce_motion(&mut self, on: bool) {
        self.still = on;
    }

    /// Drops any press without acting (the window lost focus).
    pub fn cancel(&mut self) {
        self.mouse_down = None;
        self.key_down = None;
    }

    /// Advances the motion by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        for (i, m) in self.motion.iter_mut().enumerate() {
            let hovered = self.hovered == Some(i)
                && self.enabled[i]
                && self.mouse_down.is_none_or(|d| d == i);
            let held = self.key_down == Some(i) || self.mouse_down == Some(i) && hovered;
            let pressed = held || m.hold > 0.0;
            m.hold = (m.hold - dt).max(0.0);
            let focused = self.focus_visible && self.focus == Some(i);
            m.hover
                .step(f32::from(u8::from(hovered)), dt, HOVER_IN, HOVER_OUT);
            m.press
                .step(f32::from(u8::from(pressed)), dt, PRESS_IN, PRESS_OUT);
            m.focus
                .step(f32::from(u8::from(focused)), dt, FOCUS_IN, FOCUS_OUT);
        }
    }

    /// Button `i`'s look this frame (at rest for an index past the end).
    pub fn look(&self, i: usize) -> ButtonLook {
        self.motion
            .get(i)
            .map_or_else(ButtonLook::default, |m| ButtonLook {
                hover: m.hover.value,
                press: m.press.value,
                focus: m.focus.value,
                still: self.still,
            })
    }
}

/// Draws a glass pill button over `rect` with its label, lit, pressed and focused as
/// `look` says; a disabled one is dim and still.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_button(
    shapes: &mut ShapeList,
    text: &mut TextList,
    atlas: Option<&GlyphAtlas>,
    s: &UiScale,
    rect: Rect,
    label: &Text,
    look: ButtonLook,
    enabled: bool,
) {
    if !enabled {
        shapes.glass_pill(s, rect, TINT_DISABLED, 0.4);
        if let Some(atlas) = atlas {
            label.centred(text, atlas, s, rect, LABEL_DISABLED);
        }
        return;
    }
    // Focus lights a pill half as much as the pointer; the ring says the rest.
    let lit = look.hover.max(0.5 * look.focus);
    let k = look.size();
    let r = rect.scaled(k);
    let mut tint = mix(TINT, TINT_HOVER, lit);
    tint[3] += PRESS_TINT * look.press;
    shapes.glass_pill(s, r, tint, 0.75 + 0.25 * lit + 0.2 * look.press);
    if look.focus > 0.0 {
        let mut ring = FOCUS;
        ring[3] *= look.focus;
        shapes.pill_ring(s, r.outset(FOCUS_GAP + FOCUS_WIDTH), FOCUS_WIDTH, ring);
    }
    if let Some(atlas) = atlas {
        label.centred_scaled(text, atlas, s, r, k, mix(LABEL, LABEL_HOVER, lit));
    }
}

/// Draws a focus ring around `r` (corner radius `radius`), `amount` from 0 to 1.
pub(crate) fn draw_focus_ring(
    shapes: &mut ShapeList,
    s: &UiScale,
    r: Rect,
    radius: f32,
    amount: f32,
) {
    if amount > 0.0 {
        let mut ring = FOCUS;
        ring[3] *= amount;
        let out = FOCUS_GAP + FOCUS_WIDTH;
        shapes.ring(s, r.outset(out), radius + out, FOCUS_WIDTH, ring);
    }
}

pub(crate) fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// The weights the menus use (the glyph atlas holds these).
pub const WEIGHTS: [Weight; 4] = [
    Weight::REGULAR,
    Weight::MEDIUM,
    Weight::SEMIBOLD,
    Weight::EXTRABOLD,
];

pub(crate) const WHITE_TEXT: [f32; 4] = WHITE;

const LOGO: f32 = 136.0;
const LABEL_SIZE: f32 = 19.0;
const SMALL_SIZE: f32 = 11.5;
const CHIP_SIZE: f32 = 13.0;
const WHITE: [f32; 4] = [0.95, 0.96, 1.0, 1.0];
const LABEL: [f32; 4] = [1.0, 1.0, 1.0, 0.88];
const LABEL_HOVER: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
pub(crate) const FAINT: [f32; 4] = [0.85, 0.87, 1.0, 0.45];
pub(crate) const SOFT: [f32; 4] = [0.88, 0.9, 1.0, 0.7];

/// Shaped text with the size it was shaped at (pixels) and its cap height (pixels).
#[derive(Debug, Clone)]
pub(crate) struct Text {
    pub(crate) line: Line,
    /// Cap height in pixels.
    pub(crate) cap: f32,
}

impl Text {
    pub(crate) fn new(
        fonts: &Fonts,
        text: &str,
        scale: &UiScale,
        size: f32,
        weight: Weight,
        tracking: f32,
    ) -> Text {
        let px = size * scale.px;
        let line = fonts.shape(
            text,
            TextStyle {
                size: px,
                weight,
                tracking,
            },
        );
        Text {
            cap: fonts.cap_height(weight) * px,
            line,
        }
    }

    /// Width in UI units.
    pub(crate) fn width(&self, scale: &UiScale) -> f32 {
        self.line.width / scale.px
    }

    /// Draws it with its baseline's left end at `(x, y)` (UI units).
    pub(crate) fn draw_at(
        &self,
        list: &mut TextList,
        atlas: &GlyphAtlas,
        scale: &UiScale,
        x: f32,
        y: f32,
        color: [f32; 4],
    ) {
        list.line(atlas, &self.line, x * scale.px, y * scale.px, color);
    }

    /// Draws it in `r` (UI units), capitals centred vertically, starting at the left.
    pub(crate) fn left_in(
        &self,
        list: &mut TextList,
        atlas: &GlyphAtlas,
        scale: &UiScale,
        r: Rect,
        color: [f32; 4],
    ) {
        let cy = (r.y + 0.5 * r.h) * scale.px;
        list.line(
            atlas,
            &self.line,
            r.x * scale.px,
            cy + 0.5 * self.cap,
            color,
        );
    }

    /// Draws it centred in `r` (UI units), optically: capitals centred vertically.
    pub(crate) fn centred(
        &self,
        list: &mut TextList,
        atlas: &GlyphAtlas,
        scale: &UiScale,
        r: Rect,
        color: [f32; 4],
    ) {
        self.centred_scaled(list, atlas, scale, r, 1.0, color);
    }

    /// Like `centred`, `k` times as large.
    pub(crate) fn centred_scaled(
        &self,
        list: &mut TextList,
        atlas: &GlyphAtlas,
        scale: &UiScale,
        r: Rect,
        k: f32,
        color: [f32; 4],
    ) {
        let (cx, cy) = ((r.x + 0.5 * r.w) * scale.px, (r.y + 0.5 * r.h) * scale.px);
        list.line_scaled(
            atlas,
            &self.line,
            cx - 0.5 * k * self.line.width,
            cy + 0.5 * k * self.cap,
            k,
            color,
        );
    }
}

/// How the account chip's dot shows the sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    SignedOut,
    Checking,
    SignedIn,
    /// Signed in with a limit (multiplayer off), or the sign-in failed.
    Problem,
}

impl Presence {
    fn color(self) -> [f32; 4] {
        match self {
            Presence::SignedOut => [0.85, 0.87, 1.0, 0.35],
            Presence::Checking => [0.62, 0.74, 1.0, 0.9],
            Presence::SignedIn => [0.45, 0.95, 0.62, 0.95],
            Presence::Problem => [1.0, 0.72, 0.38, 0.95],
        }
    }
}

/// The title's account chip: a small glass pill, top left, with the player's face and a
/// line.
#[derive(Debug, Clone)]
struct Chip {
    rect: Rect,
    text: Text,
}

const CHIP_H: f32 = 36.0;
const CHIP_FACE: f32 = 24.0;
const CHIP_FACE_RADIUS: f32 = 6.0;
const CHIP_MARGIN: f32 = 16.0;
/// The face sits as far from the chip's left edge as from the text.
const CHIP_GAP: f32 = 10.0;
/// Shown instead of a face until one is ready.
const FACE_PLACEHOLDER: [f32; 4] = [0.75, 0.8, 1.0, 0.14];

/// The title screen: the logo above a centred column of glass pills, the account chip in
/// the top left and the version in the bottom left. The logo is the brand: no name beside
/// it.
#[derive(Debug, Clone)]
pub struct TitleScreen {
    pub scale: UiScale,
    pub logo: Rect,
    pub buttons: Vec<Button>,
    labels: Vec<Text>,
    version: Text,
    /// The account (or why it isn't signed in): control number `buttons.len()`.
    chip: Chip,
}

impl TitleScreen {
    pub const BUTTON_WIDTH: f32 = 300.0;
    pub const BUTTON_HEIGHT: f32 = 52.0;
    pub const GAP: f32 = 14.0;

    /// Lays the screen out; `version` is the bottom-left line.
    pub fn new(scale: UiScale, fonts: &Fonts, version: &str) -> TitleScreen {
        let entries = [
            ("Multiplayer", Action::Multiplayer),
            ("Settings", Action::Settings),
            ("Quit", Action::Quit),
        ];
        let n = entries.len() as f32;
        let column = n * Self::BUTTON_HEIGHT + (n - 1.0) * Self::GAP;
        let space = 48.0;
        // The logo and the column together sit a little above the middle.
        let top = ((scale.height - (LOGO + space + column)) * 0.45).max(24.0);
        let cx = scale.width * 0.5;
        let logo = Rect {
            x: cx - 0.5 * LOGO,
            y: top,
            w: LOGO,
            h: LOGO,
        };
        let mut y = top + LOGO + space;
        let buttons: Vec<Button> = entries
            .into_iter()
            .map(|(label, action)| {
                let b = Button {
                    rect: Rect {
                        x: cx - Self::BUTTON_WIDTH * 0.5,
                        y,
                        w: Self::BUTTON_WIDTH,
                        h: Self::BUTTON_HEIGHT,
                    },
                    label,
                    action,
                };
                y += Self::BUTTON_HEIGHT + Self::GAP;
                b
            })
            .collect();
        let labels = buttons
            .iter()
            .map(|b| Text::new(fonts, b.label, &scale, LABEL_SIZE, Weight::SEMIBOLD, 0.01))
            .collect();
        let mut title = TitleScreen {
            version: Text::new(fonts, version, &scale, SMALL_SIZE, Weight::MEDIUM, 0.02),
            labels,
            scale,
            logo,
            buttons,
            chip: Chip {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                },
                text: Text::new(fonts, "", &scale, CHIP_SIZE, Weight::MEDIUM, 0.0),
            },
        };
        title.set_account(fonts, "Sign in");
        title
    }

    /// How many controls take pointer and keyboard focus: the pills, then the chip.
    pub fn controls(&self) -> usize {
        self.buttons.len() + 1
    }

    /// What control `i` does.
    pub fn action(&self, i: usize) -> Option<Action> {
        match self.buttons.get(i) {
            Some(b) => Some(b.action),
            None => (i == self.buttons.len()).then_some(Action::Accounts),
        }
    }

    /// Sets the account chip's line.
    pub fn set_account(&mut self, fonts: &Fonts, line: &str) {
        let s = self.scale;
        let text = Text::new(fonts, line, &s, CHIP_SIZE, Weight::MEDIUM, 0.0);
        let w = CHIP_GAP + CHIP_FACE + CHIP_GAP + text.width(&s) + 16.0;
        self.chip = Chip {
            rect: Rect {
                x: CHIP_MARGIN,
                y: CHIP_MARGIN,
                w,
                h: CHIP_H,
            },
            text,
        };
    }

    /// Adds the screen's shapes and, once they are on the GPU, its text (`atlas`), logo
    /// and the player's face (`logo`, `face`: texture indices); `states` says how lit
    /// each button is.
    pub fn draw(
        &self,
        shapes: &mut ShapeList,
        text: &mut TextList,
        atlas: Option<&GlyphAtlas>,
        logo: Option<u32>,
        face: Option<u32>,
        states: &ButtonStates,
    ) {
        let s = &self.scale;
        if let Some(logo) = logo {
            shapes.image(s, self.logo, logo);
        }
        for (i, (b, label)) in self.buttons.iter().zip(&self.labels).enumerate() {
            draw_button(
                shapes,
                text,
                atlas,
                s,
                b.rect,
                label,
                states.look(i),
                states.enabled(i),
            );
        }
        let chip = &self.chip;
        let look = states.look(self.buttons.len());
        let lit = look.hover.max(0.5 * look.focus);
        let r = chip.rect.scaled(if look.still {
            1.0
        } else {
            1.0 + 0.5 * HOVER_GROW * look.hover - PRESS_SHRINK * look.press
        });
        let mut tint = mix(TINT, TINT_HOVER, lit);
        tint[3] += PRESS_TINT * look.press;
        shapes.glass_pill(s, r, tint, 0.6 + 0.3 * lit);
        if look.focus > 0.0 {
            let mut ring = FOCUS;
            ring[3] *= look.focus;
            shapes.pill_ring(s, r.outset(FOCUS_GAP + FOCUS_WIDTH), FOCUS_WIDTH, ring);
        }
        // The face shrinks and grows with the pill, keeping its place.
        let k = r.h / CHIP_H;
        let head = Rect {
            x: r.x + CHIP_GAP * k,
            y: r.y + 0.5 * (r.h - CHIP_FACE * k),
            w: CHIP_FACE * k,
            h: CHIP_FACE * k,
        };
        match face {
            Some(t) => shapes.image_rounded(s, head, CHIP_FACE_RADIUS * k, t),
            None => shapes.rounded(s, head, CHIP_FACE_RADIUS * k, FACE_PLACEHOLDER),
        }
        let Some(atlas) = atlas else { return };
        text.line(
            atlas,
            &self.version.line,
            16.0 * s.px,
            (s.height - 16.0) * s.px,
            FAINT,
        );
        let line = Rect {
            x: head.x + head.w + CHIP_GAP * k,
            ..r
        };
        chip.text
            .left_in(text, atlas, s, line, mix(SOFT, LABEL_HOVER, lit));
    }

    /// The index of the control at `(x, y)` (UI units): a pill, or the chip.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons
            .iter()
            .position(|b| b.rect.contains(x, y))
            .or_else(|| self.chip.rect.contains(x, y).then_some(self.buttons.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn scale_follows_window_height() {
        let s = UiScale::new(1920, 1080);
        assert_eq!(s.px, 1.2);
        assert!(near((s.width, s.height), (1600.0, 900.0)));
        // Small windows bottom out at 0.75 px per unit.
        assert_eq!(UiScale::new(640, 360).px, 0.75);
    }

    #[test]
    fn title_column_is_centred_and_clickable() {
        let s = UiScale::new(1600, 900);
        let fonts = Fonts::new(&WEIGHTS);
        let t = TitleScreen::new(s, &fonts, "Ruthenium 0.0.0");
        assert_eq!(t.buttons.len(), 3);
        for b in &t.buttons {
            assert_eq!(b.rect.x + b.rect.w * 0.5, 800.0);
        }
        let first = t.buttons[0].rect;
        assert!(t.logo.y + t.logo.h < first.y);
        assert_eq!(t.logo.x + 0.5 * t.logo.w, 800.0);
        assert_eq!(t.hit(800.0, first.y + 1.0), Some(0));
        assert_eq!(t.buttons[0].action, Action::Multiplayer);
        let last = t.buttons[2].rect;
        assert_eq!(t.hit(800.0, last.y + last.h - 0.5), Some(2));
        assert_eq!(t.buttons[2].action, Action::Quit);
        assert_eq!(t.hit(10.0, 10.0), None);
        // Pills: radius is half the height, in pixels.
        let mut shapes = ShapeList::new();
        let mut text = TextList::new();
        let rest = ButtonStates::new(3);
        t.draw(&mut shapes, &mut text, None, None, None, &rest);
        let p = shapes.shapes()[0];
        assert_eq!(p.radius, 0.5 * (p.rect[3] - p.rect[1]));
        assert!(p.glass);
        assert!(text.quads().is_empty(), "no text before the atlas exists");
        // With an atlas, labels are centred in their pills.
        let mut atlas = GlyphAtlas::new(1024);
        atlas.add_text(&fonts, Weight::SEMIBOLD, "Multiplayer");
        t.draw(&mut shapes, &mut text, Some(&atlas), None, None, &rest);
        // The first label's glyphs come first (other labels share some of its letters).
        let q = &text.quads()[.."Multiplayer".len()];
        let (left, right) = (q.first().unwrap().rect[0], q.last().unwrap().rect[2]);
        let mid = 0.5 * (left + right) / s.px;
        assert!((mid - 800.0).abs() < 4.0, "label centre {mid}");
    }

    #[test]
    fn ease_is_frame_rate_independent_and_settles() {
        let (mut a, mut b) = (Ease::default(), Ease::default());
        a.step(1.0, 0.05, 0.1, 0.2);
        for _ in 0..10 {
            b.step(1.0, 0.005, 0.1, 0.2);
        }
        assert!(
            (a.value - b.value).abs() < 1e-4,
            "{} vs {}",
            a.value,
            b.value
        );
        assert!((a.value - (1.0 - (-0.5f32).exp())).abs() < 1e-4);
        // Falling uses the slower constant.
        let mut c = Ease { value: 1.0 };
        c.step(0.0, 0.05, 0.1, 0.2);
        assert!((c.value - (-0.25f32).exp()).abs() < 1e-4);
        // It reaches the target exactly.
        for _ in 0..200 {
            c.step(0.0, 0.016, 0.1, 0.2);
        }
        assert_eq!(c.value, 0.0);
    }

    fn run(states: &mut ButtonStates, seconds: f32) {
        for _ in 0..(seconds / 0.004).round() as usize {
            states.update(0.004);
        }
    }

    #[test]
    fn hover_fades_in_and_out() {
        let mut st = ButtonStates::new(3);
        st.hover(Some(1));
        st.update(0.016);
        let early = st.look(1).hover;
        assert!(early > 0.0 && early < 1.0, "{early}");
        run(&mut st, 0.5);
        assert_eq!(st.look(1).hover, 1.0);
        assert_eq!(st.look(0), ButtonLook::default());
        st.hover(None);
        st.update(0.016);
        // Letting go is slower than lighting up.
        assert!(1.0 - st.look(1).hover < early);
        run(&mut st, 1.0);
        assert_eq!(st.look(1), ButtonLook::default());
    }

    #[test]
    fn clicks_act_on_release_over_the_same_button() {
        let mut st = ButtonStates::new(3);
        st.mouse_down(Some(0));
        run(&mut st, 0.2);
        assert!(st.look(0).press > 0.9);
        // Dragging off un-presses it and does not light the other button.
        st.hover(Some(1));
        run(&mut st, 0.5);
        assert!(st.look(0).press < 0.1);
        assert_eq!(st.look(1).hover, 0.0);
        assert_eq!(st.mouse_up(Some(1)), None);
        // A quick click still shows a press, and acts.
        st.mouse_down(Some(2));
        assert_eq!(st.mouse_up(Some(2)), Some(2));
        st.update(0.03);
        assert!(st.look(2).press > 0.5, "{}", st.look(2).press);
        run(&mut st, 1.0);
        assert_eq!(st.look(2).press, 0.0);
        // A click focuses without showing the ring.
        assert_eq!(st.look(2).focus, 0.0);
    }

    #[test]
    fn keyboard_moves_focus_and_activates_on_release() {
        let mut st = ButtonStates::new(3);
        // Up from nothing starts at the bottom; focus wraps.
        st.key(Key::Previous, true);
        st.key(Key::Next, true);
        assert_eq!(st.focus, Some(0));
        run(&mut st, 0.5);
        assert_eq!(st.look(0).focus, 1.0);
        st.key(Key::Previous, true);
        assert_eq!(st.focus, Some(2));
        assert_eq!(st.key(Key::Activate, true), None);
        run(&mut st, 0.2);
        assert!(st.look(2).press > 0.9);
        assert_eq!(st.key(Key::Activate, false), Some(2));
        // Moving focus while Enter is held cancels.
        st.key(Key::Activate, true);
        st.key(Key::Next, true);
        assert_eq!(st.key(Key::Activate, false), None);
        // Enter with nothing focused only shows focus.
        let mut st = ButtonStates::new(3);
        assert_eq!(st.key(Key::Activate, true), None);
        assert_eq!(st.key(Key::Activate, false), None);
        assert_eq!(st.focus, Some(0));
        // A click after keyboard use hides the ring; cancel drops a held key.
        st.mouse_down(Some(1));
        st.mouse_up(Some(1));
        run(&mut st, 0.5);
        assert_eq!(st.look(0).focus, 0.0);
        assert_eq!(st.look(1).focus, 0.0);
        st.key(Key::Activate, true);
        st.cancel();
        assert_eq!(st.key(Key::Activate, false), None);
    }

    #[test]
    fn lit_pills_grow_and_focus_draws_a_ring() {
        let s = UiScale::new(1600, 900);
        let fonts = Fonts::new(&WEIGHTS);
        let t = TitleScreen::new(s, &fonts, "Ruthenium 0.0.0");
        let mut st = ButtonStates::new(3);
        st.hover(Some(0));
        st.key(Key::Next, true);
        st.key(Key::Next, true);
        run(&mut st, 1.0);
        let mut shapes = ShapeList::new();
        let mut text = TextList::new();
        t.draw(&mut shapes, &mut text, None, None, None, &st);
        let sh = shapes.shapes().to_vec();
        // Pill 0 (hovered), pill 1 (focused) and its ring, pill 2, the chip and the
        // placeholder for its face.
        assert_eq!(sh.len(), 6);
        assert!(sh[4].glass && !sh[5].glass && sh[5].image.is_none());
        // The chip is top left; with a face, the face is an image inside it.
        assert_eq!((sh[4].rect[0], sh[4].rect[1]), (16.0 * s.px, 16.0 * s.px));
        shapes.clear();
        t.draw(&mut shapes, &mut text, None, None, Some(9), &st);
        let face = shapes.shapes()[5];
        assert_eq!(face.image.map(|i| i.texture), Some(9));
        assert!(face.rect[0] > sh[4].rect[0] && face.rect[3] < sh[4].rect[3]);
        // As far from the pill's left edge as the text is from the face.
        let inset = face.rect[0] - sh[4].rect[0];
        assert!((inset - CHIP_GAP * s.px).abs() < 1e-3);
        assert!(
            (t.chip.rect.x + CHIP_GAP + CHIP_FACE + CHIP_GAP) * s.px - face.rect[2] - inset < 1e-3
        );
        assert_eq!(t.hit(20.0, 20.0), Some(3));
        assert_eq!(t.action(3), Some(Action::Accounts));
        let width = |s: &Shape| s.rect[2] - s.rect[0];
        assert!(width(&sh[0]) > width(&sh[3]));
        assert!(sh[2].ring > 0.0 && !sh[2].glass);
        assert!(sh[2].rect[0] < sh[1].rect[0] && sh[2].rect[3] > sh[1].rect[3]);
        assert!((sh[2].radius - 0.5 * (sh[2].rect[3] - sh[2].rect[1])).abs() < 1e-3);
        assert!(sh[0].fill[3] > sh[3].fill[3] && sh[1].fill[3] > sh[3].fill[3]);
    }

    #[test]
    fn window_points_to_ui_units() {
        let s = UiScale::new(1920, 1080);
        // HiDPI: 960 x 540 points over 1920 x 1080 pixels.
        assert!(near(
            s.from_window(480.0, 270.0, (960, 540), (1920, 1080)),
            (800.0, 450.0)
        ));
    }
}
