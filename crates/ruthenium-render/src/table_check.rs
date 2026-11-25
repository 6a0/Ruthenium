//! A development check of the texture table (`--exercise-textures`), for whichever kind of
//! descriptor set and size it was forced to (`--texture-table`, `--texture-slots`):
//!
//! - it fills the table with small textures of known colours until creating one is
//!   refused, holds them, then destroys a random half (every third time, all of them)
//!   and fills up again, as a resource-pack reload does. Creations that are refused
//!   while destroyed textures still hold their slots are tried again in later frames;
//! - every frame it can, it draws one pixel per slot with the GUI pipeline, a batch of
//!   quads that each sample a different texture, into an off-screen target, reads the
//!   target back and compares: a texture of its own must show its colour, a slot showing
//!   no texture the missing texture, also while the table is full and while textures
//!   retire. Slots of other owners (the menus) are drawn but not compared.
//!
//! The sequence is the same on both paths, so the same shader reads the same descriptor
//! contents through ordinary and through update-after-bind sets.

use std::sync::Arc;

use ruthenium_gpu::{
    Arrival, Encoder, Error, FrameData, Gpu, Pipelines, Readback, TextureDesc, TextureId,
    TextureTable, UploadError, UploadRing, vk,
};
use ruthenium_ui::Quad;

use crate::check_pass;

/// Side of the target, in pixels: one grid shows this many slots squared.
const GRID: u32 = 64;
const WINDOW: u32 = GRID * GRID;
/// Side of the check's textures, in texels.
const TEXTURE_SIZE: u32 = 4;
const TEXTURE_BYTES: usize = (TEXTURE_SIZE * TEXTURE_SIZE * 4) as usize;
/// Textures created per frame while filling.
const CREATES_PER_FRAME: usize = 32;
/// Frames a full table is held before the next reload.
const HOLD_FRAMES: u32 = 6;
/// Frames creations may stay refused while slots are being freed.
const STUCK_FRAMES: u32 = 240;
/// The most textures the check creates: each is a memory allocation of its own, and
/// drivers limit those. A larger table is exercised but never filled.
const MAX_OWNED: usize = 2048;
/// Mismatches printed in detail; the rest are only counted.
const PRINTED: u64 = 8;
/// In `expected`: a slot that is not compared. No texture of the check is transparent.
const SKIP: u32 = 0;

#[derive(Clone, Copy, Debug)]
enum Phase {
    Fill,
    /// Frames left until the reload.
    Hold(u32),
}

/// What the check has seen so far.
#[derive(Clone, Copy, Debug, Default)]
pub struct TableCheckReport {
    /// Grids read back, and slots compared in them.
    pub grids: u64,
    pub slots: u64,
    /// Slots that showed something else than they should.
    pub mismatches: u64,
    pub created: u64,
    /// Times a creation was refused with every slot taken and none being freed.
    pub exhausted: u64,
    pub reloads: u64,
    /// Creations refused while destroyed textures still held slots, tried again later.
    pub deferred: u64,
    /// Times creations stayed refused for [`STUCK_FRAMES`] though slots were being freed.
    pub stuck: u64,
    /// The table has more slots than the check fills, so it was never exhausted.
    pub capped: bool,
}

impl TableCheckReport {
    /// Whether the run shows the table working: grids compared without a mismatch, and
    /// the table filled and refilled (unless it is larger than the check fills).
    pub fn passed(&self) -> bool {
        self.grids > 0
            && self.mismatches == 0
            && self.stuck == 0
            && self.reloads > 0
            && (self.capped || (self.exhausted > 0 && self.deferred > 0))
    }
}

/// See the module docs.
#[derive(Debug)]
pub(crate) struct TableCheck {
    gpu: Arc<Gpu>,
    target: Readback,
    /// Per slot: the colour of the check's texture there (RGBA, lowest byte first), or 0.
    colours: Vec<u32>,
    owned: Vec<TextureId>,
    /// What the grid on its way back must show, per slot from `window_first` on.
    expected: Vec<u32>,
    window_first: u32,
    quads: Vec<Quad>,
    rng: u64,
    phase: Phase,
    /// Frames in a row in which a creation was refused while slots were being freed.
    waiting: u32,
    limit: usize,
    report: TableCheckReport,
}

impl TableCheck {
    pub fn new(gpu: Arc<Gpu>, format: vk::Format, capacity: u32) -> Result<TableCheck, Error> {
        let target = Readback::new(gpu.clone(), format, grid_extent())?;
        let limit = MAX_OWNED.min(gpu.caps().max_memory_allocations as usize / 4);
        Ok(TableCheck {
            gpu,
            target,
            colours: vec![0; capacity as usize],
            owned: Vec::with_capacity(limit.min(capacity as usize)),
            expected: Vec::with_capacity(WINDOW as usize),
            window_first: 0,
            quads: Vec::with_capacity(WINDOW as usize),
            rng: 0x2545_F491_4F6C_DD1D,
            phase: Phase::Fill,
            waiting: 0,
            limit,
            report: TableCheckReport::default(),
        })
    }

    pub fn report(&self) -> TableCheckReport {
        self.report
    }

