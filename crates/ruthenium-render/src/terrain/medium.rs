//! What the camera is in: air, water, lava or powder snow.
//!
//! As 26.3's `Camera.getFluidInCamera` finds it. Water counts by the eye alone; lava and
//! powder snow count as soon as a corner of the near plane is in them, so that their fog
//! is there before the plane cuts into their faces.

use ruthenium_sim::fluid::FluidState;
use ruthenium_sim::scene::Terrain;

use super::camera::{CameraPose, NEAR_PLANE};
use super::table::BlockTable;
use crate::atmosphere::Medium;

/// `Direction.DOWN`'s bit among the sturdy faces.
const STURDY_DOWN: u8 = 1;

fn state(terrain: &Terrain, pos: [i32; 3]) -> u32 {
    terrain.probe(pos).map_or(0, |p| u32::from(p.block))
}

/// Where a fluid's surface is for a camera (`FluidState.getHeightForCamera`): the top of
/// its block under more of it, and for a source also under a face that closes the block.
fn surface(terrain: &Terrain, table: &BlockTable, pos: [i32; 3], fluid: FluidState) -> f32 {
    let above = table.state(state(terrain, [pos[0], pos[1] + 1, pos[2]]));
    let closed = fluid.is_source() && above.sturdy & STURDY_DOWN != 0;
    let height = if closed
        || fluid
            .fluid
            .is_same(FluidState::from_bits(above.fluid).fluid)
    {
        1.0
    } else {
        fluid.own_height()
    };
    // Single precision, as the game adds them.
    pos[1] as f32 + height
}

/// What a camera at `camera` is in, whose field of view is the option's. `aspect` is the
/// target's width over its height.
pub(crate) fn medium_at(
    terrain: &Terrain,
    table: &BlockTable,
    camera: &CameraPose,
    aspect: f32,
) -> Medium {
    let block_of = |p: [f64; 3]| p.map(|c| c.floor() as i32);
    let fluid_of = |pos| FluidState::from_bits(table.state(state(terrain, pos)).fluid);
    let eye = block_of(camera.position);
    let fluid = fluid_of(eye);
    if fluid.fluid.is_water() && camera.position[1] < f64::from(surface(terrain, table, eye, fluid))
    {
        return Medium::Water;
    }
    let [forward, right, up] = camera.axes().map(|axis| axis.map(f64::from));
    let near = f64::from(NEAR_PLANE);
    let height = f64::from(camera.fov.to_radians() / 2.0).tan() * near;
    let width = height * f64::from(aspect);
    // The plane's middle, then its corners: top left and right, bottom left and right.
    for (x, y) in [
        (0.0, 0.0),
        (-1.0, 1.0),
        (1.0, 1.0),
        (-1.0, -1.0),
        (1.0, -1.0),
    ] {
        let point: [f64; 3] = std::array::from_fn(|i| {
            camera.position[i] + forward[i] * near + up[i] * height * y + right[i] * width * x
        });
        let pos = block_of(point);
        let fluid = fluid_of(pos);
        if fluid.fluid.is_lava() {
            if point[1] <= f64::from(surface(terrain, table, pos, fluid)) {
                return Medium::Lava;
            }
        } else if table.state(state(terrain, pos)).block == table.powder_snow {
            return Medium::PowderSnow;
        }
    }
    Medium::Air
}

#[cfg(test)]
mod tests {
    use super::super::testing::{self, World, lava, stone, water};
    use super::*;

    fn camera(position: [f64; 3], y_rot: f32, x_rot: f32) -> CameraPose {
        CameraPose {
            position,
            y_rot,
            x_rot,
            fov: 70.0,
        }
    }

    fn at(world: &World, camera: &CameraPose) -> Medium {
        let table = testing::table();
        world
            .scene
            .with_terrain(|t| medium_at(t, &table, camera, 16.0 / 9.0))
    }

    /// Looking along the horizon from a height in the block at 8, 8.
    fn level(world: &World, y: f64) -> Medium {
        at(world, &camera([8.5, y, 8.5], 0.0, 0.0))
    }

