//! The shared texture table: one descriptor set, bound once per frame at set 0 of every
//! pipeline, holding every sampled texture (atlases, font pages, GUI textures) in one array
//! and a fixed set of samplers. Shaders index it with the [`TextureId`] and [`SamplerId`]
//! passed in push constants, so a draw never writes or binds a descriptor.
//!
//! - There is one copy of the set per frame in flight. A copy is written only at the start
//!   of its slot's frame, when the frame that last used it has finished and before
//!   anything of the new frame binds it. Ordinary descriptor sets allow exactly that, so
//!   the table is made of them wherever their limits hold it, and of update-after-bind
//!   sets only for a table larger than those limits ([`TextureTablePath`]). Shaders and
//!   contents are the same on both.
//! - The table has a fixed number of slots (`DeviceCaps::texture_slots`). When all are
//!   taken, creating a texture fails with [`Error::TextureTableFull`], which says whether
//!   destroyed textures are still on their way out: their slots come back once the
//!   frames that may sample them have finished, so the caller tries again later.
//! - Every slot not holding a ready texture shows [`TextureId::MISSING`], a magenta and
//!   black checkerboard, so a stale or unknown index never reads a dead descriptor.
//! - A texture becomes visible once its first upload is ready ([`Arrival`]): in the frame
//!   that records it, or, for background uploads on a transfer queue, in the first frame
//!   after that queue has finished it. A destroyed one is freed only after every frame that
//!   could still sample it, and its upload, have finished.
//! - Once visible, a texture can be rewritten in part ([`TextureTable::update`]: animated
//!   textures, atlas patches); a frame's updates are recorded before its passes, so the
//!   frame samples the new texels.

use std::fmt;
use std::sync::Arc;

use ash::vk;
use ash::vk::TaggedStructure;

use crate::device::{Gpu, TextureTablePath};
use crate::frame::MAX_FRAMES_IN_FLIGHT;
use crate::memory::{Image, texel_bytes};
use crate::upload::{ImageDst, ImageRegion, UploadError, UploadRing, UploadTicket};
use crate::{Error, VkContext};

/// Mip levels a texture may have (a 32768² texture has 16).
pub const MAX_MIP_LEVELS: usize = 16;

/// Descriptor binding of the samplers in set 0 (`glsl/include/textures.glsl`).
const SAMPLER_BINDING: u32 = 0;
/// Descriptor binding of the texture array in set 0.
const TEXTURE_BINDING: u32 = 1;

/// Stages that can sample the table.
const STAGES: vk::ShaderStageFlags = vk::ShaderStageFlags::from_raw(
    vk::ShaderStageFlags::VERTEX.as_raw()
        | vk::ShaderStageFlags::FRAGMENT.as_raw()
        | vk::ShaderStageFlags::COMPUTE.as_raw(),
);

/// Descriptor writes made by one `vkUpdateDescriptorSets` call; longer lists go in several.
const WRITE_BATCH: usize = 256;

/// Side of the missing texture, in texels, and its two colours (RGBA8).
const MISSING_SIZE: u32 = 16;
const MISSING_COLORS: [[u8; 4]; 2] = [[0xF8, 0x00, 0xF8, 0xFF], [0x00, 0x00, 0x00, 0xFF]];

/// A texture's index in the table, as shaders take it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureId(u32);

impl TextureId {
    /// The checkerboard every unused slot shows; always present.
    pub const MISSING: TextureId = TextureId(0);

    pub fn index(self) -> u32 {
        self.0
    }
}

/// The table's samplers, by index (`SAMPLER_*` in `glsl/include/textures.glsl`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SamplerId {
    /// Texel-exact, edges clamped: GUI sprites, font glyphs.
    NearestClamp,
    /// Texel-exact, tiling.
    NearestRepeat,
    /// Bilinear, edges clamped: scaled images such as the panorama.
    LinearClamp,
    /// Bilinear, tiling.
    LinearRepeat,
}

/// Number of samplers in the table.
pub const SAMPLER_COUNT: u32 = SamplerId::ALL.len() as u32;

impl SamplerId {
    pub const ALL: [SamplerId; 4] = [
        SamplerId::NearestClamp,
        SamplerId::NearestRepeat,
        SamplerId::LinearClamp,
        SamplerId::LinearRepeat,
    ];

    pub fn index(self) -> u32 {
        self as u32
    }