    fn next(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// One frame of the check: compares the grid that has come back, draws the next one,
    /// then creates or destroys textures (visible from the next frame on, so the grid
    /// just drawn is of the table as this frame's set shows it). `completed` is a frame
    /// the GPU has finished; call it after `textures.begin_frame` for the frame `encoder`
    /// records.
    pub fn step(
        &mut self,
        encoder: &mut Encoder<'_>,
        completed: u64,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
        frame_data: &mut FrameData,
        pipelines: &Pipelines,
    ) -> Result<(), Error> {
        profiling::scope!("table_check");
        self.compare(completed, textures.capacity());
        if self.target.format() != pipelines.format() {
            // The window's format changed and the pipelines with it. A grid on its way
            // back is dropped with the old target (which waits for it).
            self.target = Readback::new(self.gpu.clone(), pipelines.format(), grid_extent())?;
        }
        if !self.target.pending() {
            self.draw(encoder, textures, frame_data, pipelines);
        }
        match self.phase {
            Phase::Fill => self.fill(textures, uploads)?,
            Phase::Hold(0) => self.reload(textures),
            Phase::Hold(n) => self.phase = Phase::Hold(n - 1),
        }
        Ok(())
    }

    fn compare(&mut self, completed: u64, capacity: u32) {
        let Some((frame, pixels)) = self.target.take(completed) else {
            return;
        };
        for (i, (&want, got)) in self
            .expected
            .iter()
            .zip(pixels.as_chunks::<4>().0)
            .enumerate()
        {
            if want == SKIP {
                continue;
            }
            self.report.slots += 1;
            if u32::from_le_bytes(*got) != want {
                if self.report.mismatches < PRINTED {
                    eprintln!(
                        "[table-check] slot {} in frame {frame} shows {got:02x?}, not {:02x?}",
                        self.window_first as usize + i,
                        want.to_le_bytes()
                    );
                }
                self.report.mismatches += 1;
            }
        }
        self.report.grids += 1;
        // A table larger than one grid is covered a window at a time.
        self.window_first = match self.window_first + WINDOW {
            next if next < capacity => next,
            _ => 0,
        };
    }

    /// Draws one pixel per slot of the window and records the copy back. No copy is
    /// pending.
    fn draw(
        &mut self,
        encoder: &mut Encoder<'_>,
        textures: &TextureTable,
        frame_data: &mut FrameData,
        pipelines: &Pipelines,
    ) {
        let count = WINDOW.min(textures.capacity() - self.window_first);
        let missing = u32::from_le_bytes(TextureTable::MISSING_COLORS[0]);
        self.quads.clear();
        self.expected.clear();
        for i in 0..count {
            let slot = self.window_first + i;
            let (x, y) = ((i % GRID) as f32, (i / GRID) as f32);
            self.quads.push(Quad {
                rect: [x, y, x + 1.0, y + 1.0],
                // One texel: the first colour of the missing texture's checkerboard.
                uv: [0.25; 4],
                color: 0xFFFF_FFFF,
                texture: slot,
                slant: [0.0; 2],
            });
            let own = self.colours[slot as usize];
            self.expected.push(match (textures.shows(slot), own) {
                (false, _) => missing,
                (true, 0) => SKIP,
                (true, colour) => colour,
            });
        }
        // Without room in the frame's data (the GUI took it), the next frame draws the grid.
        // SAFETY: every quad samples a slot below the table's capacity.
        unsafe {
            check_pass::gui_quads_into(
                encoder,
                pipelines,
                &mut self.target,
                frame_data,
                &self.quads,
            )
        };
    }

    fn fill(&mut self, textures: &mut TextureTable, uploads: &mut UploadRing) -> Result<(), Error> {
        for _ in 0..CREATES_PER_FRAME {
            if self.owned.len() >= self.limit {
                self.report.capped = true;
                self.phase = Phase::Hold(HOLD_FRAMES);
                break;
            }
            // Opaque, so never SKIP; a colour of its own, so a wrong slot shows.
            let colour = (self.next() as u32) | 0xFF00_0000;
            let texels: [u8; TEXTURE_BYTES] = std::array::from_fn(|i| colour.to_le_bytes()[i % 4]);
            let desc = TextureDesc::rgba8(TEXTURE_SIZE, TEXTURE_SIZE);
            match textures.create_with(uploads, desc, &[&texels], Arrival::NextFrame) {
                Ok(id) => {
                    self.colours[id.index() as usize] = colour;
                    self.owned.push(id);
                    self.report.created += 1;
                    self.waiting = 0;
                }
                Err(Error::TextureTableFull { retiring: 0, .. }) => {
                    self.report.exhausted += 1;
                    self.phase = Phase::Hold(HOLD_FRAMES);
                    break;
                }
                Err(Error::TextureTableFull { .. }) => {
                    // Destroyed textures still hold slots: try again once they are gone.
                    self.report.deferred += 1;
                    self.waiting += 1;
                    if self.waiting == STUCK_FRAMES {
                        eprintln!(
                            "[table-check] no slot was freed in {STUCK_FRAMES} frames: {:?}",
                            textures.stats()
                        );
                        self.report.stuck += 1;
                    }
                    break;
                }
                Err(Error::Upload(UploadError::Full)) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Destroys half of the check's textures, or all of them, and goes back to filling.
    fn reload(&mut self, textures: &mut TextureTable) {
        let all = self.report.reloads % 3 == 2;
        let mut i = 0;
        while i < self.owned.len() {
            if all || self.next().is_multiple_of(2) {
                let id = self.owned.swap_remove(i);
                self.colours[id.index() as usize] = 0;
                textures.destroy(id);
            } else {
                i += 1;
            }
        }
        self.report.reloads += 1;
        self.phase = Phase::Fill;
    }
}

fn grid_extent() -> vk::Extent2D {
    vk::Extent2D {
        width: GRID,
        height: GRID,
    }
}
