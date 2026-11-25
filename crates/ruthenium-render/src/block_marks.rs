//! What is drawn on single blocks: the outline of the block the crosshair is on, and the
//! cracks in the blocks being broken (`LevelRenderer.submitBlockOutline` and
//! `submitBlockDestroyAnimation`, `ShapeOutlineFeatureRenderer`,
//! `SheetedDecalTextureGenerator`).

use ruthenium_assets::model::{BakedQuad, Layer, ModelPart};
use ruthenium_gpu::{Arrival, FrameData, Pass, TextureDesc, TextureId, TextureTable, UploadRing};
use ruthenium_prepare::exporters::ModelFace;
use ruthenium_shaders::interface::{CRACK, LINE};
use ruthenium_sim::blocks::{offset as block_offset, seed};
use ruthenium_sim::scene::{Frame, Terrain};

use crate::atmosphere::Fog;
use crate::block_entity::affine::Affine;
use crate::block_entity::renderer::{MODEL_PUSH_BYTES, model_push};
use crate::pipelines::PipelineId;
use crate::sky::SkyImage;
use crate::terrain::View;
use crate::terrain::table::{BlockTable, StateInfo};
use crate::terrain::world::write_world;
use crate::terrain_vertex::{position_code, position_of};

/// How many stages of breaking there are, each with a picture of its own.
pub const STAGES: usize = 10;
/// The outline's colour: black, two fifths opaque (red in the low byte).
const OUTLINE: u32 = 102 << 24;
/// A block's cracks are drawn as far as this from the camera, in blocks, squared.
const CRACKS_WITHIN: f64 = 1024.0;
/// Quads of cracks drawn in a frame at most.
const MAX_CRACKS: usize = 4096;
/// Bytes of the frame's data left for what is drawn after these.
const RESERVED: u64 = 256 << 10;

/// How wide the outline's lines are, in pixels, on a target so wide
/// (`Window.getAppropriateLineWidth`).
pub(crate) fn line_width(target_width: f32) -> f32 {
    (target_width / 1920.0 * 2.5).max(2.5)
}

/// Where the cracks' picture is read for a point of a face that looks towards
/// `direction` (a `Direction` ordinal), `at` in blocks from the block's corner
/// (`SheetedDecalTextureGenerator` at a scale of one: the picture lies once over each
/// face of a whole block, and repeats beyond it).
pub(crate) fn crack_uv(direction: u8, [x, y, z]: [f32; 3]) -> [f32; 2] {
    match direction {
        // Down, up.
        0 => [x, -z],
        1 => [x, z],
        // North, south.
        2 => [-x, -y],
        3 => [x, -y],
        // West, east.
        4 => [-z, -y],
        _ => [z, -y],
    }
}

/// When in a frame something of this is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum When {
    /// With what is solid: the cracks in blocks that are not seen through.
    Solid,
    /// Before the terrain that is seen through: the outline of a block that is not, and
    /// the cracks in those that are.
    BeforeTranslucent,
    /// After it: the outline of a block that is seen through.
    AfterTranslucent,
}

/// What the last frame drew.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MarkStats {
    /// Edges of the outline.
    pub lines: u32,
    /// Quads of cracks, and the blocks they are on.
    pub cracks: u32,
    pub cracked: u32,
}

/// One quad of cracks: its stage, then its record's words.
type Crack = (u8, [f32; 20]);

/// See the module docs.
#[derive(Debug)]
pub(crate) struct BlockMarks {
    /// The stages' pictures.
    stages: [Option<TextureId>; STAGES],
    show_outline: bool,
    /// The outline's edges, each from the camera, and whether its block is seen through.
    lines: Vec<[[f32; 3]; 2]>,
    outline_translucent: bool,
    width: f32,
    /// The cracks in blocks that are not seen through, and in those that are.
    cracks: [Vec<Crack>; 2],
    /// The frame's world record and matrix, once something was drawn.
    records: Option<(u32, u32)>,
    stats: MarkStats,
}