    /// The name of its index constant in `textures.glsl`.
    pub fn glsl_name(self) -> &'static str {
        match self {
            SamplerId::NearestClamp => "SAMPLER_NEAREST_CLAMP",
            SamplerId::NearestRepeat => "SAMPLER_NEAREST_REPEAT",
            SamplerId::LinearClamp => "SAMPLER_LINEAR_CLAMP",
            SamplerId::LinearRepeat => "SAMPLER_LINEAR_REPEAT",
        }
    }

    fn info(self) -> vk::SamplerCreateInfo<'static> {
        let (filter, mipmap) = match self {
            SamplerId::NearestClamp | SamplerId::NearestRepeat => {
                (vk::Filter::NEAREST, vk::SamplerMipmapMode::NEAREST)
            }
            SamplerId::LinearClamp | SamplerId::LinearRepeat => {
                (vk::Filter::LINEAR, vk::SamplerMipmapMode::LINEAR)
            }
        };
        let address = match self {
            SamplerId::NearestClamp | SamplerId::LinearClamp => {
                vk::SamplerAddressMode::CLAMP_TO_EDGE
            }
            SamplerId::NearestRepeat | SamplerId::LinearRepeat => vk::SamplerAddressMode::REPEAT,
        };
        vk::SamplerCreateInfo::default()
            .mag_filter(filter)
            .min_filter(filter)
            .mipmap_mode(mipmap)
            .address_mode_u(address)
            .address_mode_v(address)
            .address_mode_w(address)
            .max_lod(vk::LOD_CLAMP_NONE)
    }
}

/// When a texture's first upload has to be visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrival {
    /// In the next frame: the copy is recorded on the graphics queue, before the frame's
    /// passes (what the first frame draws, the missing texture).
    NextFrame,
    /// A frame or more later is fine (atlases behind a loading screen, skins): on a
    /// transfer queue if there is one. Check [`TextureTable::is_ready`] before drawing it;
    /// until then its slot shows the missing texture.
    Background,
}

/// What texture to create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureDesc {
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
    /// An uncompressed colour format (see `texel_bytes`); RGBA8 UNORM for game textures,
    /// which hold display-encoded colours like everything else Ruthenium draws.
    pub format: vk::Format,
}

impl TextureDesc {
    /// One RGBA8 level of `width` × `height`.
    pub fn rgba8(width: u32, height: u32) -> TextureDesc {
        TextureDesc {
            width,
            height,
            mip_levels: 1,
            format: vk::Format::R8G8B8A8_UNORM,
        }
    }

    fn extent(&self) -> vk::Extent2D {
        vk::Extent2D {
            width: self.width,
            height: self.height,
        }
    }

    /// Why the device can't hold this texture, if it can't.
    fn check(&self, max_dimension: u32) -> Result<(), String> {
        if texel_bytes(self.format).is_none() {
            return Err(format!("textures can't be {:?}", self.format));
        }
        if self.width == 0 || self.height == 0 {
            return Err("a texture can't be empty".into());
        }
        if self.width.max(self.height) > max_dimension {
            return Err(format!(
                "a {}x{} texture is larger than this GPU's limit of {max_dimension}",
                self.width, self.height
            ));
        }
        let full_chain = 32 - self.width.max(self.height).leading_zeros();
        if self.mip_levels == 0 || self.mip_levels > full_chain {
            return Err(format!(
                "a {}x{} texture can't have {} mip levels",
                self.width, self.height, self.mip_levels
            ));
        }
        Ok(())
    }
}

/// Where a texture's first upload is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Not staged yet.
    Empty,
    /// Staged; visible once the ticket is ready.
    Arriving(UploadTicket),
    /// Its slot shows it.
    Ready,
}

struct Texture {
    image: Image,
    desc: TextureDesc,
    state: State,
    /// A render target ([`TextureTable::create_target`]): passes draw into it.
    target: bool,
}

/// Free indices of the table. Index 0 is the missing texture and never handed out.
#[derive(Debug)]
struct Slots {
    capacity: u32,
    /// Indices below this have been handed out at least once.
    next: u32,
    free: Vec<u32>,
}

impl Slots {
    fn new(capacity: u32) -> Slots {
        Slots {
            capacity,
            next: 1,
            free: Vec::new(),
        }
    }

    fn take(&mut self) -> Option<u32> {
        if let Some(i) = self.free.pop() {
            return Some(i);
        }
        (self.next < self.capacity).then(|| {
            self.next += 1;
            self.next - 1
        })
    }

    fn give_back(&mut self, i: u32) {
        debug_assert!(i != 0 && i < self.next && !self.free.contains(&i));
        self.free.push(i);
    }
}

