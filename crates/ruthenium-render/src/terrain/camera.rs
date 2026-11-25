//! Where the world is seen from: the matrix that takes blocks to the screen, and which
//! sections it can see.
//!
//! Positions reach the GPU relative to the camera's block, as whole blocks, with the
//! camera's place inside its block taken off afterwards (vanilla's terrain shader does
//! the same): far from the world's origin nothing is lost to the size of the numbers.
//!
//! Depth is reversed and the far plane is at infinity: the near plane is depth 1 and
//! depth falls towards 0 with distance, which with a float depth image spreads
//! precision evenly. A target's y points down, so the projection turns y over.

/// Blocks from the eye to the near plane, as vanilla's.
pub const NEAR_PLANE: f32 = 0.05;

/// Where the camera is and where it looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// The eye, in blocks.
    pub position: [f64; 3],
    /// Degrees: 0 looks south (+z), 90 west (-x), as an entity's yaw.
    pub y_rot: f32,
    /// Degrees: 90 looks straight down.
    pub x_rot: f32,
    /// The field of view from top to bottom, in degrees.
    pub fov: f32,
}

impl CameraPose {
    /// The directions the camera looks along, has to its right and has above it.
    pub fn axes(&self) -> [[f32; 3]; 3] {
        let (yaw, pitch) = (self.y_rot.to_radians(), self.x_rot.to_radians());
        let forward = [
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let right = [-yaw.cos(), 0.0, -yaw.sin()];
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        [forward, right, up]
    }
}

/// A camera for one frame and one target shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// The block the eye is in.
    pub block: [i32; 3],
    /// Minus the eye's place inside that block: what the shader adds to positions
    /// given relative to `block`.
    pub offset: [f32; 3],
    /// From blocks relative to the eye to clip space, columns first.
    pub view_projection: [f32; 16],
    /// The frustum's left, right, bottom, top and near planes: a point `p` (relative to
    /// the eye) is inside when `n . p + d >= 0` for each `[n, d]`.
    planes: [[f32; 4]; 5],
}