impl BlockMarks {
    /// Puts the stages' pictures on the GPU: `pictures` in their order, as far as there
    /// are any.
    pub fn new(
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
        pictures: &[SkyImage],
    ) -> BlockMarks {
        let mut stages = [None; STAGES];
        for (stage, i) in stages.iter_mut().zip(pictures) {
            if i.rgba.is_empty() || i.rgba.len() != i.width as usize * i.height as usize * 4 {
                continue;
            }
            *stage = textures
                .create_with(
                    uploads,
                    TextureDesc::rgba8(i.width, i.height),
                    &[&i.rgba],
                    Arrival::NextFrame,
                )
                .ok();
        }
        BlockMarks {
            stages,
            show_outline: true,
            lines: Vec::new(),
            outline_translucent: false,
            width: 2.5,
            cracks: [Vec::new(), Vec::new()],
            records: None,
            stats: MarkStats::default(),
        }
    }

    pub fn set_outline(&mut self, on: bool) {
        self.show_outline = on;
    }

    pub fn stats(&self) -> MarkStats {
        self.stats
    }

    /// A frame with nothing to draw.
    pub fn gather_nothing(&mut self) {
        self.lines.clear();
        self.cracks.iter_mut().for_each(Vec::clear);
        self.records = None;
        self.stats = MarkStats::default();
    }

    /// Gathers the frame's outline and cracks, as seen from `camera` by `view` on a
    /// target `target_width` pixels wide.
    pub fn gather(
        &mut self,
        frame: &Frame,
        terrain: &Terrain,
        table: &BlockTable,
        camera: [f64; 3],
        view: &View,
        target_width: f32,
    ) {
        self.gather_nothing();
        self.width = line_width(target_width);
        let mut parts: Vec<&ModelPart> = Vec::new();
        if let Some(target) = frame.player.target.as_ref().filter(|_| self.show_outline) {
            let from: [f32; 3] =
                std::array::from_fn(|i| (f64::from(target.pos[i]) - camera[i]) as f32);
            self.lines.extend(
                target
                    .edges
                    .iter()
                    .map(|ends| ends.map(|p| std::array::from_fn(|i| from[i] + p[i]))),
            );
            parts_of(table, target.state, target.pos, &mut parts);
            self.outline_translucent = is_translucent(&parts);
        }
        for &(pos, stage) in &frame.breaking {
            let away: f64 = (0..3)
                .map(|i| (f64::from(pos[i]) + 0.5 - camera[i]).powi(2))
                .sum();
            if away > CRACKS_WITHIN || usize::from(stage) >= STAGES {
                continue;
            }
            let Some(state) = terrain.probe(pos).map(|p| u32::from(p.block)) else {
                continue;
            };
            let info = table.state(state);
            if info.flags & StateInfo::DRAWN == 0 {
                continue;
            }
            parts_of(table, state, pos, &mut parts);
            // Where the block is in its section, as the mesher has it, and the
            // section's corner from the camera's block.
            let mut base: [f32; 3] = pos.map(|c| (c & 15) as f32);
            if info.offset != 0 {
                let o = table.offsets[usize::from(info.offset)];
                let d = block_offset(pos, o.vertical, o.max_horizontal, o.max_vertical);
                for a in 0..3 {
                    base[a] += d[a] as f32;
                }
            }
            let origin: [f32; 3] =
                std::array::from_fn(|i| (((pos[i] >> 4) << 4) - view.block[i]) as f32);
            let into = &mut self.cracks[usize::from(is_translucent(&parts))];
            let before = into.len();
            for quad in parts.iter().flat_map(|p| p.all()) {
                if into.len() >= MAX_CRACKS {
                    break;
                }
                if let Some(words) = crack_words(quad, base, origin) {
                    into.push((stage, words));
                }
            }
            self.stats.cracks += (into.len() - before) as u32;
            self.stats.cracked += u32::from(into.len() > before);
        }
        self.stats.lines = self.lines.len() as u32;
    }