/// How full the table is and has been.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureTableStats {
    /// Slots, including the missing texture's.
    pub capacity: u32,
    /// Textures that exist now, the missing texture included.
    pub live: u32,
    /// Destroyed textures whose slots are not free yet.
    pub retiring: u32,
    /// The most there have been of each, and of both together (slots taken at once).
    pub peak_live: u32,
    pub peak_retiring: u32,
    pub peak_taken: u32,
    /// Textures that could not be created because every slot was taken.
    pub refused: u64,
}

impl TextureTableStats {
    fn count(&mut self, live: i32, retiring: i32) {
        self.live = self.live.saturating_add_signed(live);
        self.retiring = self.retiring.saturating_add_signed(retiring);
        self.peak_live = self.peak_live.max(self.live);
        self.peak_retiring = self.peak_retiring.max(self.retiring);
        self.peak_taken = self.peak_taken.max(self.live + self.retiring);
    }
}

/// The shared texture table. See the module docs.
pub struct TextureTable {
    gpu: Arc<Gpu>,
    samplers: [vk::Sampler; SAMPLER_COUNT as usize],
    layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    sets: [vk::DescriptorSet; MAX_FRAMES_IN_FLIGHT],
    textures: Vec<Option<Texture>>,
    slots: Slots,
    /// Destroyed textures, freed once the frame number they carry has finished.
    retiring: Vec<(u64, u32, Texture)>,
    /// Textures whose first upload is staged but not ready, by index.
    arriving: Vec<u32>,
    /// Destroyed textures whose first upload is not ready yet: they retire once it is.
    orphans: Vec<(UploadTicket, u32, Texture)>,
    /// Per set: (index, view) writes to make at the start of that set's next frame.
    pending: [Vec<(u32, vk::ImageView)>; MAX_FRAMES_IN_FLIGHT],
    /// Scratch for those writes, [`WRITE_BATCH`] at a time: the image infos, then the
    /// writes that point at them. Both are empty between calls.
    write_infos: Vec<vk::DescriptorImageInfo>,
    writes: Vec<vk::WriteDescriptorSet<'static>>,
    /// Per set: whether each slot shows a texture of its own (not the missing texture),
    /// as last written.
    shown: [Vec<bool>; MAX_FRAMES_IN_FLIGHT],
    /// Number of the last frame begun, and its set.
    frame: u64,
    set: usize,
    path: TextureTablePath,
    stats: TextureTableStats,
}

impl fmt::Debug for TextureTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextureTable")
            .field("path", &self.path)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

// SAFETY: Vulkan handles may be used from any thread; all mutation goes through &mut self.
// The pointers in `writes` are only alive inside `flush_writes`, which empties it.
unsafe impl Send for TextureTable {}

impl TextureTable {
    /// The two colours of the missing texture's checkerboard (RGBA), top left first.
    pub const MISSING_COLORS: [[u8; 4]; 2] = MISSING_COLORS;

    /// Creates the table with the slots and the kind of descriptor set chosen for the
    /// device (`DeviceCaps::texture_slots`, `DeviceCaps::texture_table`) and stages the
    /// missing texture into `upload`, so it is ready in the first frame.
    pub fn new(gpu: Arc<Gpu>, upload: &mut UploadRing) -> Result<TextureTable, Error> {
        let capacity = gpu.caps().texture_slots;
        let path = gpu.caps().texture_table;
        let mut t = TextureTable {
            samplers: [vk::Sampler::null(); SAMPLER_COUNT as usize],
            layout: vk::DescriptorSetLayout::null(),
            pool: vk::DescriptorPool::null(),
            sets: [vk::DescriptorSet::null(); MAX_FRAMES_IN_FLIGHT],
            textures: Vec::with_capacity(256),
            slots: Slots::new(capacity),
            retiring: Vec::new(),
            arriving: Vec::new(),
            orphans: Vec::new(),
            pending: std::array::from_fn(|_| Vec::new()),
            write_infos: Vec::with_capacity(WRITE_BATCH),
            writes: Vec::with_capacity(WRITE_BATCH),
            shown: std::array::from_fn(|_| vec![false; capacity as usize]),
            frame: 0,
            set: 0,
            path,
            stats: TextureTableStats {
                capacity,
                ..TextureTableStats::default()
            },
            gpu,
        };
        // Drop destroys whatever was created if a step fails.
        t.create_descriptors(capacity)?;
        let missing = t.create_texture(TextureDesc::rgba8(MISSING_SIZE, MISSING_SIZE))?;
        t.textures.push(Some(missing));
        t.stats.count(1, 0);
        let pixels = missing_pixels();
        t.upload(upload, TextureId::MISSING, &[&pixels], Arrival::NextFrame)?;
        // Every slot starts on the missing texture (written directly: no set is in use).
        let view = t.textures[0]
            .as_ref()
            .map(|m| m.image.view)
            .expect("missing texture");
        let infos = vec![
            vk::DescriptorImageInfo::default()
                .image_view(view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
            capacity as usize
        ];
        let writes = t.sets.map(|set| {
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(TEXTURE_BINDING)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&infos)
        });
        // SAFETY: live sets that no command buffer uses yet; the view is live.
        unsafe { t.gpu.device.update_descriptor_sets(&writes, &[]) };
        for p in &mut t.pending {
            p.clear();
        }
        Ok(t)
    }

