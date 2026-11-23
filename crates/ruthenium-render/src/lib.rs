//! Ruthenium's renderer. For now: GPU start-up for a window, the pipeline set, the upload
//! ring and texture table, and a frame that draws the menu background and the GUI's draw
//! list, recorded and presented through [`ruthenium_gpu::FrameLoop`].

mod backdrop;
mod gui;
mod overlay;
mod pipelines;
mod starfield;
mod stats;

pub use gui::GuiFrame;
pub use overlay::{LINES as OVERLAY_LINES, Overlay, OverlayStats};
pub use pipelines::PipelineId;
pub use stats::{FrameTimes, Summary};

use std::path::Path;
use std::sync::Arc;

use ruthenium_gpu::{
    Arrival, COLOR_RANGE, FrameData, FrameLoop, Gpu, GpuOptions, GpuTime, Instance, InstanceDesc,
    PUSH_CONSTANT_STAGES, PipelineCache, PipelineLayout, Pipelines, PresentPolicy, Presented,
    Screenshot, TextureDesc, TextureId, TextureTable, UploadCheck, UploadCheckReport, UploadRing,
    Validation, vk,
};
use ruthenium_platform::{Platform, Window};

/// Creates the Vulkan instance, the window's surface and the device.
///
/// `window` must outlive the returned Gpu (and everything created from it).
pub fn create_gpu(
    platform: &Platform,
    window: &Window,
    validation: Validation,
    options: &GpuOptions,
) -> Result<Arc<Gpu>, Box<dyn std::error::Error>> {
    let surface_extensions = platform.vulkan_instance_extensions();
    let instance = Instance::new(&InstanceDesc {
        get_instance_proc_addr: platform.vk_get_instance_proc_addr()?,
        surface_extensions: &surface_extensions,
        validation,
    })?;
    // SAFETY: the instance was created with SDL's surface extensions; Gpu takes ownership
    // of the surface and destroys it before the instance; the caller keeps the window alive.
    let surface = unsafe { window.create_surface(instance.handle())? };
    Ok(Arc::new(Gpu::new(instance, surface, options)?))
}

/// What one rendered frame reports back to the loop.
#[derive(Clone, Copy, Debug)]
pub struct FrameReport {
    /// The frame's number: the one its GPU time is reported under later, and
    /// [`ruthenium_gpu::FrameTimings::frame`].
    pub frame: u64,
    pub presented: Presented,
    /// GPU time of an earlier frame that has just been read back.
    pub gpu_time: Option<GpuTime>,
}

/// The menu background: a vertical gradient between two display-encoded RGBA colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Background {
    pub top: [f32; 4],
    pub bottom: [f32; 4],
}

impl Background {
    /// `background.frag`'s push constants: the two colours, then the starfield texture.
    fn push_constants(&self, stars: TextureId) -> [u8; 36] {
        let mut b = [0u8; 36];
        for (i, v) in self.top.iter().chain(&self.bottom).enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
        }
        b[32..].copy_from_slice(&stars.index().to_ne_bytes());
        b
    }
}

/// Staging ring size (research 04 §4.3: 8 MiB on the lowest tier, 16 MiB by default).
const UPLOAD_RING_BYTES: u64 = 16 << 20;
/// Bytes one frame may upload: less on integrated GPUs, which share memory bandwidth
/// with the CPU. One larger upload still goes, alone in its frame.
const UPLOAD_BUDGET_DISCRETE: u64 = 4 << 20;
const UPLOAD_BUDGET_INTEGRATED: u64 = 3 << 19;
/// Per-frame data each frame may write (GUI and overlay rectangles for now).
const FRAME_DATA_BYTES: u64 = 1 << 20;

/// Draws frames into the window.
#[derive(Debug)]
pub struct Renderer {
    // Fields drop in order: the frame loop first (its Drop waits for the GPU), then what
    // its frames used, then the cache, which saves itself as it goes.
    frames: FrameLoop,
    pipelines: Pipelines,
    layout: PipelineLayout,
    textures: TextureTable,
    uploads: UploadRing,
    frame_data: FrameData,
    overlay: Overlay,
    cache: PipelineCache,
    stars: TextureId,
    /// `--exercise-uploads`.
    check: Option<UploadCheck>,
    /// The next presented frame is copied for a screenshot.
    screenshot_requested: bool,
    screenshot: Option<Screenshot>,
    /// The menus' backdrop, while frames draw shapes.
    backdrop: Option<backdrop::Backdrop>,
    /// The window's size as last acquired: the size the backdrop is made for.
    window_extent: vk::Extent2D,
}