/// A view that is not moved ([`View::moved`]).
const UNMOVED: [[f32; 4]; 3] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl View {
    /// `aspect` is the target's width over its height.
    pub fn new(pose: &CameraPose, aspect: f32) -> View {
        View::moved(pose, aspect, &UNMOVED)
    }

    /// The same, with what is seen moved by `bob` first, in the view's own space (x to the
    /// right, y up, ahead at negative z; three rows of four): how the view sways with
    /// the player's steps and tilts when it is hurt. What the view sees ([`Self::sees`])
    /// is judged without it, as vanilla's own frustum is.
    pub fn moved(pose: &CameraPose, aspect: f32, bob: &[[f32; 4]; 3]) -> View {
        let block = pose.position.map(|c| c.floor() as i32);
        let offset = std::array::from_fn(|i| -(pose.position[i] - f64::from(block[i])) as f32);
        let [forward, right, up] = pose.axes();
        let g = 1.0 / (pose.fov.to_radians() / 2.0).tan();
        let gx = g / aspect;
        // From blocks around the eye to the view's space, then moved.
        let to_view = [right, up, forward.map(|c| -c)];
        let moved: [[f32; 4]; 3] = std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                if c == 3 {
                    bob[r][3]
                } else {
                    (0..3).map(|k| bob[r][k] * to_view[k][c]).sum()
                }
            })
        });
        // Rows: x to the right, y down the target, the near plane's depth, and the
        // distance ahead to divide by.
        let rows = [
            moved[0].map(|c| gx * c),
            moved[1].map(|c| -g * c),
            [0.0, 0.0, 0.0, NEAR_PLANE],
            moved[2].map(|c| -c),
        ];
        let view_projection = std::array::from_fn(|i| rows[i % 4][i / 4]);
        let side = |sign: f32, axis: [f32; 3], scale: f32| {
            [
                forward[0] + sign * scale * axis[0],
                forward[1] + sign * scale * axis[1],
                forward[2] + sign * scale * axis[2],
                0.0,
            ]
        };
        View {
            block,
            offset,
            view_projection,
            planes: [
                side(1.0, right, gx),
                side(-1.0, right, gx),
                side(1.0, up, g),
                side(-1.0, up, g),
                [forward[0], forward[1], forward[2], -NEAR_PLANE],
            ],
        }
    }

    /// The view of a GUI's models on a target of `size` pixels: positions are pixels from
    /// the target's top left corner, and depths from 1000 behind the screen to 1000 before
    /// it (`GuiItemAtlas`'s projection). Everything is seen.
    pub fn gui(size: [f32; 2]) -> View {
        // Rows: x and y to clip space, the depth (nearer is greater), and no division.
        let rows = [
            [2.0 / size[0], 0.0, 0.0, -1.0],
            [0.0, 2.0 / size[1], 0.0, -1.0],
            [0.0, 0.0, 1.0 / 2000.0, 0.5],
            [0.0, 0.0, 0.0, 1.0],
        ];
        View {
            block: [0; 3],
            offset: [0.0; 3],
            view_projection: std::array::from_fn(|i| rows[i % 4][i / 4]),
            planes: [[0.0, 0.0, 0.0, 1.0]; 5],
        }
    }

    /// Whether any of a box can be on screen. Its corners are in blocks from the
    /// camera's block. A box partly outside counts as seen.
    pub fn sees(&self, min: [f32; 3], max: [f32; 3]) -> bool {
        let (min, max): ([f32; 3], [f32; 3]) = (
            std::array::from_fn(|i| min[i] + self.offset[i]),
            std::array::from_fn(|i| max[i] + self.offset[i]),
        );
        self.planes.iter().all(|p| {
            // The corner furthest along the plane's normal.
            let far = [
                if p[0] >= 0.0 { max[0] } else { min[0] },
                if p[1] >= 0.0 { max[1] } else { min[1] },
                if p[2] >= 0.0 { max[2] } else { min[2] },
            ];
            dot([p[0], p[1], p[2]], far) + p[3] >= 0.0
        })
    }

    /// Where a point (in blocks from the eye) lands: x and y from -1 to 1 across the
    /// target (y down), and its depth; `None` behind the eye.
    pub fn project(&self, p: [f32; 3]) -> Option<[f32; 3]> {
        let m = &self.view_projection;
        let row = |r: usize| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r];
        let w = row(3);
        (w > 0.0).then(|| [row(0) / w, row(1) / w, row(2) / w])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(y_rot: f32, x_rot: f32) -> CameraPose {
        CameraPose {
            position: [0.0; 3],
            y_rot,
            x_rot,
            fov: 70.0,
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn a_guis_view_is_in_pixels_and_sees_everything() {
        let v = View::gui([1920.0, 1080.0]);
        let clip = |p: [f32; 3]| {
            let m = v.view_projection;
            let row = |r: usize| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r];
            [row(0), row(1), row(2), row(3)]
        };
        // The corners of the target, and its middle at the screen's own depth.
        assert_eq!(clip([0.0, 0.0, 0.0]), [-1.0, -1.0, 0.5, 1.0]);
        assert_eq!(clip([1920.0, 1080.0, 0.0]), [1.0, 1.0, 0.5, 1.0]);
        // Nearer is greater, from 1000 behind the screen to 1000 before it.
        assert_eq!(clip([960.0, 540.0, 1000.0])[2], 1.0);
        assert_eq!(clip([960.0, 540.0, -1000.0])[2], 0.0);
        assert!(v.sees([-5000.0; 3], [-4000.0; 3]));
    }

    #[test]
    fn yaw_and_pitch_are_an_entitys() {
        // Yaw 0 looks south: a point to the south is in the middle of the target, one
        // to the west of it (-x... no: to its right, which is west) on the right half.
        let v = View::new(&pose(0.0, 0.0), 16.0 / 9.0);
        let centre = v.project([0.0, 0.0, 10.0]).unwrap();
        assert!(close(centre[0], 0.0) && close(centre[1], 0.0));
        assert!(v.project([0.0, 0.0, -10.0]).is_none());
        assert!(v.project([-1.0, 0.0, 10.0]).unwrap()[0] > 0.0);
        // Up in the world is up on the target, whose y points down.
        assert!(v.project([0.0, 1.0, 10.0]).unwrap()[1] < 0.0);
        // Yaw 90 looks west, yaw -90 east, yaw 180 north.
        for (yaw, ahead) in [
            (90.0, [-10.0, 0.0, 0.0]),
            (-90.0, [10.0, 0.0, 0.0]),
            (180.0, [0.0, 0.0, -10.0]),
        ] {
            let p = View::new(&pose(yaw, 0.0), 1.0).project(ahead).unwrap();
            assert!(close(p[0], 0.0) && close(p[1], 0.0), "yaw {yaw}: {p:?}");
        }
        // Pitch 90 looks down, -90 up.
        let down = View::new(&pose(0.0, 90.0), 1.0);
        assert!(close(down.project([0.0, -5.0, 0.0]).unwrap()[0], 0.0));
        assert!(down.project([0.0, 5.0, 0.0]).is_none());
        // Looking down while facing south, south is at the top of the target.
        assert!(down.project([0.0, -5.0, 1.0]).unwrap()[1] < 0.0);
    }

    #[test]
    fn a_moved_view_sees_everything_moved() {
        let p = pose(30.0, 10.0);
        // Unmoved, it is the view itself, to the bit where nothing is added.
        let plain = View::new(&p, 1.5);
        assert_eq!(View::moved(&p, 1.5, &UNMOVED), plain);
        // Sunk by a tenth of a block: what was at the middle is a tenth lower in the
        // view's space, which is up on... down the target.
        let sunk = View::moved(
            &p,
            1.5,
            &[
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, -0.1],
                [0.0, 0.0, 1.0, 0.0],
            ],
        );
        let [forward, _, up] = p.axes();
        let ahead = forward.map(|c| c * 4.0);
        let middle = sunk.project(ahead).unwrap();
        assert!(close(middle[0], 0.0) && middle[1] > 0.0);
        let raised: [f32; 3] = std::array::from_fn(|i| ahead[i] + up[i] * 0.1);
        let back = sunk.project(raised).unwrap();
        assert!(close(back[0], 0.0) && close(back[1], 0.0));
        // Rolled a quarter turn about the way ahead: what was to the right is below.
        let rolled = View::moved(
            &pose(0.0, 0.0),
            1.0,
            &[
                [0.0, -1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        );
        let right = rolled.project([-1.0, 0.0, 5.0]).unwrap();
        assert!(close(right[0], 0.0) && right[1] < 0.0);
        // The frustum stays where it was.
        assert_eq!(sunk.planes, plain.planes);
    }

    #[test]
    fn the_field_of_view_spans_the_targets_height() {
        let v = View::new(&pose(0.0, 0.0), 2.0);
        let t = 35.0_f32.to_radians().tan();
        let top = v.project([0.0, 10.0 * t, 10.0]).unwrap();
        assert!(close(top[1], -1.0));
        // Twice as wide as high: twice as far to the side reaches the edge.
        let side = v.project([-20.0 * t, 0.0, 10.0]).unwrap();
        assert!(close(side[0], 1.0));
    }

    #[test]
    fn depth_is_one_at_the_near_plane_and_falls_towards_zero() {
        let v = View::new(&pose(0.0, 0.0), 1.0);
        let depth = |z: f32| v.project([0.0, 0.0, z]).unwrap()[2];
        assert!(close(depth(NEAR_PLANE), 1.0));
        assert!(close(depth(1.0), 0.05));
        assert!(depth(10.0) > depth(100.0) && depth(100.0) > depth(10_000.0));
        assert!(depth(1.0e6) > 0.0);
        // Depth depends on the distance ahead only, not on where on the target.
        assert!(close(v.project([3.0, -2.0, 10.0]).unwrap()[2], depth(10.0)));
    }

    #[test]
    fn a_face_seen_from_its_front_is_counter_clockwise_on_the_target() {
        // A block's north face (it looks towards -z), in the bakery's vertex order,
        // seen from the north by a camera looking south.
        let v = View::new(&pose(0.0, 0.0), 1.0);
        let quad = [
            [1.0, 1.0, 5.0],
            [1.0, 0.0, 5.0],
            [0.0, 0.0, 5.0],
            [0.0, 1.0, 5.0],
        ];
        let p: Vec<[f32; 3]> = quad.iter().map(|c| v.project(*c).unwrap()).collect();
        // The first corner is at the top left (east is to the left of a camera looking
        // south), the second below it, the third to the right of that: round to the
        // left. In the target's coordinates (x right, y down) such a turn has a negative
        // cross product, and it is what Vulkan calls counter-clockwise.
        assert!(p[0][0] < 0.0 && p[0][1] < 0.0 && p[1][1] > p[0][1] && p[2][0] > p[1][0]);
        let cross = |a: usize, b: usize, c: usize| {
            (p[b][0] - p[a][0]) * (p[c][1] - p[a][1]) - (p[c][0] - p[a][0]) * (p[b][1] - p[a][1])
        };
        assert!(cross(0, 1, 2) < 0.0 && cross(0, 2, 3) < 0.0);
        // Seen from behind (a camera to the south of it looking north), it turns the
        // other way and is culled.
        let behind = View::new(
            &CameraPose {
                position: [0.0, 0.0, 10.0],
                y_rot: 180.0,
                x_rot: 0.0,
                fov: 70.0,
            },
            1.0,
        );
        let p: Vec<[f32; 3]> = quad
            .iter()
            .map(|c| behind.project([c[0], c[1], c[2] - 10.0]).unwrap())
            .collect();
        let cross =
            (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        assert!(cross > 0.0);
    }

    #[test]
    fn the_camera_block_and_the_offset_make_the_eye() {
        let v = View::new(
            &CameraPose {
                position: [-0.25, 70.62, 1_000_000.5],
                y_rot: 0.0,
                x_rot: 0.0,
                fov: 70.0,
            },
            1.0,
        );
        assert_eq!(v.block, [-1, 70, 1_000_000]);
        assert!(close(v.offset[0], -0.75) && close(v.offset[1], -0.62));
        assert_eq!(v.offset[2], -0.5);
    }

    #[test]
    fn only_boxes_in_the_frustum_are_seen() {
        let v = View::new(
            &CameraPose {
                position: [0.5, 0.5, 0.5],
                y_rot: 0.0,
                x_rot: 0.0,
                fov: 70.0,
            },
            16.0 / 9.0,
        );
        let section = |x: f32, y: f32, z: f32| v.sees([x, y, z], [x + 16.0, y + 16.0, z + 16.0]);
        assert!(section(-8.0, -8.0, 16.0), "ahead");
        assert!(section(-8.0, -8.0, -8.0), "around the eye");
        assert!(!section(-8.0, -8.0, -48.0), "behind");
        assert!(!section(200.0, -8.0, 32.0), "far to the left");
        assert!(!section(-216.0, -8.0, 32.0), "far to the right");
        assert!(!section(-8.0, 200.0, 32.0), "far above");
        assert!(
            section(-8.0, -8.0, 4000.0),
            "far ahead: there is no far plane"
        );
        // A box that only pokes into the view counts.
        assert!(section(40.0, -8.0, 32.0));
        // Turned round, what was behind is ahead.
        let back = View::new(
            &CameraPose {
                position: [0.5, 0.5, 0.5],
                y_rot: 180.0,
                x_rot: 0.0,
                fov: 70.0,
            },
            16.0 / 9.0,
        );
        assert!(back.sees([-8.0, -8.0, -48.0], [8.0, 8.0, -32.0]));
        assert!(!back.sees([-8.0, -8.0, 16.0], [8.0, 8.0, 32.0]));
    }
}