    fn create_descriptors(&mut self, capacity: u32) -> Result<(), Error> {
        let d = &self.gpu.device;
        for (i, s) in SamplerId::ALL.iter().enumerate() {
            // SAFETY: plain object creation; destroyed in Drop.
            self.samplers[i] =
                unsafe { d.create_sampler(&s.info(), None) }.ctx("vkCreateSampler")?;
        }
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(SAMPLER_BINDING)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(SAMPLER_COUNT)
                .stage_flags(STAGES)
                .immutable_samplers(&self.samplers),
            vk::DescriptorSetLayoutBinding::default()
                .binding(TEXTURE_BINDING)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(capacity)
                .stage_flags(STAGES),
        ];
        // Partially bound, so the driver doesn't validate slots a draw doesn't touch.
        // Update-after-bind only for its higher descriptor limits: the binding, the layout
        // and the pool carry it together, or none of them does.
        let (texture_flags, layout_flags, pool_flags) = match self.path {
            TextureTablePath::Ordinary => (
                vk::DescriptorBindingFlags::PARTIALLY_BOUND,
                vk::DescriptorSetLayoutCreateFlags::empty(),
                vk::DescriptorPoolCreateFlags::empty(),
            ),
            TextureTablePath::UpdateAfterBind => (
                vk::DescriptorBindingFlags::PARTIALLY_BOUND
                    | vk::DescriptorBindingFlags::UPDATE_AFTER_BIND,
                vk::DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL,
                vk::DescriptorPoolCreateFlags::UPDATE_AFTER_BIND,
            ),
        };
        let flags = [vk::DescriptorBindingFlags::empty(), texture_flags];
        let mut binding_flags =
            vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(&flags);
        let info = vk::DescriptorSetLayoutCreateInfo::default()
            .flags(layout_flags)
            .bindings(&bindings)
            .push(&mut binding_flags);
        // SAFETY: plain object creation; destroyed in Drop.
        self.layout = unsafe { d.create_descriptor_set_layout(&info, None) }
            .ctx("vkCreateDescriptorSetLayout")?;

