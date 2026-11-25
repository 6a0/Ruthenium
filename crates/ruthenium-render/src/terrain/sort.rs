//! The order a section's translucent quads are drawn in: farthest from the camera first,
//! as 26.3 sorts them (`VertexSorting.byDistance`, `MeshData.SortState`), and when a
//! section's order is worked out again (`TranslucencyPointOfView`,
//! `LevelRenderer.scheduleResort`).
//!
//! A quad stands for the point half-way between its first and third corners. The quads
//! are sorted by that point's distance from the camera with a stable sort, so quads at
//! the same distance (a fluid's face and its other side) keep the order they were
//! meshed in. Only indices move: the vertices stay where they are.

use ruthenium_javamath::mth;
use ruthenium_sim::scene::SectionKey;

/// Quads a section's translucent layer can have with 16-bit indices; more are drawn
/// with 32-bit ones.
pub(crate) const NARROW_QUADS: usize = 1 << 14;

/// Where the camera is from a section, along each axis: in a section before it (-1),
/// level with it (0) or after it (1).
pub(crate) type PointOfView = [i8; 3];

/// `TranslucencyPointOfView.of`.
pub(crate) fn point_of_view(camera: [f64; 3], section: SectionKey) -> PointOfView {
    std::array::from_fn(|i| ((mth::floor(camera[i]) >> 4) - section[i]).clamp(-1, 1) as i8)
}

/// `isAxisAligned`: the camera is level with the section along some axis, where every
/// block it moves can change which quad is before which.
pub(crate) fn axis_aligned(pov: PointOfView) -> bool {
    pov.contains(&0)
}

/// Whether a section's quads are sorted again: its order was made for the camera in
/// the block `sorted_at` with the point of view `sorted_from`, and the camera is at
/// `camera` now. They are when the point of view is another, or when the camera is in
/// another block and level with the section along some axis. (Vanilla asks the second
/// of the frame before, not of the section's last sort, so a section that comes into
/// view keeps a stale order until the camera next leaves its block; and it also
/// schedules sections near the camera on every block, whose task then does nothing
/// unless one of the two holds.)
pub(crate) fn needs_sort(
    camera: [f64; 3],
    section: SectionKey,
    sorted_at: [i32; 3],
    sorted_from: PointOfView,
) -> bool {
    let pov = point_of_view(camera, section);
    pov != sorted_from || (axis_aligned(pov) && camera.map(mth::floor) != sorted_at)
}

/// The camera in blocks from a section's corner, as the sort has it
/// (`createVertexSorting`).
pub(crate) fn eye(camera: [f64; 3], section: SectionKey) -> [f32; 3] {
    std::array::from_fn(|i| (camera[i] - f64::from(section[i] << 4)) as f32)
}

/// The point a quad is sorted by (`MeshData.decodeQuadCentroids`): half-way between its
/// first and its third corner.
pub(crate) fn centre(first: [f32; 3], third: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (first[i] + third[i]) / 2.0)
}

/// Sorts quads; keeps its scratch memory between sorts.
#[derive(Debug, Default)]
pub(crate) struct Sorter {
    keys: Vec<f32>,
    order: Vec<u32>,
}

impl Sorter {
    /// The quads with these centres in the order they are drawn in from `eye` (both in
    /// blocks from the section's corner): the farthest first.
    pub fn order(&mut self, centres: &[[f32; 3]], eye: [f32; 3]) -> &[u32] {
        self.keys.clear();
        // `Vector3f.distanceSquared`: x's square added to the sum of the other two. (The
        // library would fuse the multiplications and additions if told to; the game
        // does not tell it.)
        self.keys.extend(centres.iter().map(|c| {
            let (dx, dy, dz) = (eye[0] - c[0], eye[1] - c[1], eye[2] - c[2]);
            dx * dx + (dy * dy + dz * dz)
        }));
        self.order.clear();
        self.order.extend(0..centres.len() as u32);
        let keys = &self.keys;
        // Stable, as the game's merge sort: `Floats.compare(keys[o2], keys[o1])`.
        self.order
            .sort_by(|&a, &b| keys[b as usize].total_cmp(&keys[a as usize]));
        &self.order
    }

    /// The indices that draw the quads in that order, two triangles each over the
    /// quad's first diagonal: 16 bits each for up to [`NARROW_QUADS`] quads, else 32.
    pub fn indices(&mut self, centres: &[[f32; 3]], eye: [f32; 3], out: &mut Vec<u8>) {
        let wide = centres.len() > NARROW_QUADS;
        out.clear();
        out.reserve(index_bytes(centres.len()));
        for &quad in self.order(centres, eye) {
            for corner in [0, 1, 2, 2, 3, 0] {
                let index = quad * 4 + corner;
                if wide {
                    out.extend_from_slice(&index.to_le_bytes());
                } else {
                    out.extend_from_slice(&(index as u16).to_le_bytes());
                }
            }
        }
    }
}