impl Renderer {
    /// Creates the frame loop and builds every pipeline through the on-disk pipeline cache
    /// in `cache_dir` (`None`: no cache file).
    pub fn new(
        gpu: Arc<Gpu>,
        pixel_size: (u32, u32),
        policy: PresentPolicy,
        cache_dir: Option<&Path>,
    ) -> Result<Renderer, ruthenium_gpu::Error> {
        let frames = FrameLoop::new(gpu.clone(), pixel_size, policy)?;
        let budget = if gpu.caps().unified_memory {
            UPLOAD_BUDGET_INTEGRATED
        } else {
            UPLOAD_BUDGET_DISCRETE
        };
        let mut uploads = UploadRing::new(gpu.clone(), UPLOAD_RING_BYTES, budget)?;
        let mut textures = TextureTable::new(gpu.clone(), &mut uploads)?;
        let stars = textures.create_with(
            &mut uploads,
            TextureDesc::rgba8(starfield::SIZE, starfield::SIZE),
            &[&starfield::bake(starfield::SEED)],
            // The first frame draws it.
            Arrival::NextFrame,
        )?;
        eprintln!(
            "[render] texture table: {} slots; upload ring {} MiB, {:.1} MiB per frame, buffer copies and background textures on the {}",
            textures.capacity(),
            uploads.capacity() >> 20,
            uploads.budget() as f64 / f64::from(1 << 20),
            match gpu.transfer_family() {
                Some(f) => format!("transfer queue (family {f})"),
                None => "graphics queue".to_owned(),
            }
        );
        let frame_data = FrameData::new(gpu.clone(), FRAME_DATA_BYTES)?;
        let layout = PipelineLayout::new(gpu.clone(), &[textures.layout(), frame_data.layout()])?;
        let mut cache = PipelineCache::open(gpu.clone(), cache_dir)?;
        let pipelines = Pipelines::build(
            gpu,
            &mut cache,
            &layout,
            &pipelines::descs(),
            frames.color_format()?,
        )?;
        Ok(Renderer {
            frames,
            pipelines,
            layout,
            textures,
            uploads,
            frame_data,
            overlay: Overlay::new(),
            cache,
            stars,
            check: None,
            screenshot_requested: false,
            screenshot: None,
            backdrop: None,
            window_extent: vk::Extent2D {
                width: pixel_size.0,
                height: pixel_size.1,
            },
        })
    }

    /// Starts checking every frame that uploads arrive intact (see [`UploadCheck`]).
    pub fn enable_upload_check(&mut self) -> Result<(), ruthenium_gpu::Error> {
        if self.check.is_none() {
            self.check = Some(UploadCheck::new(self.frames.gpu().clone())?);
            // Also run the path where a frame's transfer copies wait for the next frame.
            self.uploads.force_deferral_every(3);
        }
        Ok(())
    }

    /// What the upload check has seen, if it is on; how many frames' transfer copies had
    /// to wait for a free command buffer; and how many images went on the transfer queue.
    pub fn upload_check(&self) -> Option<(UploadCheckReport, u64, u64)> {
        self.check.as_ref().map(|c| {
            (
                c.report(),
                self.uploads.deferred_transfers(),
                self.uploads.transferred_images(),
            )
        })
    }

    /// Creates a texture and stages its levels (see [`TextureTable::create_with`]). A
    /// full upload ring is an error the caller retries next frame.
    pub fn create_texture(
        &mut self,
        desc: TextureDesc,
        mips: &[&[u8]],
        arrival: Arrival,
    ) -> Result<TextureId, ruthenium_gpu::Error> {
        self.textures
            .create_with(&mut self.uploads, desc, mips, arrival)
    }

    /// Destroys a texture once the frames that may use it have finished (see
    /// [`TextureTable::destroy`]).
    pub fn destroy_texture(&mut self, id: TextureId) {
        self.textures.destroy(id);
    }