        let sets = MAX_FRAMES_IN_FLIGHT as u32;
        let sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLER,
                descriptor_count: SAMPLER_COUNT * sets,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: capacity * sets,
            },
        ];
        let info = vk::DescriptorPoolCreateInfo::default()
            .flags(pool_flags)
            .max_sets(sets)
            .pool_sizes(&sizes);
        // SAFETY: plain object creation; destroyed in Drop.
        self.pool =
            unsafe { d.create_descriptor_pool(&info, None) }.ctx("vkCreateDescriptorPool")?;
        let layouts = [self.layout; MAX_FRAMES_IN_FLIGHT];
        let info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.pool)
            .set_layouts(&layouts);
        // SAFETY: the pool has room for exactly these sets; they are freed with it.
        let sets = unsafe { d.allocate_descriptor_sets(&info) }.ctx("vkAllocateDescriptorSets")?;
        self.sets.copy_from_slice(&sets);
        Ok(())
    }

    /// The layout of set 0, for the shared pipeline layout.
    pub fn layout(&self) -> vk::DescriptorSetLayout {
        self.layout
    }

    /// Number of slots, including the missing texture's.
    pub fn capacity(&self) -> u32 {
        self.slots.capacity
    }

    /// The table's part of the pipelines' interface, for the pipeline cache's name: its
    /// bindings, its size and kind (both are in the set layout), and every sampler.
    pub(crate) fn hash_interface(&self, h: &mut crate::pipeline::InterfaceHash) {
        h.u32(SAMPLER_BINDING)
            .u32(TEXTURE_BINDING)
            .u32(STAGES.as_raw())
            .u32(self.slots.capacity)
            .u32(match self.path {
                TextureTablePath::Ordinary => 0,
                TextureTablePath::UpdateAfterBind => 1,
            })
            .u32(SAMPLER_COUNT);
        for s in SamplerId::ALL {
            let i = s.info();
            h.u32(i.mag_filter.as_raw() as u32)
                .u32(i.min_filter.as_raw() as u32)
                .u32(i.mipmap_mode.as_raw() as u32)
                .u32(i.address_mode_u.as_raw() as u32)
                .u32(i.address_mode_v.as_raw() as u32)
                .u32(i.address_mode_w.as_raw() as u32)
                .u32(i.anisotropy_enable)
                .u32(i.max_anisotropy.to_bits())
                .u32(i.min_lod.to_bits())
                .u32(i.max_lod.to_bits());
        }
    }

    /// The kind of descriptor set the table is made of.
    pub fn path(&self) -> TextureTablePath {
        self.path
    }

    /// How full the table is and has been.
    pub fn stats(&self) -> TextureTableStats {
        self.stats
    }

    /// Whether slot `index` shows a texture of its own in the frame last begun, as that
    /// frame's copy of the set was written; if not (a free slot, a texture still arriving
    /// or created since, an index outside the table) it shows the missing texture. A
    /// texture destroyed during the frame is still shown in it.
    pub fn shows(&self, index: u32) -> bool {
        self.shown[self.set]
            .get(index as usize)
            .copied()
            .unwrap_or(false)
    }

    /// A free slot, or why there is none.
    fn take_slot(&mut self) -> Result<u32, Error> {
        match self.slots.take() {
            Some(i) => {
                if self.textures.len() <= i as usize {
                    self.textures.resize_with(i as usize + 1, || None);
                }
                self.stats.count(1, 0);
                Ok(i)
            }
            None => {
                if self.stats.refused == 0 {
                    eprintln!(
                        "[gpu] the texture table is full ({} slots, {} of them being freed): new textures wait for a slot",
                        self.slots.capacity, self.stats.retiring
                    );
                }
                self.stats.refused += 1;
                Err(Error::TextureTableFull {
                    capacity: self.slots.capacity,
                    retiring: self.stats.retiring,
                })
            }
        }
    }

    fn create_texture(&self, desc: TextureDesc) -> Result<Texture, Error> {
        desc.check(self.gpu.caps().max_image_dimension_2d)
            .map_err(Error::Unsupported)?;
        let image = Image::new_sampled(&self.gpu, desc.format, desc.extent(), desc.mip_levels)?;
        Ok(Texture {
            image,
            desc,
            state: State::Empty,
            target: false,
        })
    }

    /// Creates a texture. Until its contents are uploaded (`upload`), its slot shows the
    /// missing texture.
    ///
    /// With every slot taken this is [`Error::TextureTableFull`], and nothing was created.
    pub fn create(&mut self, desc: TextureDesc) -> Result<TextureId, Error> {
        desc.check(self.gpu.caps().max_image_dimension_2d)
            .map_err(Error::Unsupported)?;
        // The slot first: a full table costs no image.
        let i = self.take_slot()?;
        match self.create_texture(desc) {
            Ok(texture) => {
                self.textures[i as usize] = Some(texture);
                Ok(TextureId(i))
            }
            Err(e) => {
                self.slots.give_back(i);
                self.stats.count(-1, 0);
                Err(e)
            }
        }
    }

    /// Creates a texture the renderer draws into (a render target) instead of uploading
    /// to: one mip level, undefined until its first pass
    /// ([`crate::Encoder::texture_pass`], which leaves it ready for sampling). Its slot
    /// shows it from the next frame begun, so draw into it before sampling it.
    pub fn create_target(
        &mut self,
        format: vk::Format,
        extent: vk::Extent2D,
    ) -> Result<TextureId, Error> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            mip_levels: 1,
            format,
        };
        desc.check(self.gpu.caps().max_image_dimension_2d)
            .map_err(Error::Unsupported)?;
        let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT
            | vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::TRANSFER_SRC;
        let i = self.take_slot()?;
        let image = match Image::new_2d(&self.gpu, format, extent, 1, usage) {
            Ok(image) => image,
            Err(e) => {
                self.slots.give_back(i);
                self.stats.count(-1, 0);
                return Err(e);
            }
        };
        let view = image.view;
        self.textures[i as usize] = Some(Texture {
            image,
            desc,
            state: State::Ready,
            target: true,
        });
        for p in &mut self.pending {
            p.push((i, view));
        }
        Ok(TextureId(i))
    }

    /// The image, view and description of a render target, for a pass into it.
    ///
    /// # Panics
    /// If `id` is not a live render target.
    pub(crate) fn target(&self, id: TextureId) -> (vk::Image, vk::ImageView, TextureDesc) {
        let t = self.live(id);
        assert!(t.target, "{id:?} is not a render target");
        (t.image.raw, t.image.view, t.desc)
    }

    /// Whether the texture's slot shows it in the frame last begun (and every later one).
    ///
    /// # Panics
    /// If `id` is not a live texture.
    pub fn is_ready(&self, id: TextureId) -> bool {
        self.live(id).state == State::Ready
    }

    fn live(&self, id: TextureId) -> &Texture {
        self.textures
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .unwrap_or_else(|| panic!("{id:?} is not a live texture"))
    }

    fn live_mut(&mut self, id: TextureId) -> &mut Texture {
        self.textures
            .get_mut(id.0 as usize)
            .and_then(Option::as_mut)
            .unwrap_or_else(|| panic!("{id:?} is not a live texture"))
    }

    /// The image and description of a texture, for reading it back.
    pub(crate) fn image(&self, id: TextureId) -> (vk::Image, TextureDesc) {
        let t = self.live(id);
        (t.image.raw, t.desc)
    }

    /// Stages every mip level of the texture (`mips[0]` is level 0; tightly packed rows,
    /// top first) into `upload`. The first upload makes the texture visible when
    /// `arrival` says; later ones replace its contents in the next frame (`arrival` is
    /// then ignored). On `Err(Full)` nothing was staged: try again next frame; on
    /// `Err(NotReady)`, once the first upload has arrived.
    ///
    /// # Panics
    /// If `id` is not a live texture, or `mips` doesn't match its levels and sizes.
    pub fn upload(
        &mut self,
        upload: &mut UploadRing,
        id: TextureId,
        mips: &[&[u8]],
        arrival: Arrival,
    ) -> Result<(), UploadError> {
        let t = self.live_mut(id);
        assert_eq!(
            mips.len(),
            t.desc.mip_levels as usize,
            "mip levels of {id:?}"
        );
        let mut regions = [(ImageRegion::default(), &[][..]); MAX_MIP_LEVELS];
        for (level, data) in mips.iter().enumerate() {
            regions[level] = (ImageRegion::whole_mip(t.desc.extent(), level as u32), data);
        }
        let state = t.state;
        if let State::Arriving(_) = state {
            return Err(UploadError::NotReady);
        }
        let dst = ImageDst {
            image: t.image.raw,
            format: t.desc.format,
            extent: t.desc.extent(),
            mip_levels: t.desc.mip_levels,
            fresh: state == State::Empty,
            background: arrival == Arrival::Background,
        };
        let ticket = upload.upload_image(&dst, &regions[..mips.len()])?;
        if state == State::Empty {
            t.state = State::Arriving(ticket);
            self.arriving.push(id.0);
        }
        Ok(())
    }

    /// Stages rectangles of a visible texture (each region's texels tightly packed, rows
    /// top first): animation frames, atlas patches. The next frame samples the new texels;
    /// the rest of the texture keeps its contents. Either every region is staged or none
    /// is. On `Err(Full)` try again next frame; on `Err(NotReady)`, once
    /// [`Self::is_ready`].
    ///
    /// # Panics
    /// If `id` is not a live texture, or a region lies outside its mip level or its data
    /// is not exactly its size.
    pub fn update(
        &mut self,
        upload: &mut UploadRing,
        id: TextureId,
        regions: &[(ImageRegion, &[u8])],
    ) -> Result<(), UploadError> {
        let t = self.live(id);
        if t.state != State::Ready {
            return Err(UploadError::NotReady);
        }
        let dst = ImageDst {
            image: t.image.raw,
            format: t.desc.format,
            extent: t.desc.extent(),
            mip_levels: t.desc.mip_levels,
            fresh: false,
            background: false,
        };
        upload.upload_image(&dst, regions).map(|_| ())
    }

    /// Creates a texture and stages its contents.
    pub fn create_with(
        &mut self,
        upload: &mut UploadRing,
        desc: TextureDesc,
        mips: &[&[u8]],
        arrival: Arrival,
    ) -> Result<TextureId, Error> {
        let id = self.create(desc)?;
        if let Err(e) = self.upload(upload, id, mips, arrival) {
            self.destroy(id);
            return Err(Error::Upload(e));
        }
        Ok(id)
    }

    /// Removes a texture. Its slot shows the missing texture from the next frame of each
    /// set on; the image is destroyed, and the slot reused, once those frames have finished.
    ///
    /// # Panics
    /// If `id` is the missing texture or not a live texture.
    pub fn destroy(&mut self, id: TextureId) {
        assert!(id != TextureId::MISSING, "the missing texture stays");
        let t = self.textures[id.0 as usize]
            .take()
            .unwrap_or_else(|| panic!("{id:?} is not a live texture"));
        match t.state {
            State::Ready => {
                let missing = self.textures[0]
                    .as_ref()
                    .map(|m| m.image.view)
                    .expect("missing");
                for p in &mut self.pending {
                    p.push((id.0, missing));
                }
            }
            State::Arriving(ticket) => {
                // Its copies may not even be submitted yet: it retires once they are done.
                self.arriving.retain(|&i| i != id.0);
                self.orphans.push((ticket, id.0, t));
                self.stats.count(-1, 1);
                return;
            }
            State::Empty => {}
        }
        self.stats.count(-1, 1);
        // Frames up to self.frame may sample it or record its staged copies, and each set
        // drops it at its own next frame: after self.frame + MAX_FRAMES_IN_FLIGHT, all have.
        self.retiring
            .push((self.frame + MAX_FRAMES_IN_FLIGHT as u64, id.0, t));
    }

    /// Shows the textures whose first upload is ready, brings frame `frame`'s copy of the
    /// set (slot `slot`) up to date and frees textures no unfinished frame can use. Call it
    /// at the start of every frame, after `upload.record` for that frame and before
    /// recording anything that samples.
    pub fn begin_frame(&mut self, slot: usize, frame: u64, completed: u64, upload: &UploadRing) {
        profiling::scope!("texture_table");
        self.frame = frame;
        self.set = slot;
        let mut i = 0;
        while i < self.arriving.len() {
            let index = self.arriving[i];
            let t = self.textures[index as usize]
                .as_mut()
                .expect("arriving textures are live");
            match t.state {
                State::Arriving(ticket) if upload.is_ready(ticket) => {
                    t.state = State::Ready;
                    let view = t.image.view;
                    for p in &mut self.pending {
                        p.push((index, view));
                    }
                    self.arriving.swap_remove(i);
                }
                _ => i += 1,
            }
        }
        let mut i = 0;
        while i < self.orphans.len() {
            if upload.is_ready(self.orphans[i].0) {
                // Its copies were recorded or acquired by this frame at the latest.
                let (_, index, t) = self.orphans.swap_remove(i);
                self.retiring
                    .push((frame + MAX_FRAMES_IN_FLIGHT as u64, index, t));
            } else {
                i += 1;
            }
        }
        self.flush_writes(slot);
        let mut i = 0;
        while i < self.retiring.len() {
            if self.retiring[i].0 <= completed {
                let (_, index, t) = self.retiring.swap_remove(i);
                // SAFETY: every frame that could sample it has finished, and both sets
                // were rewritten to the missing texture in frames that came before.
                unsafe { t.image.destroy(&self.gpu) };
                self.slots.give_back(index);
                self.stats.count(0, -1);
            } else {
                i += 1;
            }
        }
    }

    /// Makes the writes pending for `slot`'s copy of the set, in order (a slot may be
    /// written twice: shown, then back to the missing texture), [`WRITE_BATCH`] at a time
    /// through the table's scratch. Nothing to do in a steady frame: writes are pending
    /// only when textures come or go.
    fn flush_writes(&mut self, slot: usize) {
        let set = self.sets[slot];
        let missing = self.textures[0].as_ref().map(|m| m.image.view);
        for batch in self.pending[slot].chunks(WRITE_BATCH) {
            for &(index, view) in batch {
                self.shown[slot][index as usize] = Some(view) != missing;
            }
            // Every info first: the writes point into this storage, which must not move
            // (it never grows past the capacity it was created with).
            self.write_infos.extend(batch.iter().map(|&(_, view)| {
                vk::DescriptorImageInfo::default()
                    .image_view(view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            }));
            self.writes.extend(
                batch
                    .iter()
                    .zip(&self.write_infos)
                    .map(|(&(index, _), info)| {
                        let mut write = vk::WriteDescriptorSet::default()
                            .dst_set(set)
                            .dst_binding(TEXTURE_BINDING)
                            .dst_array_element(index)
                            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE);
                        write.descriptor_count = 1;
                        write.p_image_info = info;
                        write
                    }),
            );
            // SAFETY: the slot's previous frame has finished (FrameLoop::begin waited) and
            // nothing of the new one has bound this set, so no command buffer uses it;
            // writes apply in order; each points at one info in write_infos, which is not
            // touched until after the call; every view is alive (destroyed textures
            // outlive their pending writes, see destroy()).
            unsafe { self.gpu.device.update_descriptor_sets(&self.writes, &[]) };
            self.writes.clear();
            self.write_infos.clear();
        }
        self.pending[slot].clear();
    }

    /// Binds this frame's set at set 0 for graphics and compute.
    ///
    /// # Safety
    /// `cmd` is recording the frame passed to `begin_frame` with this `slot`; `layout` was
    /// created with [`Self::layout`] as set 0.
    pub(crate) unsafe fn bind(
        &self,
        cmd: vk::CommandBuffer,
        slot: usize,
        layout: vk::PipelineLayout,
    ) {
        let sets = [self.sets[slot]];
        for point in [
            vk::PipelineBindPoint::GRAPHICS,
            vk::PipelineBindPoint::COMPUTE,
        ] {
            // SAFETY: guaranteed by the caller.
            unsafe {
                self.gpu
                    .device
                    .cmd_bind_descriptor_sets(cmd, point, layout, 0, &sets, &[])
            };
        }
    }
}

