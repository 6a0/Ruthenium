//! The GUI draw list and vanilla's sprite blits
//! (`client/gui/GuiGraphicsExtractor.java` `blitSprite`, `blitNineSlicedSprite`,
//! `blitTiledSprite`; `render/state/TiledBlitRenderState.java`).

use std::collections::HashMap;

use ruthenium_assets::Id;
use ruthenium_assets::atlas::{Atlas, missing_id};
use ruthenium_assets::meta::{Border, GuiScaling};

/// [`Quad::texture`] for an untextured (filled) quad.
pub const NO_TEXTURE: u32 = u32::MAX;

/// One rectangle to draw, in GUI units, back to front in list order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// `x0, y0, x1, y1`.
    pub rect: [f32; 4],
    /// `u0, v0, u1, v1` in the texture.
    pub uv: [f32; 4],
    /// `0xAARRGGBB`, multiplied with the texture.
    pub color: u32,
    /// The renderer's texture index, or [`NO_TEXTURE`].
    pub texture: u32,
}

/// Where one GUI sprite is and how it scales.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteInfo {
    /// `u0, v0, u1, v1` in the atlas.
    pub uv: [f32; 4],
    pub width: u32,
    pub height: u32,
    pub scaling: GuiScaling,
}

impl SpriteInfo {
    /// `TextureAtlasSprite.getU`: `u0 + (u1 - u0) * f`.
    fn u(&self, f: f32) -> f32 {
        self.uv[0] + (self.uv[2] - self.uv[0]) * f
    }

    fn v(&self, f: f32) -> f32 {
        self.uv[1] + (self.uv[3] - self.uv[1]) * f
    }

    /// UVs of texels `[tx, tx + w) x [ty, ty + h)` of a sprite `sw` x `sh` large.
    fn sub_uv(&self, sw: i32, sh: i32, tx: i32, ty: i32, w: i32, h: i32) -> [f32; 4] {
        [
            self.u(tx as f32 / sw as f32),
            self.v(ty as f32 / sh as f32),
            self.u((tx + w) as f32 / sw as f32),
            self.v((ty + h) as f32 / sh as f32),
        ]
    }
}

/// The GUI atlas's sprites, with the texture index the renderer gave it.
#[derive(Debug, Clone)]
pub struct Sprites {
    texture: u32,
    map: HashMap<Id, SpriteInfo>,
    missing: SpriteInfo,
}

impl Sprites {
    pub fn from_atlas(atlas: &Atlas, texture: u32) -> Sprites {
        let info = |s: &ruthenium_assets::atlas::Sprite| SpriteInfo {
            uv: s.uv(atlas.width, atlas.height),
            width: s.width,
            height: s.height,
            scaling: s.gui.unwrap_or_default(),
        };
        let map: HashMap<Id, SpriteInfo> = atlas
            .sprites
            .iter()
            .map(|(id, s)| (id.clone(), info(s)))
            .collect();
        let missing = map[&missing_id()];
        Sprites {
            texture,
            map,
            missing,
        }
    }

    /// A sprite, or the missing sprite as vanilla substitutes it.
    pub fn get(&self, id: &Id) -> &SpriteInfo {
        self.map.get(id).unwrap_or(&self.missing)
    }

    pub fn texture(&self) -> u32 {
        self.texture
    }
}

/// Quads for one frame of GUI.
#[derive(Debug, Default)]
pub struct DrawList {
    quads: Vec<Quad>,
}

impl DrawList {
    pub fn new() -> DrawList {
        DrawList {
            quads: Vec::with_capacity(1024),
        }
    }

    pub fn clear(&mut self) {
        self.quads.clear();
    }

    pub fn quads(&self) -> &[Quad] {
        &self.quads
    }

    #[allow(clippy::too_many_arguments)]
    fn push(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, uv: [f32; 4], color: u32, texture: u32) {
        self.quads.push(Quad {
            rect: [x0 as f32, y0 as f32, x1 as f32, y1 as f32],
            uv,
            color,
            texture,
        });
    }

