//! The clouds: a layer of cells 12 blocks wide and 4 high read from the clouds picture,
//! drifting with the game's time, as 26.3's `CloudRenderer` builds and draws them.
//!
//! Each opaque texel of the picture is a cell. Around the camera, out to the Cloud
//! Distance, the cells' faces are listed from the middle outwards: with Fancy clouds the
//! top and bottom faces the camera can see, the sides that face it and border an empty
//! cell, and inside faces around the camera's own cell; with Fast clouds only flat
//! squares. The list is made again only when the layer has moved a whole cell under the
//! camera, the camera has gone from below the layer to inside or above it (or back), or
//! the options changed: between those, the frame only moves the layer by what is left of
//! a cell. A face is one instance of six vertices, its corners and colour worked out in
//! the vertex shader from a word.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use ruthenium_gpu::{
    BufferPool, Completed, FrameData, Gpu, Pass, PoolConfig, RangeHandle, UploadRing, UploadTicket,
    Urgency, Users, VertexAttribute, VertexInput, vk,
};
use ruthenium_shaders::interface::WORLD_MATRIX;

use crate::atmosphere::fog_line;
use crate::pipelines::PipelineId;
use crate::terrain::View;

/// How wide and deep a cell is, and how high, in blocks.
pub const CELL_SIZE: f32 = 12.0;
pub const CELL_HEIGHT: f32 = 4.0;
/// Game ticks the layer takes to drift one cell to the east.
const TICKS_PER_CELL: i64 = 400;
/// How far the layer drifts each tick (a cell every 400 ticks), and how far it is
/// moved south once and for all.
const DRIFT_PER_TICK: f32 = 0.030_000_001;
const SOUTH_SHIFT: f64 = 3.960_000_038_146_972_7;
/// A texel with less alpha than this is no cloud.
const EMPTY_ALPHA: u8 = 10;

/// What the clouds look like (the Clouds option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloudMode {
    Off,
    /// Flat squares at the layer's bottom.
    Fast,
    /// Cells with sides, top and bottom.
    #[default]
    Fancy,
}

/// The Clouds and Cloud Distance options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudOptions {
    pub mode: CloudMode,
    /// How far out clouds are drawn, in chunks (2 to 128).
    pub range: u32,
}

impl Default for CloudOptions {
    fn default() -> CloudOptions {
        CloudOptions {
            mode: CloudMode::Fancy,
            range: 128,
        }
    }
}

/// The cells of the clouds picture: for each texel, whether it is a cloud, and which of
/// its four neighbours are not (`CloudRenderer.prepare`'s packing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudCells {
    width: u32,
    height: u32,
    /// [`CELL`] for a cloud, with the [`NORTH_EMPTY`]... bits of its empty neighbours.
    cells: Vec<u8>,
}

const WEST_EMPTY: u8 = 1;
const SOUTH_EMPTY: u8 = 1 << 1;
const EAST_EMPTY: u8 = 1 << 2;
const NORTH_EMPTY: u8 = 1 << 3;
const CELL: u8 = 1 << 4;

impl CloudCells {
    /// The cells of a picture `width` by `height`, as red, green, blue and alpha bytes.
    /// Vanilla finds the east and west neighbours modulo the picture's height, not its
    /// width (the same for its square picture); a neighbour that falls outside a picture
    /// wider than it is high counts as empty here, where the game's would fail to load.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<CloudCells> {
        let (w, h) = (width as i64, height as i64);
        if w == 0 || h == 0 || rgba.len() != (w * h * 4) as usize {
            return None;
        }
        let empty = |x: i64, y: i64| {
            if x >= w || y >= h {
                return true;
            }
            rgba[((x + y * w) * 4 + 3) as usize] < EMPTY_ALPHA
        };
        let mut cells = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                if empty(x, y) {
                    continue;
                }
                let mut c = CELL;
                if empty(x, (y - 1).rem_euclid(h)) {
                    c |= NORTH_EMPTY;
                }
                if empty((x + 1).rem_euclid(h), y) {
                    c |= EAST_EMPTY;
                }
                if empty(x, (y + 1).rem_euclid(h)) {
                    c |= SOUTH_EMPTY;
                }
                if empty((x - 1).rem_euclid(h), y) {
                    c |= WEST_EMPTY;
                }
                cells[(x + y * w) as usize] = c;
            }
        }
        Some(CloudCells {
            width,
            height,
            cells,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    fn at(&self, x: i32, z: i32) -> u8 {
        let x = x.rem_euclid(self.width as i32) as usize;
        let z = z.rem_euclid(self.height as i32) as usize;
        self.cells[x + z * self.width as usize]
    }
}

/// Where the camera is from the layer (`CloudRenderer.RelativeCameraPos`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativePos {
    Above,
    Inside,
    Below,
}

