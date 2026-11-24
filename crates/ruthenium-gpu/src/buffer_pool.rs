//! Pooled GPU buffer ranges: the pages behind a [`RangePool`], for data with many small,
//! short-lived allocations (terrain meshes first). A range is named by a generational
//! handle, lives in one page (one `VkBuffer` with its own memory, made through the
//! allocator seam in `memory.rs`) and is taken back only when the last frame that drew it
//! and the last copy that wrote it have finished.
//!
//! The pool grows only within its budget and never on the path of a frame that needs the
//! room: when free space falls to the low watermark, the next page is created and bound
//! on a worker thread (`vkAllocateMemory` can take milliseconds) and adopted once it
//! exists. Empty pages are released with hysteresis. Ranges never move: nothing here
//! relocates or compacts.

use std::sync::Arc;
use std::thread::JoinHandle;

use ash::vk;

use crate::Error;
use crate::device::Gpu;
use crate::memory::{Buffer, MemoryUse};
use crate::pool::{PoolConfig, PoolError, PoolStats, RangeHandle, RangePool};
use crate::users::{Completed, Users};

/// A page as its worker thread hands it over.
struct NewPage(Result<Buffer, Error>);

// SAFETY: a buffer is Vulkan handles and, where its memory is host-visible, the address of
// a mapping nothing has used yet; both may move to another thread.
unsafe impl Send for NewPage {}

/// Where a live range is, as a draw or a copy needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PooledBuffer {
    pub buffer: vk::Buffer,
    /// The page's index: ranges of one page share a buffer binding.
    pub page: u32,
    /// Where the range starts in the buffer, and its size as requested.
    pub offset: u64,
    pub size: u64,
    /// The generation of its handle.
    pub generation: u32,
}

/// See the module docs.
pub struct BufferPool {
    gpu: Arc<Gpu>,
    ranges: RangePool,
    /// The buffers of the pages, by page index.
    pages: Vec<Option<Buffer>>,
    usage: vk::BufferUsageFlags,
    /// The page being created on a worker thread.
    pending: Option<JoinHandle<NewPage>>,
    /// Pages that could not be created (out of device memory).
    failed_pages: u32,
}

impl std::fmt::Debug for BufferPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferPool")
            .field("stats", &self.ranges.stats())
            .finish_non_exhaustive()
    }
}

// SAFETY: Vulkan handles may move threads; all mutation goes through &mut self, and the
// mappings of the pages (if their memory has any) are never used.
unsafe impl Send for BufferPool {}

impl BufferPool {
    /// A pool of pages with `usage` (copies into them are always allowed), sized by
    /// `config`. It starts without a page: [`Self::grow_now`] makes the first one behind
    /// a loading screen, or [`Self::maintain`] has it made on a worker.
    pub fn new(gpu: Arc<Gpu>, config: PoolConfig, usage: vk::BufferUsageFlags) -> BufferPool {
        BufferPool {
            gpu,
            ranges: RangePool::new(config),
            pages: Vec::new(),
            usage: usage | vk::BufferUsageFlags::TRANSFER_DST,
            pending: None,
            failed_pages: 0,
        }
    }

    /// A range of `size` bytes at an offset that is a multiple of `align` (any multiple of
    /// the pool's granule: a vertex stride, say). [`PoolError::Growing`]: nothing fits
    /// now, but a page is on its way or will be after the next [`Self::maintain`]; try
    /// again in a later frame. [`PoolError::Full`]: nothing fits and the budget is used
    /// up; retire ranges, or do without.
    pub fn alloc(&mut self, size: u64, align: u64) -> Result<RangeHandle, PoolError> {
        self.ranges.alloc(size, align)
    }

    /// Where the range of a handle is. `None` once the range was retired: a stale handle
    /// is gone, and never names the range that took its place.
    pub fn resolve(&self, handle: RangeHandle) -> Option<PooledBuffer> {
        let r = self.ranges.resolve(handle)?;
        let buffer = self.pages[r.page as usize]
            .as_ref()
            .expect("a live range is on a page the pool has")
            .raw;
        Some(PooledBuffer {
            buffer,
            page: r.page,
            offset: r.offset,
            size: r.size,
            generation: r.generation,
        })
    }

