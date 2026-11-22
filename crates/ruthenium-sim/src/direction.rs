//! `Direction`: the six faces of a block, and the turns and orderings placing
//! blocks reads from the player's view.

use ruthenium_javamath::mth;

use crate::shape::Axis;

/// `Direction`, in its ordinal order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

/// Degrees to radians as the game multiplies floats (`0.017453292F`).
const DEG_TO_RAD: f32 = 0.017_453_292;

impl Direction {
    /// `Direction.VALUES`.
    pub const ALL: [Direction; 6] = [
        Direction::Down,
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ];

    /// The horizontal directions in their 2D data order (south, west, north, east).
    pub const BY_2D: [Direction; 4] = [
        Direction::South,
        Direction::West,
        Direction::North,
        Direction::East,
    ];

    /// `Direction.Plane.HORIZONTAL`'s order: north, east, south, west.
    pub const HORIZONTAL: [Direction; 4] = [
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
    ];

    /// The 3D data value, which is also the ordinal and the network id.
    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn from_id(id: u8) -> Option<Direction> {
        Direction::ALL.get(usize::from(id)).copied()
    }

    /// The ordinal, for indexing per-face tables.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The unit step towards the direction.
    pub fn normal(self) -> [i32; 3] {
        match self {
            Direction::Down => [0, -1, 0],
            Direction::Up => [0, 1, 0],
            Direction::North => [0, 0, -1],
            Direction::South => [0, 0, 1],
            Direction::West => [-1, 0, 0],
            Direction::East => [1, 0, 0],
        }
    }

    /// The block position one step this way from `pos` (`BlockPos.relative`).
    pub fn step(self, pos: [i32; 3]) -> [i32; 3] {
        let n = self.normal();
        [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
    }

    pub fn opposite(self) -> Direction {
        match self {
            Direction::Down => Direction::Up,
            Direction::Up => Direction::Down,
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::West => Direction::East,
            Direction::East => Direction::West,
        }
    }

    pub fn axis(self) -> Axis {
        match self {
            Direction::Down | Direction::Up => Axis::Y,
            Direction::North | Direction::South => Axis::Z,
            Direction::West | Direction::East => Axis::X,
        }
    }

    pub fn is_horizontal(self) -> bool {
        self.axis() != Axis::Y
    }

    /// Whether the direction points along its axis's positive side (up, south, east).
    pub fn is_positive(self) -> bool {
        matches!(self, Direction::Up | Direction::South | Direction::East)
    }

    /// `fromAxisAndDirection`.
    pub fn from_axis(axis: Axis, positive: bool) -> Direction {
        match (axis, positive) {
            (Axis::X, true) => Direction::East,
            (Axis::X, false) => Direction::West,
            (Axis::Y, true) => Direction::Up,
            (Axis::Y, false) => Direction::Down,
            (Axis::Z, true) => Direction::South,
            (Axis::Z, false) => Direction::North,
        }
    }

    /// The name block-state properties use (`north`).
    pub fn name(self) -> &'static str {
        match self {
            Direction::Down => "down",
            Direction::Up => "up",
            Direction::North => "north",
            Direction::South => "south",
            Direction::West => "west",
            Direction::East => "east",
        }
    }

    pub fn from_name(name: &str) -> Option<Direction> {
        Direction::ALL.into_iter().find(|d| d.name() == name)
    }

    /// The 2D data value (south 0, west 1, north 2, east 3; -1 for up and down).
    pub fn data_2d(self) -> i32 {
        match self {
            Direction::South => 0,
            Direction::West => 1,
            Direction::North => 2,
            Direction::East => 3,
            Direction::Down | Direction::Up => -1,
        }
    }

    /// `from2DDataValue`: the index taken modulo 4, its sign dropped.
    pub fn from_2d(data: i32) -> Direction {
        Direction::BY_2D[(data % 4).unsigned_abs() as usize]
    }

    /// `fromYRot`: the horizontal direction a yaw faces.
    pub fn from_y_rot(y_rot: f64) -> Direction {
        Direction::from_2d(mth::floor(y_rot / 90.0 + 0.5) & 3)
    }

    /// `toYRot`: the yaw a horizontal direction faces (up and down give 270).
    pub fn to_y_rot(self) -> f32 {
        ((self.data_2d() & 3) * 90) as f32
    }

    /// `getClockWise` seen from above. Panics for up and down, as the game throws.
    pub fn clockwise(self) -> Direction {
        match self {
            Direction::North => Direction::East,
            Direction::East => Direction::South,
            Direction::South => Direction::West,
            Direction::West => Direction::North,
            _ => panic!("no clockwise turn of {self:?} around y"),
        }
    }

    /// `getCounterClockWise` seen from above. Panics for up and down.
    pub fn counter_clockwise(self) -> Direction {
        match self {
            Direction::North => Direction::West,
            Direction::West => Direction::South,
            Direction::South => Direction::East,
            Direction::East => Direction::North,
            _ => panic!("no counter-clockwise turn of {self:?} around y"),
        }
    }

    /// `getClockWise(axis)`: a quarter turn around an axis (directions along it stay).
    pub fn clockwise_around(self, axis: Axis) -> Direction {
        use Direction::*;
        match (axis, self) {
            (Axis::Y, Up | Down) | (Axis::X, West | East) | (Axis::Z, North | South) => self,
            (Axis::Y, _) => self.clockwise(),
            (Axis::X, Down) => South,
            (Axis::X, Up) => North,
            (Axis::X, North) => Down,
            (Axis::X, _) => Up,
            (Axis::Z, Down) => West,
            (Axis::Z, Up) => East,
            (Axis::Z, West) => Up,
            (Axis::Z, _) => Down,
        }
    }