    /// Whether frames recorded from now on may sample `id`.
    pub fn texture_ready(&self, id: TextureId) -> bool {
        self.textures.is_ready(id)
    }

    /// Copies the next frame drawn into the window (see [`Self::take_screenshot`]).
    pub fn request_screenshot(&mut self) {
        self.screenshot_requested = true;
    }

    /// A requested screenshot once its frame has finished: `(width, height, RGBA)`.
    /// Swapchains that cannot be read give none, and the request stays pending.
    pub fn take_screenshot(&mut self) -> Result<Option<(u32, u32, Vec<u8>)>, ruthenium_gpu::Error> {
        match &self.screenshot {
            Some(s) if s.done(&self.frames)? => {
                Ok(self.screenshot.take().map(Screenshot::into_rgba))
            }
            _ => Ok(None),
        }
    }

    /// The development overlay (hidden until `set_visible`).
    pub fn overlay_mut(&mut self) -> &mut Overlay {
        &mut self.overlay
    }

    pub fn gpu(&self) -> &Arc<Gpu> {
        self.frames.gpu()
    }

    pub fn frames(&self) -> &FrameLoop {
        &self.frames
    }

    pub fn frames_mut(&mut self) -> &mut FrameLoop {
        &mut self.frames
    }

    /// Creates the backdrop's images when shapes are drawn (before the texture table's
    /// `begin_frame`, so this frame's set holds them), remakes them when the window size or
    /// format changed, and frees them when no shapes are drawn.
    fn update_backdrop(&mut self, wanted: bool) {
        let (format, extent) = (self.pipelines.format(), self.window_extent);
        let fits = self
            .backdrop
            .as_ref()
            .is_some_and(|b| b.matches(format, extent));
        if wanted && fits {
            return;
        }
        if let Some(old) = self.backdrop.take() {
            old.destroy(&mut self.textures);
        }
        if wanted && extent.width > 0 && extent.height > 0 {
            match backdrop::Backdrop::new(&mut self.textures, format, extent) {
                Ok(b) => self.backdrop = Some(b),
                // Glass falls back to a flat tint.
                Err(e) => eprintln!("[render] no glass backdrop: {e}"),
            }
        }
    }

    /// Rebuilds the pipelines if the swapchain's format changed (rare: a different monitor
    /// or surface). Waits for earlier frames, which may still use the old pipelines.
    /// Returns whether the pipelines were rebuilt.
    fn ensure_format(&mut self, format: vk::Format) -> Result<bool, ruthenium_gpu::Error> {
        if format == self.pipelines.format() {
            return Ok(false);
        }
        self.frames.wait_for_gpu(1)?;
        let gpu = self.frames.gpu().clone();
        self.pipelines = Pipelines::build(
            gpu,
            &mut self.cache,
            &self.layout,
            &pipelines::descs(),
            format,
        )?;
        Ok(true)
    }