/// Where the layer is this frame, seen from the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The cell under the camera, in the picture's cells.
    pub cell: [i32; 2],
    /// How far into that cell the camera is, in blocks east and south.
    pub in_cell: [f32; 2],
    /// The layer's bottom, in blocks above the camera.
    pub bottom: f32,
    pub relative: RelativePos,
}

/// Where the layer whose bottom is at `bottom_y` is from a camera at `camera`, at
/// `game_time` and `partial` of the way to the next tick, over a picture of `width` by
/// `height` cells (the first half of `CloudRenderer.prepare`).
pub fn placement(
    width: u32,
    height: u32,
    camera: [f64; 3],
    bottom_y: f32,
    game_time: i64,
    partial: f32,
) -> Placement {
    let bottom = (f64::from(bottom_y) - camera[1]) as f32;
    let top = bottom + CELL_HEIGHT;
    let relative = if top < 0.0 {
        RelativePos::Above
    } else if bottom > 0.0 {
        RelativePos::Below
    } else {
        RelativePos::Inside
    };
    let offset = (game_time % (i64::from(width) * TICKS_PER_CELL)) as f32 + partial;
    let mut x = camera[0] + f64::from(offset * DRIFT_PER_TICK);
    let mut z = camera[2] + SOUTH_SHIFT;
    let wide = f64::from(width) * f64::from(CELL_SIZE);
    let deep = f64::from(height) * f64::from(CELL_SIZE);
    x -= f64::from((x / wide).floor() as i32) * wide;
    z -= f64::from((z / deep).floor() as i32) * deep;
    let cell = [(x / 12.0).floor() as i32, (z / 12.0).floor() as i32];
    // The cell's corner is worked out in single precision, as the game does.
    let in_cell = [
        (x - f64::from(cell[0] as f32 * CELL_SIZE)) as f32,
        (z - f64::from(cell[1] as f32 * CELL_SIZE)) as f32,
    ];
    Placement {
        cell,
        in_cell,
        bottom,
        relative,
    }
}

/// How many cells out the layer reaches for a Cloud Distance of `range` chunks.
pub fn radius_cells(range: u32) -> i32 {
    (range as f32 * 16.0 / CELL_SIZE).ceil() as i32
}

/// A face's direction (`Direction.get3DDataValue`).
pub(crate) const DOWN: u32 = 0;
pub(crate) const UP: u32 = 1;
pub(crate) const NORTH: u32 = 2;
pub(crate) const SOUTH: u32 = 3;
pub(crate) const WEST: u32 = 4;
pub(crate) const EAST: u32 = 5;
/// Bits of a face word: the cell east and south of the camera's cell, offset by 512, in
/// ten bits each; then the direction, and two flags.
const FACE_DIRECTION_SHIFT: u32 = 20;
pub const FACE_INSIDE: u32 = 1 << 23;
pub const FACE_TOP_COLOUR: u32 = 1 << 24;
const FACE_BIAS: i32 = 512;