    /// `getCounterClockWise(axis)`.
    pub fn counter_clockwise_around(self, axis: Axis) -> Direction {
        // Three clockwise quarter turns.
        self.clockwise_around(axis)
            .clockwise_around(axis)
            .clockwise_around(axis)
    }

    /// `getApproximateNearest`: the direction the vector points most along, in
    /// floats; north when it points nowhere.
    pub fn approximate_nearest(v: [f64; 3]) -> Direction {
        let (x, y, z) = (v[0] as f32, v[1] as f32, v[2] as f32);
        let mut result = Direction::North;
        // `1.0E-45F`, the smallest float above zero.
        let mut highest = f32::from_bits(1);
        for d in Direction::ALL {
            let n = d.normal();
            let dot = x * n[0] as f32 + y * n[1] as f32 + z * n[2] as f32;
            if dot > highest {
                highest = dot;
                result = d;
            }
        }
        result
    }

    /// `isFacingAngle`: whether a yaw looks somewhat this way (horizontal parts
    /// only).
    pub fn is_facing_angle(self, y_rot: f32) -> bool {
        let radians = y_rot * DEG_TO_RAD;
        let dx = -mth::sin(f64::from(radians));
        let dz = mth::cos(f64::from(radians));
        let n = self.normal();
        n[0] as f32 * dx + n[2] as f32 * dz > 0.0
    }

    /// `orderedByNearest`: all six directions from the one the view points most
    /// along to its opposite. The first three are the view's sign on each axis, by how
    /// much of the view lies along it (floats, the game's sine table); the last three
    /// are their opposites in reverse.
    pub fn ordered_by_nearest(y_rot: f32, x_rot: f32) -> [Direction; 6] {
        let pitch = x_rot * DEG_TO_RAD;
        let yaw = -y_rot * DEG_TO_RAD;
        let (ps, pc) = (mth::sin(f64::from(pitch)), mth::cos(f64::from(pitch)));
        let (ys, yc) = (mth::sin(f64::from(yaw)), mth::cos(f64::from(yaw)));
        let (east, up, south) = (ys > 0.0, ps < 0.0, yc > 0.0);
        let x_yaw = if east { ys } else { -ys };
        let y_mag = if up { -ps } else { ps };
        let z_yaw = if south { yc } else { -yc };
        let x_mag = x_yaw * pc;
        let z_mag = z_yaw * pc;
        let ax = if east {
            Direction::East
        } else {
            Direction::West
        };
        let ay = if up { Direction::Up } else { Direction::Down };
        let az = if south {
            Direction::South
        } else {
            Direction::North
        };
        let (a, b, c) = if x_yaw > z_yaw {
            if y_mag > x_mag {
                (ay, ax, az)
            } else if z_mag > y_mag {
                (ax, az, ay)
            } else {
                (ax, ay, az)
            }
        } else if y_mag > z_mag {
            (ay, az, ax)
        } else if x_mag > y_mag {
            (az, ax, ay)
        } else {
            (az, ay, ax)
        };
        [a, b, c, c.opposite(), b.opposite(), a.opposite()]
    }

    /// `getFacingAxis`: which way along an axis the view points (`x_rot` below zero
    /// is up).
    pub fn facing_axis(y_rot: f32, x_rot: f32, axis: Axis) -> Direction {
        match axis {
            Axis::X if Direction::East.is_facing_angle(y_rot) => Direction::East,
            Axis::X => Direction::West,
            Axis::Y if x_rot < 0.0 => Direction::Up,
            Axis::Y => Direction::Down,
            Axis::Z if Direction::South.is_facing_angle(y_rot) => Direction::South,
            Axis::Z => Direction::North,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_and_yaws() {
        assert_eq!(Direction::from_y_rot(0.0), Direction::South);
        assert_eq!(Direction::from_y_rot(-90.0), Direction::East);
        assert_eq!(Direction::from_y_rot(135.0), Direction::North);
        assert_eq!(Direction::from_y_rot(-44.9), Direction::South);
        assert_eq!(Direction::from_y_rot(-45.0), Direction::South);
        assert_eq!(Direction::from_y_rot(-45.1), Direction::East);
        assert_eq!(Direction::North.clockwise(), Direction::East);
        assert_eq!(Direction::East.counter_clockwise(), Direction::North);
        for d in Direction::ALL {
            for a in Axis::ALL {
                assert_eq!(d.clockwise_around(a).counter_clockwise_around(a), d);
            }
            assert_eq!(Direction::from_name(d.name()), Some(d));
        }
        assert_eq!(Direction::Up.clockwise_around(Axis::X), Direction::North);
        assert_eq!(Direction::Down.clockwise_around(Axis::Z), Direction::West);
        assert_eq!(Direction::from_2d(-3), Direction::East);
        assert_eq!(Direction::West.to_y_rot(), 90.0);
    }

    #[test]
    fn nearest_directions_follow_the_view() {
        // Looking south and a little down.
        let d = Direction::ordered_by_nearest(10.0, 30.0);
        assert_eq!(d[0], Direction::South);
        assert_eq!(d[5], Direction::North);
        // Looking almost straight down while facing west.
        let d = Direction::ordered_by_nearest(90.0, 80.0);
        assert_eq!(
            &d[..3],
            &[Direction::Down, Direction::West, Direction::North]
        );
        assert_eq!(Direction::facing_axis(90.0, 80.0, Axis::X), Direction::West);
        assert_eq!(Direction::facing_axis(90.0, -1.0, Axis::Y), Direction::Up);
    }
}