impl Drop for TextureTable {
    fn drop(&mut self) {
        let gpu = &self.gpu;
        let d = &gpu.device;
        // SAFETY: the owner drops the table once the GPU is idle (the frame loop waits in
        // its Drop), so nothing uses these objects.
        unsafe {
            for t in self.textures.drain(..).flatten() {
                t.image.destroy(gpu);
            }
            for (_, _, t) in self.retiring.drain(..) {
                t.image.destroy(gpu);
            }
            for (_, _, t) in self.orphans.drain(..) {
                t.image.destroy(gpu);
            }
            if self.pool != vk::DescriptorPool::null() {
                d.destroy_descriptor_pool(self.pool, None);
            }
            if self.layout != vk::DescriptorSetLayout::null() {
                d.destroy_descriptor_set_layout(self.layout, None);
            }
            for &s in &self.samplers {
                if s != vk::Sampler::null() {
                    d.destroy_sampler(s, None);
                }
            }
        }
    }
}

/// The missing texture: a 2×2 checkerboard of 8×8-texel squares, magenta top left.
fn missing_pixels() -> Vec<u8> {
    let half = MISSING_SIZE / 2;
    (0..MISSING_SIZE * MISSING_SIZE)
        .flat_map(|i| {
            let (x, y) = (i % MISSING_SIZE, i / MISSING_SIZE);
            MISSING_COLORS[usize::from((x < half) != (y < half))]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_skip_the_missing_texture_and_are_reused() {
        let mut s = Slots::new(4);
        assert_eq!(s.take(), Some(1));
        assert_eq!(s.take(), Some(2));
        assert_eq!(s.take(), Some(3));
        assert_eq!(s.take(), None);
        s.give_back(2);
        assert_eq!(s.take(), Some(2));
        assert_eq!(s.take(), None);
    }

    #[test]
    fn texture_descriptions_are_checked() {
        let ok = TextureDesc::rgba8(256, 64);
        assert!(ok.check(16384).is_ok());
        assert!(
            TextureDesc {
                mip_levels: 9,
                ..ok
            }
            .check(16384)
            .is_ok()
        );
        assert!(
            TextureDesc {
                mip_levels: 10,
                ..ok
            }
            .check(16384)
            .is_err()
        );
        assert!(
            TextureDesc {
                mip_levels: 0,
                ..ok
            }
            .check(16384)
            .is_err()
        );
        assert!(ok.check(128).is_err());
        assert!(TextureDesc::rgba8(0, 4).check(16384).is_err());
        assert!(
            TextureDesc {
                format: vk::Format::BC1_RGB_UNORM_BLOCK,
                ..ok
            }
            .check(16384)
            .is_err()
        );
    }

    #[test]
    fn missing_texture_is_a_checkerboard() {
        let p = missing_pixels();
        assert_eq!(p.len(), (MISSING_SIZE * MISSING_SIZE * 4) as usize);
        let at = |x: u32, y: u32| {
            let i = ((y * MISSING_SIZE + x) * 4) as usize;
            [p[i], p[i + 1], p[i + 2], p[i + 3]]
        };
        assert_eq!(at(0, 0), MISSING_COLORS[0]);
        assert_eq!(at(15, 15), MISSING_COLORS[0]);
        assert_eq!(at(8, 0), MISSING_COLORS[1]);
        assert_eq!(at(0, 8), MISSING_COLORS[1]);
    }

    #[test]
    fn samplers_stay_inside_the_portability_subset() {
        // No LOD bias (samplerMipLodBias) and no comparison (mutableComparisonSamplers):
        // a sampler that needs one has to check the device's PortabilitySubset first.
        for s in SamplerId::ALL {
            let i = s.info();
            assert_eq!(i.mip_lod_bias, 0.0, "{s:?}");
            assert_eq!(i.compare_enable, vk::FALSE, "{s:?}");
            assert_eq!(i.anisotropy_enable, vk::FALSE, "{s:?}");
        }
    }

    #[test]
    fn sampler_ids_are_dense() {
        for (i, s) in SamplerId::ALL.iter().enumerate() {
            assert_eq!(s.index(), i as u32);
        }
    }
}