    /// Ends the life of a range. Its bytes are handed out again only after `users` have
    /// all finished: name the last frame that may draw it and the upload batches that
    /// write it ([`crate::UploadRing::users_of`]), also ones not submitted yet. `false`
    /// for a stale handle.
    pub fn retire(&mut self, handle: RangeHandle, users: Users) -> bool {
        self.ranges.retire(handle, users)
    }

    /// Once per frame: takes back the retired ranges whose users have finished, adopts a
    /// page a worker has finished creating, has the next one created if free space is
    /// down to the low watermark (or an allocation failed for want of one), and releases
    /// an empty page that has waited long enough.
    pub fn maintain(&mut self, completed: &Completed) {
        self.ranges.reclaim(completed);
        if self.pending.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(page) = self.pending.take()
        {
            match page.join() {
                Ok(NewPage(Ok(buffer))) => self.adopt(buffer),
                Ok(NewPage(Err(e))) => self.page_failed(&e.to_string()),
                Err(_) => self.page_failed("its thread panicked"),
            }
        }
        if self.pending.is_none() && self.ranges.wants_page() {
            let (gpu, usage) = (self.gpu.clone(), self.usage);
            let size = self.ranges.config().page_bytes;
            let spawned = std::thread::Builder::new()
                .name("gpu-page".into())
                .spawn(move || NewPage(Buffer::new(&gpu, size, usage, MemoryUse::GpuOnly)));
            match spawned {
                Ok(thread) => {
                    self.ranges.page_requested();
                    self.pending = Some(thread);
                }
                Err(e) => self.page_failed(&e.to_string()),
            }
        }
        if let Some(index) = self.ranges.take_empty_page()
            && let Some(buffer) = self.pages[index as usize].take()
        {
            // SAFETY: the page has no live or retiring range, so every frame that drew
            // from it and every copy into it have finished (retire's contract).
            unsafe { buffer.destroy(&self.gpu) };
        }
    }

    /// Creates a page on this thread, if the budget allows one and none is on its way.
    /// For a loading screen; while playing, [`Self::maintain`] grows the pool off the
    /// frame's path. Returns whether a page was added.
    pub fn grow_now(&mut self) -> Result<bool, Error> {
        let config = self.ranges.config();
        let reserved = self.ranges.stats().reserved_bytes;
        if self.pending.is_some() || reserved + config.page_bytes > config.budget_bytes {
            return Ok(false);
        }
        let buffer = Buffer::new(&self.gpu, config.page_bytes, self.usage, MemoryUse::GpuOnly)?;
        self.adopt(buffer);
        Ok(true)
    }

    fn adopt(&mut self, buffer: Buffer) {
        let index = self.ranges.add_page() as usize;
        if self.pages.len() <= index {
            self.pages.resize_with(index + 1, || None);
        }
        self.pages[index] = Some(buffer);
    }

    fn page_failed(&mut self, why: &str) {
        if self.failed_pages == 0 {
            eprintln!("[gpu] a page of pooled buffer memory could not be created: {why}");
        }
        self.failed_pages += 1;
        self.ranges.page_failed();
    }

    /// Whether a page is being created on a worker now.
    pub fn growing(&self) -> bool {
        self.pending.is_some()
    }

    pub fn stats(&self) -> PoolStats {
        self.ranges.stats()
    }
}

impl Drop for BufferPool {
    fn drop(&mut self) {
        let created = self.pending.take().and_then(|thread| thread.join().ok());
        // SAFETY: the owner drops the pool once the GPU is idle (after the frame loop and
        // the upload ring, which wait for their queues), so nothing uses the pages; the
        // page from the worker was never used.
        unsafe {
            if let Some(NewPage(Ok(buffer))) = created {
                buffer.destroy(&self.gpu);
            }
            for buffer in self.pages.drain(..).flatten() {
                buffer.destroy(&self.gpu);
            }
        }
    }
}