/// Bytes of the indices of so many translucent quads.
pub(crate) fn index_bytes(quads: usize) -> usize {
    quads * 6 * if quads > NARROW_QUADS { 4 } else { 2 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_point_of_view_is_the_cameras_section_from_the_sections() {
        let section = [2, -1, 0];
        // Blocks 32..48, -16..0, 0..16.
        assert_eq!(point_of_view([40.0, -8.0, 8.0], section), [0, 0, 0]);
        assert_eq!(point_of_view([31.99, -8.0, 16.0], section), [-1, 0, 1]);
        assert_eq!(point_of_view([48.0, -16.01, -0.01], section), [1, -1, -1]);
        assert_eq!(point_of_view([1000.0, 300.0, -1000.0], section), [1, 1, -1]);
        assert!(axis_aligned([0, 1, -1]) && axis_aligned([1, 1, 0]));
        assert!(!axis_aligned([1, -1, 1]));
    }

    #[test]
    fn a_section_is_sorted_again_when_the_view_changes_or_the_camera_moves_beside_it() {
        let section = [0, 0, 0];
        let sorted = |camera: [f64; 3]| (camera.map(mth::floor), point_of_view(camera, section));
        // Level with the section along y: every block the camera enters sorts again.
        let (at, from) = sorted([40.5, 8.5, 40.5]);
        assert!(!needs_sort([40.9, 8.9, 40.1], section, at, from));
        assert!(needs_sort([41.0, 8.5, 40.5], section, at, from));
        assert!(needs_sort([40.5, 9.0, 40.5], section, at, from));
        // Diagonally off along all three: only another point of view does.
        let (at, from) = sorted([40.5, 30.5, 40.5]);
        assert!(!needs_sort([45.5, 35.5, 20.5], section, at, from));
        assert!(needs_sort([45.5, 15.5, 20.5], section, at, from));
        assert!(needs_sort([15.5, 30.5, 40.5], section, at, from));
    }

    #[test]
    fn the_eye_is_taken_from_the_sections_corner_in_double_precision() {
        // Far from the origin the difference is still exact before it is rounded once.
        let camera = [30_000_000.3, 70.7, -29_999_999.9];
        let e = eye(camera, [1_875_000, 4, -1_875_000]);
        assert!(
            (e[0] - 0.3).abs() < 1e-6 && (e[1] - 6.7).abs() < 1e-5 && (e[2] - 0.1).abs() < 1e-6
        );
    }

    #[test]
    fn the_farthest_quad_comes_first_and_ties_keep_their_order() {
        let centres = [
            [0.5, 0.5, 0.5],
            [0.5, 0.5, 3.5],
            [0.5, 0.5, 1.5],
            // The same place as the second: after it.
            [0.5, 0.5, 3.5],
            [0.5, 0.5, 2.5],
        ];
        let mut s = Sorter::default();
        assert_eq!(s.order(&centres, [0.5, 0.5, -4.0]), [1, 3, 4, 2, 0]);
        assert_eq!(s.order(&centres, [0.5, 0.5, 20.0]), [0, 2, 4, 1, 3]);
        // From between them: by distance, not by side.
        assert_eq!(s.order(&centres, [0.5, 0.5, 2.4]), [0, 1, 3, 2, 4]);
        assert_eq!(s.order(&[], [0.0; 3]), [0u32; 0]);
    }

    #[test]
    fn small_offsets_on_two_axes_break_a_distant_tie() {
        // At 4096² the step between floats is two. Adding each one's square to
        // the large square separately would round both offsets away.
        let centres = [[0.0; 3], [0.0, 1.0, 1.0]];
        assert_eq!(
            Sorter::default().order(&centres, [4096.0, 0.0, 0.0]),
            [1, 0]
        );
    }

    #[test]
    fn indices_draw_each_quad_as_two_triangles_in_the_sorted_order() {
        let centres = [[0.0, 0.0, 1.0], [0.0, 0.0, 5.0]];
        let mut s = Sorter::default();
        let mut bytes = Vec::new();
        s.indices(&centres, [0.0; 3], &mut bytes);
        let words: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect();
        assert_eq!(words, [4, 5, 6, 6, 7, 4, 0, 1, 2, 2, 3, 0]);
        assert_eq!(bytes.len(), index_bytes(2));
        // More quads than 16 bits can name the corners of: 32-bit indices.
        let many = vec![[0.0_f32; 3]; NARROW_QUADS + 1];
        s.indices(&many[..NARROW_QUADS], [0.0; 3], &mut bytes);
        assert_eq!(bytes.len(), NARROW_QUADS * 12);
        assert_eq!(
            &bytes[bytes.len() - 4..bytes.len() - 2],
            &u16::MAX.to_le_bytes()
        );
        s.indices(&many, [0.0; 3], &mut bytes);
        assert_eq!(bytes.len(), (NARROW_QUADS + 1) * 24);
        assert_eq!(bytes.len(), index_bytes(NARROW_QUADS + 1));
        let last = u32::from_le_bytes(bytes[bytes.len() - 8..bytes.len() - 4].try_into().unwrap());
        assert_eq!(last, NARROW_QUADS as u32 * 4 + 3);
        assert_eq!(index_bytes(NARROW_QUADS), NARROW_QUADS * 12);
    }
}
