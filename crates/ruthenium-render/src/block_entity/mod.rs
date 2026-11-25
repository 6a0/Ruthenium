//! Block entities drawn from models: chests, shulker boxes, skulls and banners.

pub mod affine;
pub mod model;
pub mod place;
pub mod renderer;

#[cfg(test)]
mod golden;

#[cfg(test)]
mod tests {
    use super::model::{Model, ModelSet, Pose};
    use super::place::{Kind, chest_half, transform};

    /// Every state's posed corners against the game's own, which `BlockEntityGolden`
    /// writes. They are the game's geometry, so neither file is ever committed: run with
    /// `RUTHENIUM_MODEL_LAYERS` (the export) and `RUTHENIUM_CORNERS` set.
    #[test]
    #[ignore = "needs the game's own models"]
    fn corners_are_the_games() {
        let read = |var: &str| std::fs::read(std::env::var(var).expect(var)).unwrap();
        let layers =
            ruthenium_prepare::exporters::read_model_layers(&read("RUTHENIUM_MODEL_LAYERS"))
                .unwrap();
        let set = ModelSet::new(&layers).unwrap();
        let bytes = read("RUTHENIUM_CORNERS");
        let words = bytes.as_chunks::<4>().0;
        let (mut at, mut compared, mut worst) = (0, 0u64, 0f32);
        while at < words.len() {
            let state = u32::from_le_bytes(words[at]);
            let p = f32::from_le_bytes(words[at + 1]);
            let count = u32::from_le_bytes(words[at + 2]) as usize;
            let want = &words[at + 3..at + 3 + count];
            at += 3 + count;
            let kind = Kind::of(state).expect("a kind for every state the game has one for");
            let models = match kind {
                Kind::Chest(_) => vec![(chest_half(state).0, Pose::Chest(p))],
                Kind::Shulker(_) => vec![(Model::Shulker, Pose::Shulker(p))],
                Kind::Skull { skull, .. } => vec![(Model::Skull(skull), Pose::Skull(p))],
                Kind::Banner { wall: false, .. } => vec![
                    (Model::StandingBanner, Pose::Rest),
                    (Model::StandingFlag, Pose::Flag(p)),
                ],
                Kind::Banner { wall: true, .. } => vec![
                    (Model::WallBanner, Pose::Rest),
                    (Model::WallFlag, Pose::Flag(p)),
                ],
            };
            let outer = transform(kind, state);
            let mut got = Vec::new();
            for (model, pose) in models {
                let mut parts = Vec::new();
                set.draw(model, pose, &mut parts);
                for part in parts {
                    let t = outer.then_inner(part.transform);
                    let first = part.first_quad as usize;
                    for face in &set.quads()[first..first + part.quads as usize] {
                        for c in face.corners {
                            got.extend(t.point([c[0] / 16.0, c[1] / 16.0, c[2] / 16.0]));
                        }
                    }
                }
            }
            assert_eq!(got.len(), want.len(), "state {state}, pose {p}");
            for (g, w) in got.iter().zip(want) {
                worst = worst.max((g - f32::from_le_bytes(*w)).abs());
                compared += 1;
            }
            assert!(worst < 1e-5, "state {state}, pose {p}: off by {worst}");
        }
        assert!(compared > 0);
        eprintln!("{compared} coordinates, the worst off by {worst}");
    }
}