/// One face as the vertex shader reads it.
pub fn face(x: i32, z: i32, direction: u32, flags: u32) -> u32 {
    let x = (x + FACE_BIAS) as u32 & 0x3FF;
    let z = (z + FACE_BIAS) as u32 & 0x3FF;
    x | z << 10 | direction << FACE_DIRECTION_SHIFT | flags
}

/// The parts of a face word: the cell east and south, the direction and the flags.
pub fn unpack_face(word: u32) -> (i32, i32, u32, u32) {
    (
        (word & 0x3FF) as i32 - FACE_BIAS,
        (word >> 10 & 0x3FF) as i32 - FACE_BIAS,
        word >> FACE_DIRECTION_SHIFT & 7,
        word & (FACE_INSIDE | FACE_TOP_COLOUR),
    )
}

/// The faces of the cells around `centre` out to `radius` cells, nearest first, into
/// `out` (`CloudRenderer.buildMesh`): with `fancy` whole cells as seen from
/// `relative`, else flat squares.
pub fn build_mesh(
    cells: &CloudCells,
    relative: RelativePos,
    centre: [i32; 2],
    fancy: bool,
    radius: i32,
    out: &mut Vec<u32>,
) {
    out.clear();
    let mut cell = |x: i32, z: i32| {
        let c = cells.at(centre[0] + x, centre[1] + z);
        if c == 0 {
            return;
        }
        if fancy {
            extruded(out, relative, x, z, c);
        } else {
            out.push(face(x, z, DOWN, FACE_TOP_COLOUR));
        }
    };
    for ring in 0..=2 * radius {
        for x in -ring..=ring {
            let z = ring - x.abs();
            if z >= 0 && z <= radius && x * x + z * z <= radius * radius {
                if z != 0 {
                    cell(x, -z);
                }
                cell(x, z);
            }
        }
    }
}

/// One cell's faces with sides (`buildExtrudedCell`).
fn extruded(out: &mut Vec<u32>, relative: RelativePos, x: i32, z: i32, c: u8) {
    if relative != RelativePos::Below {
        out.push(face(x, z, UP, 0));
    }
    if relative != RelativePos::Above {
        out.push(face(x, z, DOWN, 0));
    }
    if c & NORTH_EMPTY != 0 && z > 0 {
        out.push(face(x, z, NORTH, 0));
    }
    if c & SOUTH_EMPTY != 0 && z < 0 {
        out.push(face(x, z, SOUTH, 0));
    }
    if c & WEST_EMPTY != 0 && x > 0 {
        out.push(face(x, z, WEST, 0));
    }
    if c & EAST_EMPTY != 0 && x < 0 {
        out.push(face(x, z, EAST, 0));
    }
    if x.abs() <= 1 && z.abs() <= 1 {
        for direction in [DOWN, UP, NORTH, SOUTH, WEST, EAST] {
            out.push(face(x, z, direction, FACE_INSIDE));
        }
    }
}

/// The layout the cloud pipelines read: one face word per instance.
pub const CLOUD_FACE: VertexInput = VertexInput {
    stride: 4,
    attributes: &[VertexAttribute {
        offset: 0,
        format: vk::Format::R32_UINT,
    }],
    per_instance: true,
};

/// What the air says of the clouds this frame (the `cloud_color` and `cloud_height`
/// attributes at the camera).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloudLook {
    /// Red, green, blue and alpha, 0 to 1.
    pub colour: [f32; 4],
    /// The layer's bottom, in blocks.
    pub height: f32,
}

impl CloudLook {
    /// The colour as the game hands it to the shader: each channel cut to a step of 255
    /// (`ARGB.colorFromVector4f`, then back to floats).
    pub fn colour_steps(&self) -> [f32; 4] {
        let step = |v: f32| (v * 255.0).floor() as i32;
        let [r, g, b, a] = self.colour.map(step);
        let packed = (a << 24 | r << 16 | g << 8 | b) as u32;
        [16, 8, 0, 24].map(|shift| (packed >> shift & 0xFF) as f32 / 255.0)
    }
}