    /// Projects the decal in block space after the part (for example a chest lid) has moved.
    pub fn add_model_block<'a>(
        &mut self,
        stage: u8,
        parts: impl Iterator<Item = (bool, Affine, &'a [ModelFace])>,
        origin: [f32; 3],
        view: &View,
    ) {
        if usize::from(stage) >= STAGES {
            return;
        }
        let mut count = 0;
        for (translucent, transform, faces) in parts {
            let into = &mut self.cracks[usize::from(translucent)];
            let faces = &faces[..faces.len().min(MAX_CRACKS.saturating_sub(into.len()))];
            into.extend(
                faces
                    .iter()
                    .map(|face| (stage, model_crack(face, transform, origin, view.offset))),
            );
            count += faces.len() as u32;
        }
        self.stats.cracks += count;
        self.stats.cracked += u32::from(count > 0);
    }

    /// Draws what of the gathered is drawn `when`, into `pass`, in `fog`; `size` is the
    /// target's, in pixels.
    ///
    /// # Safety
    /// `pass` is of the frame being recorded, whose `UploadRing::record` has run, and has
    /// a depth attachment.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn draw(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        textures: &TextureTable,
        view: &View,
        fog: &Fog,
        size: [f32; 2],
        when: When,
    ) {
        let (cracks, outline) = match when {
            When::Solid => (Some(0), false),
            When::BeforeTranslucent => (Some(1), !self.outline_translucent),
            When::AfterTranslucent => (None, self.outline_translucent),
        };
        let cracks = cracks.filter(|&of| !self.cracks[of].is_empty());
        let outline = outline && !self.lines.is_empty();
        if cracks.is_none() && !outline {
            return;
        }
        let records = match self.records {
            Some(r) => r,
            None => {
                let Some(r) = write_world(frame_data, view, [1.0; 2], TextureId::MISSING) else {
                    return;
                };
                self.records = Some(r);
                r
            }
        };
        let room = |frame_data: &FrameData, bytes: u32| {
            ((frame_data.capacity() - frame_data.used()).saturating_sub(RESERVED)
                / u64::from(bytes)) as usize
        };
        if let Some(of) = cracks {
            let list = &mut self.cracks[of];
            // One draw per stage across terrain and block entities.
            list.sort_unstable_by_key(|c| c.0);
            let count = list.len().min(room(frame_data, CRACK.bytes()));
            if let Some((first, bytes)) = frame_data
                .alloc(count as u32 * CRACK.words(), 4)
                .filter(|_| count > 0)
            {
                for ((_, words), record) in list.iter().zip(bytes.as_chunks_mut::<80>().0) {
                    for (word, out) in words.iter().zip(record.as_chunks_mut::<4>().0) {
                        *out = word.to_ne_bytes();
                    }
                }
                let mut at = 0;
                while at < count {
                    let stage = list[at].0;
                    let run = list[at..count].iter().take_while(|c| c.0 == stage).count();
                    if let Some(picture) =
                        self.stages[usize::from(stage)].filter(|t| textures.is_ready(*t))
                    {
                        let words = [
                            records.0,
                            records.1,
                            first + at as u32 * CRACK.words(),
                            picture.index(),
                        ];
                        // SAFETY: the push constants are crack.vert's and the records
                        // they point at were written just above; the picture is of the
                        // table and has arrived.
                        unsafe {
                            pass.draw(
                                PipelineId::Crack.index(),
                                &model_push(words, fog),
                                6,
                                run as u32,
                            );
                        }
                    }
                    at += run;
                }
            }
        }
        if outline {
            let count = self.lines.len().min(room(frame_data, LINE.bytes()));
            if let Some((first, bytes)) = frame_data
                .alloc(count as u32 * LINE.words(), 4)
                .filter(|_| count > 0)
            {
                for (line, record) in self.lines.iter().zip(bytes.as_chunks_mut::<32>().0) {
                    write_line(record, line, self.width, OUTLINE);
                }
                let mut push = [0u8; MODEL_PUSH_BYTES + 8];
                push[..MODEL_PUSH_BYTES]
                    .copy_from_slice(&model_push([records.0, records.1, first, 1], fog));
                push[MODEL_PUSH_BYTES..MODEL_PUSH_BYTES + 4]
                    .copy_from_slice(&size[0].to_ne_bytes());
                push[MODEL_PUSH_BYTES + 4..].copy_from_slice(&size[1].to_ne_bytes());
                // SAFETY: the push constants are outline.vert's and the records they
                // point at were written just above.
                unsafe { pass.draw(PipelineId::Outline.index(), &push, 6, count as u32) };
            }
        }
    }
}

