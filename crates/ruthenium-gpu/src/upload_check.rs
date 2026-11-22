//! A development check of the upload path (`--exercise-uploads`):
//!
//! - every frame, random-sized chunks of known bytes are uploaded through the ring into a
//!   GPU-only buffer; once a chunk's ticket is ready, the frame copies it on to a
//!   host-visible buffer, and after that frame has finished the CPU compares the bytes;
//! - a texture in the table gets a background first upload, then random partial updates
//!   of all its mip levels; after each step the whole texture is read back and compared
//!   with a CPU copy. Now and then a texture is destroyed before its upload arrived.
//!
//! Run under validation (with synchronization checks) it covers the ring's wrap-around,
//! the per-frame budget, retries when full, partial image writes and, on GPUs with a copy
//! engine, the transfer queue, its timeline wait and the image ownership transfer. The
//! renderer also makes every third frame's transfer copies wait a frame
//! ([`UploadRing::force_deferral_every`]), as when the copy engine falls behind.

use std::sync::Arc;

use ash::vk;

use crate::Error;
use crate::device::Gpu;
use crate::frame::FrameCtx;
use crate::memory::{Buffer, MemoryUse};
use crate::textures::{Arrival, TextureDesc, TextureId, TextureTable};
use crate::upload::{ImageRegion, UploadError, UploadRing, UploadTicket};

/// Chunks in flight at once, each with its own range of the two buffers.
const CHUNKS: usize = 16;
/// The largest chunk; sizes are random multiples of 4 up to this. Sixteen of them are
/// twice the discrete per-frame budget, so uploads regularly have to wait a frame.
const CHUNK_BYTES: usize = 512 << 10;
/// Mismatches printed in detail; the rest are only counted.
const PRINTED: u64 = 8;
/// The checked texture: RGBA8, three mip levels.
const TEX_WIDTH: u32 = 128;
const TEX_HEIGHT: u32 = 64;
const TEX_MIPS: u32 = 3;
/// Rectangles per texture update call.
const MAX_RECTS: usize = 4;

/// Byte offset of each mip level in the texture's tightly packed copy, and the total.
const fn tex_layout() -> ([usize; TEX_MIPS as usize], usize) {
    let mut offsets = [0; TEX_MIPS as usize];
    let mut at = 0;
    let mut m = 0;
    while m < TEX_MIPS as usize {
        offsets[m] = at;
        at += ((TEX_WIDTH >> m) * (TEX_HEIGHT >> m) * 4) as usize;
        m += 1;
    }
    (offsets, at)
}
const TEX_OFFSETS: [usize; TEX_MIPS as usize] = tex_layout().0;
const TEX_BYTES: usize = tex_layout().1;

#[derive(Clone, Copy, Debug)]
enum TexState {
    /// No texture: create one next.
    None,
    /// Its first upload is staged.
    Arriving(TextureId),
    /// The frame copying it to the readback buffer.
    Reading(TextureId, u64),
    /// Compared: stage updates next.
    Updating(TextureId),
    /// Updates staged; the next frame records them, then reads the texture back.
    Staged(TextureId),
}

#[derive(Clone, Copy, Debug)]
enum State {
    Idle,
    Uploading {
        ticket: UploadTicket,
        frame: u64,
        seed: u64,
        len: usize,
    },
    Reading {
        frame: u64,
        seed: u64,
        len: usize,
    },
}

/// What the check has seen so far.
#[derive(Clone, Copy, Debug, Default)]
pub struct UploadCheckReport {
    /// Chunks read back and compared.
    pub chunks: u64,
    pub bytes: u64,
    /// Chunks whose bytes differed.
    pub mismatches: u64,
    /// Frames from staging a chunk to the first frame that could read it, summed.
    pub latency_frames: u64,
    /// Uploads the ring refused (full for the frame) and that were tried again later.
    pub retries: u64,
    /// Texture read-backs compared, and how many differed.
    pub textures: u64,
    pub texture_mismatches: u64,
    /// Rectangles written by partial updates.
    pub texture_rects: u64,
    /// Textures destroyed before their first upload arrived.
    pub orphans: u64,
}