/// A list of faces on the GPU, and what it was made for.
#[derive(Debug)]
struct Mesh {
    range: Option<RangeHandle>,
    faces: u32,
    cell: [i32; 2],
    mode: CloudMode,
    upload: Option<UploadTicket>,
    /// The last frame that drew it.
    last_frame: u64,
}

/// What a list was made for: when any of it changes, the list is made again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Built {
    cell: [i32; 2],
    relative: RelativePos,
    options: CloudOptions,
}

/// The layer as this frame draws it.
#[derive(Debug, Clone, Copy)]
struct Drawn {
    placement: Placement,
    colour: [f32; 4],
}

/// A list for the worker to make, with the bytes of an earlier one to make it in.
struct Job {
    cells: Arc<CloudCells>,
    built: Built,
    bytes: Vec<u8>,
}

/// A list the worker made: its faces' words as bytes.
struct Done {
    built: Built,
    bytes: Vec<u8>,
}

/// The clouds' lists of faces, made when vanilla makes them (on a worker thread, so that
/// no frame waits for one), and their draws.
pub(crate) struct CloudRenderer {
    cells: Arc<CloudCells>,
    pool: BufferPool,
    jobs: Option<Sender<Job>>,
    done: Receiver<Done>,
    worker: Option<JoinHandle<()>>,
    /// What the last list asked for was made for, whether the worker is on it, and a
    /// list made that the upload ring had no room for yet.
    requested: Option<Built>,
    in_flight: bool,
    waiting: Option<Done>,
    /// Bytes of a list already copied, for the next.
    spare: Vec<u8>,
    /// The list drawn, and the one on its way to the GPU to take its place.
    current: Option<Mesh>,
    next: Option<Mesh>,
    frame: Option<Drawn>,
}

impl std::fmt::Debug for CloudRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudRenderer")
            .field("requested", &self.requested)
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