    /// Records, submits and presents one frame showing `background`, then the GUI.
    pub fn render(
        &mut self,
        background: &Background,
        gui: Option<GuiFrame<'_>>,
    ) -> Result<FrameReport, ruthenium_gpu::Error> {
        profiling::scope!("render");
        let (mut ctx, gpu_time) = self.frames.begin()?;
        let completed = self.frames.completed()?;
        let wants_backdrop = gui.is_some_and(|g| !g.shapes.is_empty());
        self.update_backdrop(wants_backdrop);
        // SAFETY: ctx.cmd is recording frame ctx.number on the graphics queue; completed
        // was just read from the timeline; every upload destination is owned by self and
        // outlives the frame; the layout's set 0 is the texture table's.
        unsafe {
            let zone = self.frames.zone_begin(&ctx, "uploads");
            self.uploads.record(&mut ctx, completed)?;
            self.textures
                .begin_frame(ctx.slot(), ctx.number, completed, &self.uploads);
            self.frames.zone_end(&ctx, zone);
            if let Some(c) = &mut self.check {
                let zone = self.frames.zone_begin(&ctx, "upload check");
                c.step(&mut self.uploads, &mut self.textures, &ctx, completed)?;
                self.frames.zone_end(&ctx, zone);
            }
            self.textures.bind(ctx.cmd, ctx.slot(), self.layout.raw());
            self.frame_data.begin_frame(ctx.slot());
            self.frame_data.bind(ctx.cmd, self.layout.raw());
            // First frame (and after a rebuild): draw once with every pipeline.
            if !self.pipelines.warmed_up() {
                let zone = self.frames.zone_begin(&ctx, "warm-up");
                self.pipelines.record_warm_up(ctx.cmd, &self.layout)?;
                self.frames.zone_end(&ctx, zone);
            }
        }
        // Off-screen passes will be recorded here, before the late acquire.
        let target = self.frames.acquire(&ctx)?;
        if let Some(t) = target {
            if self.ensure_format(t.format)? {
                // SAFETY: as above; outside a render pass.
                unsafe { self.pipelines.record_warm_up(ctx.cmd, &self.layout)? };
            }
            self.window_extent = t.extent;
            // The backdrop for this size and format (else it is remade next frame).
            let backdrop = match &mut self.backdrop {
                Some(b) if wants_backdrop && b.matches(t.format, t.extent) => {
                    let zone = self.frames.zone_begin(&ctx, "backdrop");
                    // SAFETY: outside a render pass, sets bound; the images were created
                    // before this frame's begin_frame.
                    unsafe {
                        b.record(
                            self.frames.gpu(),
                            ctx.cmd,
                            &self.pipelines,
                            &self.layout,
                            background,
                            self.stars,
                        )
                    };
                    self.frames.zone_end(&ctx, zone);
                    Some(b.textures())
                }
                _ => None,
            };
            let zone = self.frames.zone_begin(&ctx, "window pass");
            let gpu = self.frames.gpu().clone();
            let cmd = ctx.cmd;
            let to_attachment = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::NONE)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(t.image)
                .subresource_range(COLOR_RANGE)];
            let to_present = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::NONE)
                .dst_access_mask(vk::AccessFlags2::NONE)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .image(t.image)
                .subresource_range(COLOR_RANGE)];
            // The background covers every pixel, so the old contents are never loaded.
            let attachments = [vk::RenderingAttachmentInfo::default()
                .image_view(t.view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)];
            let area = vk::Rect2D {
                offset: vk::Offset2D::default(),
                extent: t.extent,
            };
            let rendering = vk::RenderingInfo::default()
                .render_area(area)
                .layer_count(1)
                .color_attachments(&attachments);
            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: t.extent.width as f32,
                height: t.extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let push = background.push_constants(self.stars);
            let d = gpu.device();
            // SAFETY: cmd is recording; the image was acquired this frame and its acquire
            // semaphore is waited at COLOR_ATTACHMENT_OUTPUT, the first barrier's source stage;
            // the pipeline was built for the target's format with the shared layout.
            unsafe {
                gpu.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_attachment),
                );
                gpu.cmd_begin_rendering(cmd, &rendering);
                // With a backdrop, the background is already drawn: copy it.
                let (pipeline, push) = match backdrop {
                    Some((scene, _)) => {
                        (PipelineId::Blur, backdrop::blur_push_copy(scene).to_vec())
                    }
                    None => (PipelineId::Background, push.to_vec()),
                };
                d.cmd_bind_pipeline(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipelines.get(pipeline.index()),
                );
                d.cmd_set_viewport(cmd, 0, &[viewport]);
                d.cmd_set_scissor(cmd, 0, &[area]);
                d.cmd_push_constants(cmd, self.layout.raw(), PUSH_CONSTANT_STAGES, 0, &push);
                d.cmd_draw(cmd, 3, 1, 0, 0);
                if let Some(g) = gui.filter(|g| !g.quads.is_empty() && g.scale > 0)
                    && let Some((first, bytes)) =
                        self.frame_data.alloc(g.quads.len() as u32, gui::QUAD_BYTES)
                {
                    gui::write_quads(g.quads, bytes);
                    let mut push = [0u8; 16];
                    push[0..4].copy_from_slice(&(t.extent.width as f32).to_ne_bytes());
                    push[4..8].copy_from_slice(&(t.extent.height as f32).to_ne_bytes());
                    push[8..12].copy_from_slice(&(g.scale as f32).to_ne_bytes());
                    push[12..16].copy_from_slice(&first.to_ne_bytes());
                    d.cmd_bind_pipeline(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.pipelines.get(PipelineId::Gui.index()),
                    );
                    d.cmd_push_constants(cmd, self.layout.raw(), PUSH_CONSTANT_STAGES, 0, &push);
                    d.cmd_draw(cmd, 6, g.quads.len() as u32, 0, 0);
                }
                if let Some(g) = gui.filter(|g| !g.shapes.is_empty())
                    && let Some((first, bytes)) = self
                        .frame_data
                        .alloc(g.shapes.len() as u32, gui::SHAPE_BYTES)
                {
                    gui::write_shapes(g.shapes, bytes);
                    let (scene, blurred) =
                        backdrop.unwrap_or((TextureId::MISSING, TextureId::MISSING));
                    let mut push = [0u8; 24];
                    push[0..4].copy_from_slice(&(t.extent.width as f32).to_ne_bytes());
                    push[4..8].copy_from_slice(&(t.extent.height as f32).to_ne_bytes());
                    push[8..12].copy_from_slice(&scene.index().to_ne_bytes());
                    push[12..16].copy_from_slice(&blurred.index().to_ne_bytes());
                    push[16..20].copy_from_slice(&first.to_ne_bytes());
                    push[20..24].copy_from_slice(&u32::from(backdrop.is_some()).to_ne_bytes());
                    d.cmd_bind_pipeline(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.pipelines.get(PipelineId::Shape.index()),
                    );
                    d.cmd_push_constants(cmd, self.layout.raw(), PUSH_CONSTANT_STAGES, 0, &push);
                    d.cmd_draw(cmd, 6, g.shapes.len() as u32, 0, 0);
                }
                if let Some(g) = gui.filter(|g| !g.glyphs.is_empty())
                    && let Some((first, bytes)) = self
                        .frame_data
                        .alloc(g.glyphs.len() as u32, gui::GLYPH_BYTES)
                {
                    gui::write_glyphs(g.glyphs, bytes);
                    let mut push = [0u8; 16];
                    push[0..4].copy_from_slice(&(t.extent.width as f32).to_ne_bytes());
                    push[4..8].copy_from_slice(&(t.extent.height as f32).to_ne_bytes());
                    push[8..12].copy_from_slice(&g.font_atlas.to_ne_bytes());
                    push[12..16].copy_from_slice(&first.to_ne_bytes());
                    d.cmd_bind_pipeline(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.pipelines.get(PipelineId::Text.index()),
                    );
                    d.cmd_push_constants(cmd, self.layout.raw(), PUSH_CONSTANT_STAGES, 0, &push);
                    d.cmd_draw(cmd, 6, g.glyphs.len() as u32, 0, 0);
                }
                let rects = self.overlay.build(t.extent.width, t.extent.height);
                if let Some((first, bytes)) = self
                    .frame_data
                    .alloc(rects.len() as u32, overlay::RECT_BYTES)
                    && !rects.is_empty()
                {
                    overlay::write_rects(rects, bytes);
                    let mut push = [0u8; 12];
                    push[0..4].copy_from_slice(&(t.extent.width as f32).to_ne_bytes());
                    push[4..8].copy_from_slice(&(t.extent.height as f32).to_ne_bytes());
                    push[8..12].copy_from_slice(&first.to_ne_bytes());
                    d.cmd_bind_pipeline(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.pipelines.get(PipelineId::Overlay.index()),
                    );
                    d.cmd_push_constants(cmd, self.layout.raw(), PUSH_CONSTANT_STAGES, 0, &push);
                    d.cmd_draw(cmd, 6, rects.len() as u32, 0, 0);
                }
                gpu.cmd_end_rendering(cmd);
                let shot = if self.screenshot_requested && self.screenshot.is_none() {
                    Screenshot::record(&gpu, &ctx, &t)?
                } else {
                    None
                };
                if let Some(s) = shot {
                    // It made the transition to PRESENT_SRC itself.
                    self.screenshot = Some(s);
                    self.screenshot_requested = false;
                } else {
                    gpu.cmd_pipeline_barrier2(
                        cmd,
                        &vk::DependencyInfo::default().image_memory_barriers(&to_present),
                    );
                }
            }
            self.frames.zone_end(&ctx, zone);
        }
        let frame = ctx.number;
        let presented = self.frames.submit(ctx, target)?;
        Ok(FrameReport {
            frame,
            presented,
            gpu_time,
        })
    }
}