/// The parts of a state's model at a place, as the mesher picks them.
fn parts_of<'a>(table: &'a BlockTable, state: u32, pos: [i32; 3], out: &mut Vec<&'a ModelPart>) {
    out.clear();
    let s = table.state(state).seed_offset;
    table.models.parts(
        state,
        seed(
            pos[0] + i32::from(s[0]),
            pos[1] + i32::from(s[1]),
            pos[2] + i32::from(s[2]),
        ),
        out,
    );
}

/// Whether a model has anything that is seen through (`hasMaterialFlag` of the
/// translucent flag).
fn is_translucent(parts: &[&ModelPart]) -> bool {
    parts
        .iter()
        .flat_map(|p| p.all())
        .any(|q| q.layer == Layer::Translucent)
}

/// The record of a quad of cracks: its corners from the camera's block, each where the
/// terrain's own vertex is, and where the picture is read at each. `base` is the block's
/// place in its section and `origin` the section's corner from the camera's block.
/// `None` for a quad the terrain cannot hold either.
fn crack_words(quad: &BakedQuad, base: [f32; 3], origin: [f32; 3]) -> Option<[f32; 20]> {
    let mut words = [0.0; 20];
    for (n, p) in quad.positions.iter().enumerate() {
        for a in 0..3 {
            let in_section = position_of(position_code(base[a] + p[a])?);
            words[n * 3 + a] = in_section + origin[a];
        }
        let uv = crack_uv(quad.direction, *p);
        words[12 + n * 2] = uv[0];
        words[13 + n * 2] = uv[1];
    }
    Some(words)
}

/// The model's pose is relative to the camera. Its decal is projected from the block's origin.
fn model_crack(
    face: &ModelFace,
    transform: Affine,
    origin: [f32; 3],
    camera_offset: [f32; 3],
) -> [f32; 20] {
    let normal = transform.0.map(|[x, y, z, _]| {
        f64::from(x * face.normal[0] + y * face.normal[1] + z * face.normal[2])
    });
    let direction = ruthenium_sim::direction::Direction::approximate_nearest(normal).id();
    let mut words = [0.0; 20];
    for (i, [x, y, z, _, _]) in face.corners.into_iter().enumerate() {
        let point = transform.point([x / 16.0, y / 16.0, z / 16.0]);
        let local = std::array::from_fn(|a| point[a] - origin[a]);
        for a in 0..3 {
            words[3 * i + a] = point[a] - camera_offset[a];
        }
        words[12 + 2 * i..14 + 2 * i].copy_from_slice(&crack_uv(direction, local));
    }
    words
}

