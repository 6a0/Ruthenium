//! A development check of the upload path (`--exercise-uploads`): every frame, random-sized
//! chunks of known bytes are uploaded through the ring into a GPU-only buffer; once a
//! chunk's ticket is ready, the frame copies it on to a host-visible buffer, and after that
//! frame has finished the CPU compares the bytes. Run under validation (with
//! synchronization checks) it covers the ring's wrap-around, the per-frame budget, retries
//! when full, and, on GPUs with a copy engine, the transfer queue and its timeline wait.

use std::sync::Arc;

use ash::vk;

use crate::Error;
use crate::device::Gpu;
use crate::frame::FrameCtx;
use crate::memory::{Buffer, MemoryUse};
use crate::upload::{UploadError, UploadRing, UploadTicket};

/// Chunks in flight at once, each with its own range of the two buffers.
const CHUNKS: usize = 16;
/// The largest chunk; sizes are random multiples of 4 up to this. Sixteen of them are
/// twice the discrete per-frame budget, so uploads regularly have to wait a frame.
const CHUNK_BYTES: usize = 512 << 10;
/// Mismatches printed in detail; the rest are only counted.
const PRINTED: u64 = 8;

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
        Ok(UploadCheck {
            gpu,
            target: Some(target),
            readback: Some(readback),
            chunks: [State::Idle; CHUNKS],
            rng: 0x9E37_79B9_7F4A_7C15,
            scratch: vec![0; CHUNK_BYTES],
            report: UploadCheckReport::default(),
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
    pub unsafe fn step(&mut self, ring: &mut UploadRing, ctx: &FrameCtx, completed: u64) {
        profiling::scope!("upload_check");
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
                Err(UploadError::TooLarge) => unreachable!("a chunk is smaller than the ring"),
            }
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
        }
    }
}