    /// A solid rectangle between two corners, in either order.
    pub fn fill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        self.push(
            x0.min(x1),
            y0.min(y1),
            x0.max(x1),
            y0.max(y1),
            [0.0; 4],
            color,
            NO_TEXTURE,
        );
    }

    /// Draws a GUI sprite over `(x, y, w, h)` the way its `gui` metadata says.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_sprite(
        &mut self,
        sprites: &Sprites,
        id: &Id,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    ) {
        self.blit(sprites.get(id), sprites.texture(), x, y, w, h, color);
    }

    /// As [`Self::blit_sprite`], for a sprite looked up before, of the texture `t`.
    #[allow(clippy::too_many_arguments)]
    pub fn blit(&mut self, s: &SpriteInfo, t: u32, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let s = *s;
        match s.scaling {
            GuiScaling::Stretch => {
                if w != 0 && h != 0 {
                    self.push(x, y, x + w, y + h, s.uv, color, t);
                }
            }
            GuiScaling::Tile { width, height } => {
                let (tw, th) = (width as i32, height as i32);
                self.tiled(&s, t, x, y, w, h, [0, 0, tw, th], tw, th, color);
            }
            GuiScaling::NineSlice {
                width,
                height,
                border,
                stretch_inner,
            } => {
                self.nine_slice(
                    &s,
                    t,
                    width as i32,
                    height as i32,
                    border,
                    stretch_inner,
                    x,
                    y,
                    w,
                    h,
                    color,
                );
            }
        }
    }

    /// A part of a sprite (`blitSprite` with a sprite size and a texel origin): texels
    /// `[tx, tx + w) x [ty, ty + h)` of a sprite treated as `sw` x `sh`, at `(x, y)`.
    /// Only sprites that stretch are drawn so; vanilla clips the others, which nothing
    /// here does yet.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_part(
        &mut self,
        s: &SpriteInfo,
        t: u32,
        [sw, sh]: [i32; 2],
        [tx, ty]: [i32; 2],
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    ) {
        self.sub(s, t, sw, sh, tx, ty, x, y, w, h, color);
    }

    /// Texels `[tx, tx + w) x [ty, ty + h)` of a sprite treated as `sw` x `sh`, at `(x, y)`.
    #[allow(clippy::too_many_arguments)]
    fn sub(
        &mut self,
        s: &SpriteInfo,
        t: u32,
        sw: i32,
        sh: i32,
        tx: i32,
        ty: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    ) {
        if w != 0 && h != 0 {
            self.push(x, y, x + w, y + h, s.sub_uv(sw, sh, tx, ty, w, h), color, t);
        }
    }

    /// Repeats texels `tex = [tx, ty, tw, th]` of a sprite treated as `sw` x `sh` over
    /// `(x, y, w, h)`, clipping the last row and column.
    #[allow(clippy::too_many_arguments)]
    fn tiled(
        &mut self,
        s: &SpriteInfo,
        t: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        tex: [i32; 4],
        sw: i32,
        sh: i32,
        color: u32,
    ) {
        let [tx, ty, tw, th] = tex;
        if w <= 0 || h <= 0 || tw <= 0 || th <= 0 {
            return;
        }
        let [u0, v0, u1, v1] = s.sub_uv(sw, sh, tx, ty, tw, th);
        let lerp = |f: f32, a: f32, b: f32| a + f * (b - a);
        let mut ox = 0;
        while ox < w {
            let rest = w - ox;
            let (cw, cu1) = if tw <= rest {
                (tw, u1)
            } else {
                (rest, lerp(rest as f32 / tw as f32, u0, u1))
            };
            let mut oy = 0;
            while oy < h {
                let rest = h - oy;
                let (ch, cv1) = if th <= rest {
                    (th, v1)
                } else {
                    (rest, lerp(rest as f32 / th as f32, v0, v1))
                };
                self.push(
                    x + ox,
                    y + oy,
                    x + ox + cw,
                    y + oy + ch,
                    [u0, v0, cu1, cv1],
                    color,
                    t,
                );
                oy += th;
            }
            ox += tw;
        }
    }

    /// An edge or the centre of a nine-slice: stretched or tiled.
    #[allow(clippy::too_many_arguments)]
    fn inner(
        &mut self,
        s: &SpriteInfo,
        t: u32,
        stretch: bool,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        tex: [i32; 4],
        sw: i32,
        sh: i32,
        color: u32,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        if stretch {
            let [tx, ty, tw, th] = tex;
            self.push(
                x,
                y,
                x + w,
                y + h,
                s.sub_uv(sw, sh, tx, ty, tw, th),
                color,
                t,
            );
        } else {
            self.tiled(s, t, x, y, w, h, tex, sw, sh, color);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn nine_slice(
        &mut self,
        s: &SpriteInfo,
        t: u32,
        sw: i32,
        sh: i32,
        b: Border,
        stretch: bool,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    ) {
        let bl = (b.left as i32).min(w / 2);
        let br = (b.right as i32).min(w / 2);
        let bt = (b.top as i32).min(h / 2);
        let bb = (b.bottom as i32).min(h / 2);
        let (iw, ih) = (w - br - bl, h - bb - bt);
        let (tiw, tih) = (sw - br - bl, sh - bb - bt);
        if w == sw && h == sh {
            self.sub(s, t, sw, sh, 0, 0, x, y, w, h, color);
        } else if h == sh {
            self.sub(s, t, sw, sh, 0, 0, x, y, bl, h, color);
            self.inner(
                s,
                t,
                stretch,
                x + bl,
                y,
                iw,
                h,
                [bl, 0, tiw, sh],
                sw,
                sh,
                color,
            );
            self.sub(s, t, sw, sh, sw - br, 0, x + w - br, y, br, h, color);
        } else if w == sw {
            self.sub(s, t, sw, sh, 0, 0, x, y, w, bt, color);
            self.inner(
                s,
                t,
                stretch,
                x,
                y + bt,
                w,
                ih,
                [0, bt, sw, tih],
                sw,
                sh,
                color,
            );
            self.sub(s, t, sw, sh, 0, sh - bb, x, y + h - bb, w, bb, color);
        } else {
            self.sub(s, t, sw, sh, 0, 0, x, y, bl, bt, color);
            self.inner(
                s,
                t,
                stretch,
                x + bl,
                y,
                iw,
                bt,
                [bl, 0, tiw, bt],
                sw,
                sh,
                color,
            );
            self.sub(s, t, sw, sh, sw - br, 0, x + w - br, y, br, bt, color);
            self.sub(s, t, sw, sh, 0, sh - bb, x, y + h - bb, bl, bb, color);
            self.inner(
                s,
                t,
                stretch,
                x + bl,
                y + h - bb,
                iw,
                bb,
                [bl, sh - bb, tiw, bb],
                sw,
                sh,
                color,
            );
            self.sub(
                s,
                t,
                sw,
                sh,
                sw - br,
                sh - bb,
                x + w - br,
                y + h - bb,
                br,
                bb,
                color,
            );
            self.inner(
                s,
                t,
                stretch,
                x,
                y + bt,
                bl,
                ih,
                [0, bt, bl, tih],
                sw,
                sh,
                color,
            );
            self.inner(
                s,
                t,
                stretch,
                x + bl,
                y + bt,
                iw,
                ih,
                [bl, bt, tiw, tih],
                sw,
                sh,
                color,
            );
            self.inner(
                s,
                t,
                stretch,
                x + w - br,
                y + bt,
                br,
                ih,
                [sw - br, bt, br, tih],
                sw,
                sh,
                color,
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A sprite filling UV 0..1, so UVs read as fractions of the sprite.
    pub(crate) fn sheet(scaling: GuiScaling, w: u32, h: u32) -> Sprites {
        let info = SpriteInfo {
            uv: [0.0, 0.0, 1.0, 1.0],
            width: w,
            height: h,
            scaling,
        };
        let mut map = HashMap::new();
        map.insert(Id::minecraft("s").unwrap(), info);
        Sprites {
            texture: 7,
            map,
            missing: SpriteInfo {
                scaling: GuiScaling::Stretch,
                ..info
            },
        }
    }

    fn rects(dl: &DrawList) -> Vec<[i32; 4]> {
        dl.quads()
            .iter()
            .map(|q| q.rect.map(|v| v as i32))
            .collect()
    }

    #[test]
    fn stretch_is_one_quad() {
        let s = sheet(GuiScaling::Stretch, 16, 16);
        let mut dl = DrawList::new();
        dl.blit_sprite(&s, &Id::minecraft("s").unwrap(), 10, 20, 30, 40, !0);
        assert_eq!(rects(&dl), [[10, 20, 40, 60]]);
        assert_eq!(dl.quads()[0].uv, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(dl.quads()[0].texture, 7);
        // Zero size draws nothing; unknown ids draw the missing sprite.
        dl.blit_sprite(&s, &Id::minecraft("s").unwrap(), 0, 0, 0, 5, !0);
        dl.blit_sprite(&s, &Id::minecraft("nope").unwrap(), 0, 0, 5, 5, !0);
        assert_eq!(dl.quads().len(), 2);
    }

    #[test]
    fn tiles_clip_the_last_row_and_column() {
        let s = sheet(
            GuiScaling::Tile {
                width: 8,
                height: 8,
            },
            8,
            8,
        );
        let mut dl = DrawList::new();
        dl.blit_sprite(&s, &Id::minecraft("s").unwrap(), 0, 0, 20, 10, !0);
        // Columns first (x outer, y inner).
        assert_eq!(
            rects(&dl),
            [
                [0, 0, 8, 8],
                [0, 8, 8, 10],
                [8, 0, 16, 8],
                [8, 8, 16, 10],
                [16, 0, 20, 8],
                [16, 8, 20, 10]
            ]
        );
        assert_eq!(dl.quads()[5].uv, [0.0, 0.0, 0.5, 0.25]);
    }

    #[test]
    fn button_nine_slice() {
        // widget/button: 200 x 20, border 3 (its gui metadata in 26.3 is a fact we test
        // against only through the shape here).
        let b = Border {
            left: 3,
            top: 3,
            right: 3,
            bottom: 3,
        };
        let s = sheet(
            GuiScaling::NineSlice {
                width: 200,
                height: 20,
                border: b,
                stretch_inner: false,
            },
            200,
            20,
        );
        let id = Id::minecraft("s").unwrap();
        let mut dl = DrawList::new();
        // Same size: the whole sprite once.
        dl.blit_sprite(&s, &id, 0, 0, 200, 20, !0);
        assert_eq!(rects(&dl), [[0, 0, 200, 20]]);
        // Same height, narrower: left cap, tiled middle (one 194-wide tile, clipped), right cap.
        dl.clear();
        dl.blit_sprite(&s, &id, 0, 0, 98, 20, !0);
        assert_eq!(rects(&dl), [[0, 0, 3, 20], [3, 0, 95, 20], [95, 0, 98, 20]]);
        let mid = dl.quads()[1].uv;
        assert_eq!(mid[0], 3.0 / 200.0);
        assert_eq!(
            mid[2],
            3.0 / 200.0 + (92.0 / 194.0) * (197.0 / 200.0 - 3.0 / 200.0)
        );
        assert_eq!(dl.quads()[2].uv[0], 197.0 / 200.0);
        // Both different: nine pieces in vanilla's order.
        dl.clear();
        dl.blit_sprite(&s, &id, 0, 0, 20, 21, !0);
        assert_eq!(
            rects(&dl)[..3],
            [[0, 0, 3, 3], [3, 0, 17, 3], [17, 0, 20, 3]]
        );
        // Corners, top and bottom edges once each; the left edge, centre and right edge
        // are 15 tall from 14-tall tiles, so two quads each.
        assert_eq!(dl.quads().len(), 12);
        // Small sizes clamp the borders to half the size.
        dl.clear();
        dl.blit_sprite(&s, &id, 0, 0, 4, 4, !0);
        assert_eq!(
            rects(&dl),
            [[0, 0, 2, 2], [2, 0, 4, 2], [0, 2, 2, 4], [2, 2, 4, 4]]
        );
    }

    #[test]
    fn fill_normalises_corners() {
        let mut dl = DrawList::new();
        dl.fill(10, 10, 0, 5, 0xFF00FF00);
        assert_eq!(rects(&dl), [[0, 5, 10, 10]]);
        assert_eq!(dl.quads()[0].texture, NO_TEXTURE);
    }
}