impl Drop for CloudRenderer {
    fn drop(&mut self) {
        // Without jobs the worker ends.
        self.jobs = None;
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// Bytes of a page of face lists: two of the largest (radius 171) fit. Lists start at
/// multiples of the granule.
const PAGE_BYTES: u64 = 4 << 20;
const POOL_GRANULE: u64 = 256;

/// Makes the lists asked for until there are no more jobs.
fn work(jobs: Receiver<Job>, done: Sender<Done>) {
    let mut faces = Vec::new();
    while let Ok(mut job) = jobs.recv() {
        let b = job.built;
        build_mesh(
            &job.cells,
            b.relative,
            b.cell,
            b.options.mode == CloudMode::Fancy,
            radius_cells(b.options.range),
            &mut faces,
        );
        job.bytes.clear();
        job.bytes.extend(faces.iter().flat_map(|f| f.to_ne_bytes()));
        if done
            .send(Done {
                built: b,
                bytes: job.bytes,
            })
            .is_err()
        {
            return;
        }
    }
}

impl CloudRenderer {
    /// With the cells of the clouds picture: makes the page lists go to and starts the
    /// worker. Not on a frame's path.
    pub fn new(gpu: &Arc<Gpu>, cells: CloudCells) -> Result<CloudRenderer, ruthenium_gpu::Error> {
        let mut pool = BufferPool::new(
            gpu.clone(),
            PoolConfig {
                page_bytes: PAGE_BYTES,
                budget_bytes: 2 * PAGE_BYTES,
                low_watermark: 0,
                high_watermark: PAGE_BYTES,
                release_after: u32::MAX,
                granule: POOL_GRANULE,
            },
            vk::BufferUsageFlags::VERTEX_BUFFER,
        );
        pool.grow_now()?;
        let (jobs, job_rx) = channel();
        let (done_tx, done) = channel();
        let worker = std::thread::Builder::new()
            .name("clouds".into())
            .spawn(move || work(job_rx, done_tx))
            .map_err(|e| ruthenium_gpu::Error::Unsupported(format!("no clouds thread: {e}")))?;
        Ok(CloudRenderer {
            cells: Arc::new(cells),
            pool,
            jobs: Some(jobs),
            done,
            worker: Some(worker),
            requested: None,
            in_flight: false,
            waiting: None,
            spare: Vec::new(),
            current: None,
            next: None,
            frame: None,
        })
    }

    /// Works out where the layer is this frame, asks for its list again if vanilla would
    /// make it again (`CloudRenderer.prepare`), and stages a list the worker has made:
    /// before the frame's `UploadRing::record`. Nothing is drawn with the clouds off or
    /// fully transparent.
    pub fn stage(
        &mut self,
        uploads: &mut UploadRing,
        options: CloudOptions,
        look: &CloudLook,
        camera: [f64; 3],
        game_time: i64,
        partial: f32,
    ) {
        if let Ok(done) = self.done.try_recv() {
            self.in_flight = false;
            self.waiting = Some(done);
        }
        if let Some(done) = self.waiting.take() {
            self.upload(done, uploads);
        }
        let colour = look.colour_steps();
        if options.mode == CloudMode::Off || colour[3] <= 0.0 {
            self.frame = None;
            return;
        }
        let (w, h) = (self.cells.width(), self.cells.height());
        let placement = placement(w, h, camera, look.height, game_time, partial);
        self.frame = Some(Drawn { placement, colour });
        let wanted = Built {
            cell: placement.cell,
            relative: placement.relative,
            options,
        };
        if self.in_flight || self.requested == Some(wanted) {
            return;
        }
        let job = Job {
            cells: self.cells.clone(),
            built: wanted,
            bytes: std::mem::take(&mut self.spare),
        };
        if self.jobs.as_ref().is_some_and(|j| j.send(job).is_ok()) {
            self.in_flight = true;
            self.requested = Some(wanted);
        }
    }

    /// Copies a list the worker made to the GPU, or keeps it for the next frame if there
    /// is no room now.
    fn upload(&mut self, done: Done, uploads: &mut UploadRing) {
        let mut mesh = Mesh {
            range: None,
            faces: (done.bytes.len() / 4) as u32,
            cell: done.built.cell,
            mode: done.built.options.mode,
            upload: None,
            last_frame: 0,
        };
        if !done.bytes.is_empty() {
            let Ok(range) = self.pool.alloc(done.bytes.len() as u64, POOL_GRANULE) else {
                self.waiting = Some(done);
                return;
            };
            let Some(at) = self.pool.resolve(range) else {
                self.waiting = Some(done);
                return;
            };
            match uploads.upload_buffer_with(at.buffer, at.offset, &done.bytes, Urgency::Edit) {
                Ok(ticket) => {
                    mesh.range = Some(range);
                    mesh.upload = Some(ticket);
                }
                Err(_) => {
                    self.pool.retire(range, Users::NONE);
                    self.waiting = Some(done);
                    return;
                }
            }
        }
        self.spare = done.bytes;
        if let Some(old) = self.next.replace(mesh) {
            self.retire(old, uploads);
        }
    }

    /// The list on its way takes the drawn one's place once the frame may read it:
    /// after the frame's `UploadRing::record`.
    pub fn note_ready(&mut self, uploads: &UploadRing) {
        let arrived = self
            .next
            .as_ref()
            .is_some_and(|m| m.upload.is_none_or(|t| uploads.is_ready(t)));
        if arrived && let Some(old) = std::mem::replace(&mut self.current, self.next.take()) {
            self.retire(old, uploads);
        }
    }

    fn retire(&mut self, mesh: Mesh, uploads: &UploadRing) {
        if let Some(range) = mesh.range {
            let mut users = Users::until_frame(mesh.last_frame);
            if let Some(t) = mesh.upload {
                users = users.and(uploads.users_of(t));
            }
            self.pool.retire(range, users);
        }
    }

    /// Takes a resource reload's clouds picture: the layer is made again from its cells
    /// (the one drawn stays until the new one is ready).
    pub fn set_cells(&mut self, cells: CloudCells) {
        self.cells = Arc::new(cells);
        self.requested = None;
    }

    /// Once a frame: takes back the lists no frame reads any more.
    pub fn maintain(&mut self, completed: &Completed) {
        self.pool.maintain(completed);
    }

    /// Draws the layer into `pass`, which has a depth attachment: after the world's
    /// translucent layer, as vanilla. `fog_end` is where the clouds have faded out.
    ///
    /// # Safety
    /// `pass` is of frame `frame`, being recorded, whose `UploadRing::record` and
    /// [`Self::note_ready`] have run.
    pub unsafe fn draw(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        view: &View,
        fog_end: f32,
        frame: u64,
    ) {
        let (Some(drawn), Some(mesh)) = (self.frame, &mut self.current) else {
            return;
        };
        let Some(range) = mesh.range.and_then(|r| self.pool.resolve(r)) else {
            return;
        };
        let Some((matrix, bytes)) = frame_data.alloc(WORLD_MATRIX.words(), 4) else {
            return;
        };
        for (i, c) in view.view_projection.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&c.to_ne_bytes());
        }
        let (w, h) = (self.cells.width(), self.cells.height());
        let offset = mesh_offset(&drawn.placement, mesh.cell, w, h);
        let push = cloud_push(matrix, drawn.colour, offset, fog_end);
        let pipeline = if mesh.mode == CloudMode::Fancy {
            PipelineId::CloudsFancy
        } else {
            PipelineId::CloudsFast
        };
        mesh.last_frame = frame;
        // SAFETY: the list's copy can be read (`note_ready`), every instance drawn lies in
        // it, and the matrix record was written just above; the rest is the caller's.
        unsafe {
            pass.bind_vertex_buffer(range.buffer, range.offset);
            pass.draw_instances(pipeline.index(), &push, 6, 0, mesh.faces);
        }
    }
}