impl UploadCheckReport {
    pub fn mean_latency_frames(&self) -> f64 {
        if self.chunks == 0 {
            0.0
        } else {
            self.latency_frames as f64 / self.chunks as f64
        }
    }
}

/// See the module docs.
pub struct UploadCheck {
    gpu: Arc<Gpu>,
    target: Option<Buffer>,
    readback: Option<Buffer>,
    chunks: [State; CHUNKS],
    rng: u64,
    scratch: Vec<u8>,
    report: UploadCheckReport,
    tex: TexState,
    /// What the texture should hold, mip levels one after another.
    tex_mirror: Vec<u8>,
    /// Update data, rectangles one after another.
    tex_scratch: Vec<u8>,
    tex_readback: Option<Buffer>,
}

impl std::fmt::Debug for UploadCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadCheck")
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}

// SAFETY: the readback mapping is only read through &mut self; handles may move threads.
unsafe impl Send for UploadCheck {}

impl UploadCheck {
    pub fn new(gpu: Arc<Gpu>) -> Result<UploadCheck, Error> {
        let size = (CHUNKS * CHUNK_BYTES) as u64;
        let target = Buffer::new(
            &gpu,
            size,
            vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::TRANSFER_SRC,
            MemoryUse::GpuOnly,
        )?;
        let readback = match Buffer::new(
            &gpu,
            size,
            vk::BufferUsageFlags::TRANSFER_DST,
            MemoryUse::Readback,
        ) {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: unused.
                unsafe { target.destroy(&gpu) };
                return Err(e);
            }
        };
        assert!(
            !readback.memory.mapped.is_null(),
            "readback memory is host-visible"
        );
        let tex_readback = match Buffer::new(
            &gpu,
            TEX_BYTES as u64,
            vk::BufferUsageFlags::TRANSFER_DST,
            MemoryUse::Readback,
        ) {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: unused.
                unsafe {
                    target.destroy(&gpu);
                    readback.destroy(&gpu);
                }
                return Err(e);
            }
        };
        Ok(UploadCheck {
            gpu,
            target: Some(target),
            readback: Some(readback),
            chunks: [State::Idle; CHUNKS],
            rng: 0x9E37_79B9_7F4A_7C15,
            scratch: vec![0; CHUNK_BYTES],
            report: UploadCheckReport::default(),
            tex: TexState::None,
            tex_mirror: vec![0; TEX_BYTES],
            tex_scratch: vec![0; TEX_BYTES],
            tex_readback: Some(tex_readback),
        })
    }

    pub fn report(&self) -> UploadCheckReport {
        self.report
    }

    fn next(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Fills `self.scratch[..len]` with the bytes of `seed`.
    fn pattern(&mut self, seed: u64, len: usize) {
        let mut x = seed | 1;
        for word in self.scratch[..len].as_chunks_mut::<8>().0 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *word = x.to_le_bytes();
        }
        let tail = len - len % 8;
        x ^= x << 13;
        self.scratch[tail..len].copy_from_slice(&x.to_le_bytes()[..len - tail]);
    }

    /// One frame of the check. Call it after `ring.record(ctx, …)` and before the frame's
    /// passes.
    ///
    /// # Safety
    /// `ctx.cmd` is recording frame `ctx.number` on the graphics queue; `completed` is a
    /// frame the GPU has finished; `ring` is the ring recorded into this frame.
    pub unsafe fn step(
        &mut self,
        ring: &mut UploadRing,
        table: &mut TextureTable,
        ctx: &FrameCtx,
        completed: u64,
    ) -> Result<(), Error> {
        profiling::scope!("upload_check");
        // SAFETY: forwarded from the caller.
        unsafe { self.step_texture(ring, table, ctx, completed)? };
        let target = self.target.as_ref().expect("live").raw;
        let readback = self.readback.as_ref().expect("live").raw;
        let mapped = self.readback.as_ref().expect("live").memory.mapped;
        let mut copies = [vk::BufferCopy::default(); CHUNKS];
        let mut n = 0;
        for i in 0..CHUNKS {
            let offset = i * CHUNK_BYTES;
            match self.chunks[i] {
                State::Reading { frame, seed, len } if frame <= completed => {
                    self.pattern(seed, len);
                    // SAFETY: the copy into this range finished with its frame; the memory
                    // is host-coherent and nothing writes it until the chunk is reused.
                    let got = unsafe { std::slice::from_raw_parts(mapped.add(offset), len) };
                    if got != &self.scratch[..len] {
                        if self.report.mismatches < PRINTED {
                            let at = got.iter().zip(&self.scratch).position(|(a, b)| a != b);
                            eprintln!(
                                "[upload-check] chunk {i}: {len} bytes from frame {frame} differ (first at byte {at:?})"
                            );
                        }
                        self.report.mismatches += 1;
                    }
                    self.report.chunks += 1;
                    self.report.bytes += len as u64;
                    self.chunks[i] = State::Idle;
                }
                State::Uploading {
                    ticket,
                    frame,
                    seed,
                    len,
                } if ring.is_ready(ticket) => {
                    self.report.latency_frames += ctx.number - frame;
                    copies[n] = vk::BufferCopy {
                        src_offset: offset as u64,
                        dst_offset: offset as u64,
                        size: len as u64,
                    };
                    n += 1;
                    self.chunks[i] = State::Reading {
                        frame: ctx.number,
                        seed,
                        len,
                    };
                }
                _ => {}
            }
        }
        if n > 0 {
            let gpu = &*self.gpu;
            // The ring's own barrier (graphics path) covers shader and vertex reads, not
            // copies; on the transfer path the frame's semaphore wait covers everything.
            let before = [vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)];
            let after = [vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::HOST)
                .dst_access_mask(vk::AccessFlags2::HOST_READ)];
            // SAFETY: guaranteed by the caller; both buffers are alive and these ranges are
            // not in use by other work.
            unsafe {
                gpu.cmd_pipeline_barrier2(
                    ctx.cmd,
                    &vk::DependencyInfo::default().memory_barriers(&before),
                );
                gpu.device
                    .cmd_copy_buffer(ctx.cmd, target, readback, &copies[..n]);
                gpu.cmd_pipeline_barrier2(
                    ctx.cmd,
                    &vk::DependencyInfo::default().memory_barriers(&after),
                );
            }
        }
        // Stage new chunks into every idle range until the ring says this frame is full.
        for i in 0..CHUNKS {
            if !matches!(self.chunks[i], State::Idle) {
                continue;
            }
            let seed = self.next();
            let len = 4 * (1 + (self.next() % (CHUNK_BYTES as u64 / 4)) as usize);
            self.pattern(seed, len);
            match ring.upload_buffer(target, (i * CHUNK_BYTES) as u64, &self.scratch[..len]) {
                Ok(ticket) => {
                    self.chunks[i] = State::Uploading {
                        ticket,
                        frame: ctx.number + 1,
                        seed,
                        len,
                    }
                }
                Err(UploadError::Full) => {
                    self.report.retries += 1;
                    break;
                }
                Err(e) => unreachable!("a chunk always fits a ring: {e}"),
            }
        }
        Ok(())
    }

    /// The texture half of [`Self::step`].
    ///
    /// # Safety
    /// As [`Self::step`]; `table.begin_frame` has run for this frame.
    unsafe fn step_texture(
        &mut self,
        ring: &mut UploadRing,
        table: &mut TextureTable,
        ctx: &FrameCtx,
        completed: u64,
    ) -> Result<(), Error> {
        match self.tex {
            TexState::None => {
                for i in 0..TEX_BYTES / 8 {
                    let v = self.next();
                    self.tex_mirror[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
                }
                // Now and then, a texture destroyed before its upload arrives.
                let orphan = self.next().is_multiple_of(4);
                let mips: [&[u8]; TEX_MIPS as usize] = std::array::from_fn(|m| {
                    let end = TEX_OFFSETS.get(m + 1).copied().unwrap_or(TEX_BYTES);
                    &self.tex_mirror[TEX_OFFSETS[m]..end]
                });
                let desc = TextureDesc {
                    mip_levels: TEX_MIPS,
                    ..TextureDesc::rgba8(TEX_WIDTH, TEX_HEIGHT)
                };
                match table.create_with(ring, desc, &mips, Arrival::Background) {
                    Ok(id) if orphan => {
                        table.destroy(id);
                        self.report.orphans += 1;
                    }
                    Ok(id) => self.tex = TexState::Arriving(id),
                    Err(Error::Upload(UploadError::Full)) => self.report.retries += 1,
                    Err(e) => return Err(e),
                }
            }
            TexState::Arriving(id) => {
                if table.is_ready(id) {
                    // SAFETY: forwarded from the caller.
                    unsafe { self.read_texture(table, id, ctx) };
                    self.tex = TexState::Reading(id, ctx.number);
                }
            }
            TexState::Staged(id) => {
                // SAFETY: as above; this frame recorded the staged updates.
                unsafe { self.read_texture(table, id, ctx) };
                self.tex = TexState::Reading(id, ctx.number);
            }
            TexState::Reading(id, frame) if frame <= completed => {
                let mapped = self.tex_readback.as_ref().expect("live").memory.mapped;
                // SAFETY: the copy into it finished with its frame; host-coherent memory
                // nothing else writes.
                let got = unsafe { std::slice::from_raw_parts(mapped, TEX_BYTES) };
                if got != &self.tex_mirror[..] {
                    if self.report.texture_mismatches < PRINTED {
                        let at = got.iter().zip(&self.tex_mirror).position(|(a, b)| a != b);
                        eprintln!(
                            "[upload-check] texture {id:?} read back in frame {frame} differs (first at byte {at:?})"
                        );
                    }
                    self.report.texture_mismatches += 1;
                }
                self.report.textures += 1;
                if self.next().is_multiple_of(16) {
                    table.destroy(id);
                    self.tex = TexState::None;
                } else {
                    self.tex = TexState::Updating(id);
                }
            }
            TexState::Updating(id) => {
                // One or two update calls in the same frame, 1-4 rectangles each.
                let calls = 1 + (self.next() % 2) as usize;
                let mut staged = false;
                for _ in 0..calls {
                    match self.update_texture(ring, table, id) {
                        Ok(()) => staged = true,
                        Err(UploadError::Full) => {
                            self.report.retries += 1;
                            break;
                        }
                        Err(e) => unreachable!("the texture is ready and small: {e}"),
                    }
                }
                if staged {
                    self.tex = TexState::Staged(id);
                }
            }
            TexState::Reading(..) => {}
        }
        Ok(())
    }

    /// Stages random rectangles into the texture and, if they were staged, applies them to
    /// the CPU copy.
    fn update_texture(
        &mut self,
        ring: &mut UploadRing,
        table: &mut TextureTable,
        id: TextureId,
    ) -> Result<(), UploadError> {
        let n = 1 + (self.next() % MAX_RECTS as u64) as usize;
        let mut rects = [ImageRegion::default(); MAX_RECTS];
        let mut spans = [(0usize, 0usize); MAX_RECTS];
        let mut count = 0;
        let mut at = 0;
        for _ in 0..n {
            let mip = (self.next() % u64::from(TEX_MIPS)) as u32;
            let (w, h) = (TEX_WIDTH >> mip, TEX_HEIGHT >> mip);
            let width = 1 + (self.next() % u64::from(w)) as u32;
            let height = 1 + (self.next() % u64::from(h)) as u32;
            let x = (self.next() % u64::from(w - width + 1)) as u32;
            let y = (self.next() % u64::from(h - height + 1)) as u32;
            let len = (width * height * 4) as usize;
            if at + len > TEX_BYTES {
                break;
            }
            for b in 0..len {
                self.tex_scratch[at + b] = (self.next() >> 56) as u8;
            }
            rects[count] = ImageRegion {
                mip,
                x,
                y,
                width,
                height,
            };
            spans[count] = (at, len);
            count += 1;
            at += len;
        }
        let regions: [(ImageRegion, &[u8]); MAX_RECTS] = std::array::from_fn(|i| {
            let (s, len) = spans[i];
            (rects[i], &self.tex_scratch[s..s + len])
        });
        table.update(ring, id, &regions[..count])?;
        for (r, data) in &regions[..count] {
            let base = TEX_OFFSETS[r.mip as usize];
            let row = ((TEX_WIDTH >> r.mip) * 4) as usize;
            let w = (r.width * 4) as usize;
            for y in 0..r.height as usize {
                let dst = base + (r.y as usize + y) * row + r.x as usize * 4;
                self.tex_mirror[dst..dst + w].copy_from_slice(&data[y * w..y * w + w]);
            }
        }
        self.report.texture_rects += count as u64;
        Ok(())
    }

    /// Records a copy of every mip level of the texture into the readback buffer.
    ///
    /// # Safety
    /// `ctx.cmd` is recording on the graphics queue; the texture is ready and its written
    /// texels are visible to this frame.
    unsafe fn read_texture(&self, table: &TextureTable, id: TextureId, ctx: &FrameCtx) {
        let (image, _) = table.image(id);
        let readback = self.tex_readback.as_ref().expect("live").raw;
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: TEX_MIPS,
            base_array_layer: 0,
            layer_count: 1,
        };
        let shaders = vk::PipelineStageFlags2::VERTEX_SHADER
            | vk::PipelineStageFlags2::FRAGMENT_SHADER
            | vk::PipelineStageFlags2::COMPUTE_SHADER;
        // After the ring's copies (COPY) and its or the acquire's transition (shaders).
        let before = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY | shaders)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .image(image)
            .subresource_range(range)];
        // Back for sampling; shaders as the destination, so the ring's next write to it
        // (which waits for shaders) comes after this read.
        let after_image = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_stage_mask(shaders)
            .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image(image)
            .subresource_range(range)];
        let after_host = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::HOST)
            .dst_access_mask(vk::AccessFlags2::HOST_READ)];
        let copies: [vk::BufferImageCopy; TEX_MIPS as usize] =
            std::array::from_fn(|m| vk::BufferImageCopy {
                buffer_offset: TEX_OFFSETS[m] as u64,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: m as u32,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: vk::Offset3D::default(),
                image_extent: vk::Extent3D {
                    width: TEX_WIDTH >> m,
                    height: TEX_HEIGHT >> m,
                    depth: 1,
                },
            });
        let gpu = &*self.gpu;
        // SAFETY: guaranteed by the caller; the readback buffer is not in use (the last
        // read-back was compared after its frame finished).
        unsafe {
            gpu.cmd_pipeline_barrier2(
                ctx.cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&before),
            );
            gpu.device.cmd_copy_image_to_buffer(
                ctx.cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                readback,
                &copies,
            );
            gpu.cmd_pipeline_barrier2(
                ctx.cmd,
                &vk::DependencyInfo::default()
                    .memory_barriers(&after_host)
                    .image_memory_barriers(&after_image),
            );
        }
    }
}

impl Drop for UploadCheck {
    fn drop(&mut self) {
        // SAFETY: the owner drops the check only after the GPU is idle (after the frame
        // loop, whose Drop waits for every queue).
        unsafe {
            if let Some(b) = self.target.take() {
                b.destroy(&self.gpu);
            }
            if let Some(b) = self.readback.take() {
                b.destroy(&self.gpu);
            }
            if let Some(b) = self.tex_readback.take() {
                b.destroy(&self.gpu);
            }
        }
    }
}
