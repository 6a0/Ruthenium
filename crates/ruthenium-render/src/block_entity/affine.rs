//! Affine transforms of models, as three rows of four.

/// Maps a point `p` to `rows * (p, 1)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine(pub [[f32; 4]; 3]);

impl Affine {
    pub const IDENTITY: Affine = Affine([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ]);

    /// First `other`, then `self`.
    pub fn then_inner(self, other: Affine) -> Affine {
        let (a, b) = (self.0, other.0);
        Affine(std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                let linear = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
                if c == 3 { linear + a[r][3] } else { linear }
            })
        }))
    }

    pub fn translate(self, [x, y, z]: [f32; 3]) -> Affine {
        self.then_inner(Affine([
            [1.0, 0.0, 0.0, x],
            [0.0, 1.0, 0.0, y],
            [0.0, 0.0, 1.0, z],
        ]))
    }

    pub fn scale(self, [x, y, z]: [f32; 3]) -> Affine {
        self.then_inner(Affine([
            [x, 0.0, 0.0, 0.0],
            [0.0, y, 0.0, 0.0],
            [0.0, 0.0, z, 0.0],
        ]))
    }

    pub fn rotate_x(self, radians: f32) -> Affine {
        let (s, c) = radians.sin_cos();
        self.then_inner(Affine([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, c, -s, 0.0],
            [0.0, s, c, 0.0],
        ]))
    }

    pub fn rotate_y(self, radians: f32) -> Affine {
        let (s, c) = radians.sin_cos();
        self.then_inner(Affine([
            [c, 0.0, s, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [-s, 0.0, c, 0.0],
        ]))
    }

    pub fn rotate_z(self, radians: f32) -> Affine {
        let (s, c) = radians.sin_cos();
        self.then_inner(Affine([
            [c, -s, 0.0, 0.0],
            [s, c, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ]))
    }

    pub fn point(&self, p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|r| {
            let row = self.0[r];
            row[0] * p[0] + row[1] * p[1] + row[2] * p[2] + row[3]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-6)
    }

    #[test]
    fn what_is_said_last_is_done_first() {
        let t = Affine::IDENTITY
            .translate([1.0, 0.0, 0.0])
            .scale([2.0, 2.0, 2.0]);
        assert_eq!(t.point([1.0, 1.0, 0.0]), [3.0, 2.0, 0.0]);
    }

    #[test]
    fn turns_are_right_handed() {
        let i = Affine::IDENTITY;
        assert!(close(
            i.rotate_x(FRAC_PI_2).point([0.0, 1.0, 0.0]),
            [0.0, 0.0, 1.0]
        ));
        assert!(close(
            i.rotate_y(FRAC_PI_2).point([0.0, 0.0, 1.0]),
            [1.0, 0.0, 0.0]
        ));
        assert!(close(
            i.rotate_z(FRAC_PI_2).point([1.0, 0.0, 0.0]),
            [0.0, 1.0, 0.0]
        ));
    }
}