/// Where a list made around `cell` starts from the camera, in blocks, with the layer
/// placed as `p` over a picture of `width` by `height` cells (vanilla's `CloudOffset`):
/// the list's cells are counted from the cell it was made around, which is the camera's
/// unless a new list is still on its way, the shorter way round the picture.
pub fn mesh_offset(p: &Placement, cell: [i32; 2], width: u32, height: u32) -> [f32; 3] {
    let off = |d: i32, size: u32| {
        let size = size as i32;
        (d + size / 2).rem_euclid(size) - size / 2
    };
    [
        off(cell[0] - p.cell[0], width) as f32 * CELL_SIZE - p.in_cell[0],
        p.bottom,
        off(cell[1] - p.cell[1], height) as f32 * CELL_SIZE - p.in_cell[1],
    ]
}

/// Bytes of the clouds' push constants.
const CLOUD_PUSH_BYTES: usize = 48;

/// The push constants of `clouds.vert` and `clouds.frag`.
pub(crate) fn cloud_push(
    matrix: u32,
    colour: [f32; 4],
    offset: [f32; 3],
    fog_end: f32,
) -> [u8; CLOUD_PUSH_BYTES] {
    let mut push = [0u8; CLOUD_PUSH_BYTES];
    let mut put = |at: usize, word: [u8; 4]| push[at..at + 4].copy_from_slice(&word);
    put(0, matrix.to_ne_bytes());
    put(4, 1u32.to_ne_bytes());
    for (i, c) in colour.iter().enumerate() {
        put(16 + i * 4, c.to_ne_bytes());
    }
    for (i, c) in offset.iter().enumerate() {
        put(32 + i * 4, c.to_ne_bytes());
    }
    // The fade from the camera to the end as a fog line, whose bias is 0.
    put(44, fog_line([0.0, fog_end])[0].to_ne_bytes());
    push
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clouds_fade_by_one_over_their_end() {
        let push = cloud_push(3, [1.0; 4], [0.5, -2.0, 0.25], 192.0);
        let float = |at: usize| f32::from_ne_bytes(push[at..at + 4].try_into().unwrap());
        assert_eq!(float(44), 1.0 / 192.0);
        assert_eq!([float(32), float(36), float(40)], [0.5, -2.0, 0.25]);
        // A fade from the camera needs no bias: clouds.frag has none.
        for end in [0.0, 1.0, 192.0, 2048.0, f32::INFINITY] {
            assert_eq!(fog_line([0.0, end])[1], 0.0, "{end}");
        }
    }

    /// The clouds golden's generator (a 64-bit LCG, as the tool's).
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: u32) -> i32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) as u32 % n) as i32
        }
    }

    fn fnv(words: &[u32]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for w in words {
            for b in w.to_le_bytes() {
                hash ^= u64::from(b);
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }
        hash
    }

    /// Case `c` of the golden: its picture, where the camera is, the mode, the centre
    /// and the radius.
    fn cases() -> Vec<(CloudCells, RelativePos, bool, [i32; 2], i32)> {
        const SIZES: [(u32, u32); 4] = [(16, 16), (32, 16), (64, 64), (8, 8)];
        let mut g = Lcg(0x436c_6f75_6473_4131);
        (0..crate::clouds_golden::MESHES.len())
            .map(|_| {
                let (w, h) = SIZES[g.below(4) as usize];
                let density = g.below(101);
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                for _ in 0..w * h {
                    let r = g.below(100);
                    let alpha = if r < density {
                        255
                    } else if r < density + 8 {
                        5 + g.below(10)
                    } else {
                        0
                    };
                    rgba.extend_from_slice(&[255, 255, 255, alpha as u8]);
                }
                let relative = [RelativePos::Above, RelativePos::Inside, RelativePos::Below]
                    [g.below(3) as usize];
                let fancy = g.below(2) == 0;
                let centre = [g.below(w * 3) - w as i32, g.below(h * 3) - h as i32];
                let radius = if g.below(4) == 0 {
                    1 + g.below(3)
                } else {
                    1 + g.below(30)
                };
                let cells = CloudCells::from_rgba(w, h, &rgba).expect("a whole picture");
                (cells, relative, fancy, centre, radius)
            })
            .collect()
    }

    #[test]
    fn the_faces_are_the_games() {
        let mut out = Vec::new();
        for (i, (cells, relative, fancy, centre, radius)) in cases().into_iter().enumerate() {
            build_mesh(&cells, relative, centre, fancy, radius, &mut out);
            let want = crate::clouds_golden::MESHES[i];
            assert_eq!(
                (out.len() as u32, fnv(&out)),
                want,
                "case {i}: {}x{} {relative:?} fancy {fancy} centre {centre:?} radius {radius}",
                cells.width,
                cells.height
            );
        }
    }

    #[test]
    fn faces_pack_and_unpack() {
        for (x, z) in [(0, 0), (-171, 171), (171, -171), (-1, 1)] {
            for direction in 0..6 {
                for flags in [0, FACE_INSIDE, FACE_TOP_COLOUR] {
                    assert_eq!(
                        unpack_face(face(x, z, direction, flags)),
                        (x, z, direction, flags)
                    );
                }
            }
        }
    }

    #[test]
    fn the_camera_is_below_inside_or_above_the_layer() {
        let at = |y: f64| placement(256, 256, [0.0, y, 0.0], 192.33, 0, 0.0).relative;
        assert_eq!(at(100.0), RelativePos::Below);
        // The height is a float: 192.33 is a little more, which is below it.
        assert_eq!(at(192.33), RelativePos::Below);
        assert_eq!(at(193.0), RelativePos::Inside);
        // Level with the layer's top is still inside it.
        assert_eq!(at(f64::from(192.33f32) + 4.0), RelativePos::Inside);
        assert_eq!(at(196.0), RelativePos::Inside);
        assert_eq!(at(196.5), RelativePos::Above);
    }

    #[test]
    fn the_layer_drifts_a_cell_every_400_ticks_and_wraps() {
        let p = |t: i64, partial: f32| placement(256, 256, [5.0, 70.0, 7.0], 192.33, t, partial);
        let start = p(0, 0.0);
        // 3.96 blocks south, and the camera's 5 east.
        assert_eq!(start.cell, [0, 0]);
        assert_eq!(start.in_cell, [5.0, (7.0 + SOUTH_SHIFT) as f32]);
        assert_eq!(start.bottom, (192.33f32 as f64 - 70.0) as f32);
        // 7 blocks of 12 to go east: 233.33 ticks.
        assert_eq!(p(233, 0.0).cell, [0, 0]);
        assert_eq!(p(234, 0.0).cell, [1, 0]);
        assert_eq!(p(233, 1.0).cell, [1, 0]);
        // The drift and the shift south are the game's single-precision numbers.
        let drifted = placement(256, 256, [0.0, 70.0, 0.0], 192.33, 1000, 0.0);
        assert_eq!(drifted.cell, [2, 0]);
        assert_eq!(
            drifted.in_cell,
            [
                (f64::from(1000.0f32 * 0.030_000_001) - 24.0) as f32,
                3.960_000_038_146_972_7_f64 as f32
            ]
        );
        assert_ne!(drifted.in_cell[0], 6.0);
        // The picture is 256 cells: after 102400 ticks it is where it began.
        assert_eq!(p(256 * 400, 0.0), start);
        // Far from the origin the place still wraps into the picture.
        let far = placement(256, 256, [-1e6, 70.0, 3e6], 192.33, 5, 0.5);
        assert!((0..256).contains(&far.cell[0]) && (0..256).contains(&far.cell[1]));
        assert!((0.0..12.0).contains(&far.in_cell[0]) && (0.0..12.0).contains(&far.in_cell[1]));
    }

    #[test]
    fn a_list_is_drawn_from_the_cell_it_was_made_around() {
        let p = Placement {
            cell: [0, 5],
            in_cell: [2.5, 7.0],
            bottom: 120.0,
            relative: RelativePos::Below,
        };
        // Made around the camera's cell: vanilla's offset.
        assert_eq!(mesh_offset(&p, [0, 5], 256, 256), [-2.5, 120.0, -7.0]);
        // A cell behind, across the picture's edge, and one ahead.
        assert_eq!(mesh_offset(&p, [255, 6], 256, 256), [-14.5, 120.0, 5.0]);
        assert_eq!(mesh_offset(&p, [1, 4], 256, 256), [9.5, 120.0, -19.0]);
    }

    #[test]
    fn the_colour_is_cut_to_steps_of_255() {
        let look = CloudLook {
            colour: [1.0, 0.5, 0.25, 0.8],
            height: 192.33,
        };
        assert_eq!(
            look.colour_steps(),
            [1.0, 127.0 / 255.0, 63.0 / 255.0, 204.0 / 255.0]
        );
        let clear = CloudLook {
            colour: [1.0, 1.0, 1.0, 0.003],
            height: 0.0,
        };
        assert_eq!(clear.colour_steps()[3], 0.0);
    }

    #[test]
    fn the_radius_is_the_distance_in_whole_cells() {
        assert_eq!(radius_cells(128), 171);
        assert_eq!(radius_cells(2), 3);
        assert_eq!(radius_cells(12), 16);
    }

    #[test]
    fn a_texel_with_little_alpha_is_no_cloud() {
        let rgba =
            |alphas: &[u8]| -> Vec<u8> { alphas.iter().flat_map(|a| [0, 0, 0, *a]).collect() };
        let cells = CloudCells::from_rgba(2, 1, &rgba(&[10, 9])).unwrap();
        assert_ne!(cells.at(0, 0), 0);
        assert_eq!(cells.at(1, 0), 0);
        assert!(CloudCells::from_rgba(2, 2, &rgba(&[255])).is_none());
    }
}