    #[test]
    fn water_counts_up_to_its_surface() {
        let mut w = World::new(&[(0, 0)]);
        w.set(8, 4, 8, water(0));
        w.set(8, 5, 8, water(0));
        w.set(8, 10, 8, water(3));
        w.publish();
        assert_eq!(level(&w, 4.99), Medium::Water);
        // A source with air above it stands eight ninths high.
        assert_eq!(level(&w, 5.88), Medium::Water);
        assert_eq!(level(&w, 5.8889), Medium::Air);
        assert_eq!(level(&w, 3.99), Medium::Air);
        // Flowing water five ninths.
        assert_eq!(level(&w, 10.55), Medium::Water);
        assert_eq!(level(&w, 10.56), Medium::Air);
        // Beside it, or where no chunk is, there is none.
        assert_eq!(at(&w, &camera([9.5, 4.5, 8.5], 0.0, 0.0)), Medium::Air);
        assert_eq!(at(&w, &camera([900.0, 4.5, 8.5], 0.0, 0.0)), Medium::Air);
        // Only the eye counts: one just over the surface looks into water below it.
        assert_eq!(at(&w, &camera([8.5, 5.9, 8.5], 0.0, 90.0)), Medium::Air);
    }

    #[test]
    fn a_source_under_a_closed_face_fills_its_block() {
        let mut w = World::new(&[(0, 0)]);
        w.set(8, 4, 8, water(0));
        w.set(8, 5, 8, stone());
        w.set(8, 10, 8, water(1));
        w.set(8, 11, 8, stone());
        // A lower slab with water in it is water too, and its lower face is not sturdy
        // in the made-up game.
        w.set(8, 20, 8, testing::wet_slab());
        w.publish();
        assert_eq!(level(&w, 4.99), Medium::Water);
        // Flowing water does not rise to it.
        assert_eq!(level(&w, 10.99), Medium::Air);
        assert_eq!(level(&w, 20.7), Medium::Water);
    }

    #[test]
    fn lava_counts_from_the_near_planes_corners() {
        let mut w = World::new(&[(0, 0)]);
        w.set(8, 4, 8, lava(0));
        w.publish();
        let surface = 4.0 + 8.0 / 9.0;
        assert_eq!(level(&w, 4.5), Medium::Lava);
        // The plane's lower corners are 0.035 below the eye: they dip in first.
        assert_eq!(level(&w, surface + 0.03), Medium::Lava);
        assert_eq!(level(&w, surface + 0.04), Medium::Air);
        // Looking straight down the plane is 0.05 ahead, and up it is out of it.
        let over = [8.5, surface + 0.04, 8.5];
        assert_eq!(at(&w, &camera(over, 0.0, 90.0)), Medium::Lava);
        let under = [8.5, surface - 0.04, 8.5];
        assert_eq!(at(&w, &camera(under, 0.0, -90.0)), Medium::Air);
        // Beside it: the plane's side reaches 0.062 sideways and 0.05 ahead.
        let beside = |x: f64| at(&w, &camera([x, 4.5, 8.5], 0.0, 0.0));
        assert_eq!(beside(9.05), Medium::Lava);
        assert_eq!(beside(9.07), Medium::Air);
        assert_eq!(beside(7.95), Medium::Lava);
        assert_eq!(beside(7.93), Medium::Air);
        let ahead = |z: f64| at(&w, &camera([8.5, 4.5, z], 0.0, 0.0));
        assert_eq!(ahead(7.96), Medium::Lava);
        assert_eq!(ahead(7.94), Medium::Air);
    }

    #[test]
    fn powder_snow_counts_as_a_whole_block() {
        let mut w = World::new(&[(0, 0)]);
        w.set(8, 4, 8, testing::state("minecraft:powder_snow", &[]));
        w.publish();
        assert_eq!(level(&w, 4.99), Medium::PowderSnow);
        assert_eq!(level(&w, 5.03), Medium::PowderSnow);
        assert_eq!(level(&w, 5.04), Medium::Air);
        // Stone is nothing to be in.
        w.set(8, 4, 8, stone());
        w.publish();
        assert_eq!(level(&w, 4.5), Medium::Air);
    }
}