/// A line's record.
fn write_line(bytes: &mut [u8; 32], [from, to]: &[[f32; 3]; 2], width: f32, colour: u32) {
    let mut put = |word: u32, value: [u8; 4]| {
        let at = word as usize * 4;
        bytes[at..at + 4].copy_from_slice(&value);
    };
    for i in 0..3 {
        put(LINE.offset("from") + i, from[i as usize].to_ne_bytes());
        put(LINE.offset("to") + i, to[i as usize].to_ne_bytes());
    }
    put(LINE.offset("width"), width.to_ne_bytes());
    put(LINE.offset("colour"), colour.to_ne_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_two_and_a_half_pixels_wide_and_wider_on_wide_targets() {
        assert_eq!(line_width(854.0), 2.5);
        assert_eq!(line_width(1920.0), 2.5);
        assert_eq!(line_width(3840.0), 5.0);
    }

    #[test]
    fn crack_uvs_match_the_games_decal_generator() {
        // SheetedDecalTextureGenerator with an identity pose, unit scale, and
        // three distinct nonzero coordinates so swapped axes and sign errors show.
        let expected = [
            [0.12500006, -0.625],
            [0.12500006, 0.625],
            [-0.12500006, -0.375],
            [0.12500006, -0.375],
            [-0.625, -0.375],
            [0.625, -0.375],
        ];
        for (direction, uv) in expected.into_iter().enumerate() {
            let actual = crack_uv(direction as u8, [0.125, 0.375, 0.625]);
            for (value, want) in actual.into_iter().zip(uv) {
                assert!((value - want).abs() < 1e-6, "face {direction}: {actual:?}");
            }
        }
    }

    #[test]
    fn a_line_is_written_where_the_shader_reads_it() {
        let mut bytes = [0u8; 32];
        write_line(
            &mut bytes,
            &[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
            2.5,
            OUTLINE,
        );
        let word = |i: usize| f32::from_ne_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        assert_eq!([word(0), word(1), word(2), word(3)], [1.0, 2.0, 3.0, 2.5]);
        assert_eq!([word(4), word(5), word(6)], [4.0, 5.0, 6.0]);
        assert_eq!(bytes[28..], [0, 0, 0, 102]);
        assert_eq!((LINE.bytes(), CRACK.bytes()), (32, 80));
    }

    #[test]
    fn a_turned_model_projects_from_its_block_on_the_turned_face() {
        let face = ModelFace {
            normal: [1.0, 0.0, 0.0],
            corners: [[4.0, 8.0, 12.0, 0.0, 0.0]; 4],
        };
        let origin = [10.5, 1.0, -3.0];
        let model = Affine::IDENTITY
            .translate(origin)
            .rotate_y(std::f32::consts::FRAC_PI_2);
        let words = model_crack(&face, model, origin, [-0.5, 0.0, 0.0]);
        assert_eq!(&words[..3], &[11.75, 1.5, -3.25]);
        assert_eq!(&words[12..14], &[-0.75, -0.5]);
    }

    #[test]
    fn a_crack_is_where_the_terrains_vertex_is() {
        let quad = BakedQuad {
            positions: [
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, 1.0, 0.0],
            ],
            uvs: [[0.0; 2]; 4],
            direction: 1,
            tint_index: -1,
            shade_direction: None,
            light_emission: 0,
            layer: Layer::Solid,
            animated: false,
        };
        // The block at 3, 5, 15 of a section whose corner is 16 blocks west of the
        // camera's block.
        let words = crack_words(&quad, [3.0, 5.0, 15.0], [-16.0, 0.0, 32.0]).unwrap();
        assert_eq!(words[..3], [-13.0, 6.0, 47.0]);
        assert_eq!(words[6..9], [-12.0, 6.0, 48.0]);
        // The picture's corners are the face's.
        assert_eq!(words[12..], [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0]);
        // A corner that is no whole 2048th of a block is where the terrain rounds it to.
        let mut slanted = quad.clone();
        slanted.positions[0] = [0.1, 0.9, 0.3];
        let words = crack_words(&slanted, [0.0; 3], [0.0; 3]).unwrap();
        assert_eq!(words[0], (0.1f32 * 2048.0).round() / 2048.0);
        // Beyond what a section's vertices reach there is nothing.
        slanted.positions[0] = [40.0, 0.0, 0.0];
        assert!(crack_words(&slanted, [0.0; 3], [0.0; 3]).is_none());
    }
}
