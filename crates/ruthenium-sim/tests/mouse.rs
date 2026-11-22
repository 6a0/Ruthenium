//! The mouse's turn against vanilla's: `mouse_turn` moves the cursor 25 points right
//! for 40 ticks, then 40 left and 6 down for 20, at vanilla's default sensitivity. The
//! moves go through [`input::MouseMotion`] (which drops the first, as vanilla did in the
//! recording), [`input::turn_amount`] and [`LocalPlayer::turn`], and every rotation the
//! trace records must come out bit for bit. Needs only the committed trace.

use std::path::Path;

use ruthenium_sim::input::{self, MouseMotion, MouseOptions};
use ruthenium_sim::player::LocalPlayer;
use ruthenium_trace::Trace;

#[test]
fn mouse_turn_matches_vanilla_bit_for_bit() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/26.3/mouse_turn.jsonl");
    let t = Trace::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    let sensitivity = t.header["options"]["sensitivity"].as_f64().unwrap();
    let options = MouseOptions {
        sensitivity,
        ..MouseOptions::DEFAULT
    };
    let mut p = LocalPlayer::new();
    p.y_rot = 0.0;
    p.x_rot = 0.0;
    // The scenario's cursor moves, one per tick.
    let mut moves =
        std::iter::repeat_n((25.0f32, 0.0f32), 40).chain(std::iter::repeat_n((-40.0, 6.0), 20));
    let mut motion = MouseMotion::new();
    let mut turns = 0;
    let mut checked = 0;
    for r in t
        .records
        .iter()
        .filter(|r| r.kind == "tick" || r.kind == "state")
    {
        let want = (r.f32("yRot").unwrap(), r.f32("xRot").unwrap());
        if r.kind == "tick" && want != (p.y_rot, p.x_rot) {
            // The frames since the last tick turned the camera once, with the next move
            // that counts.
            loop {
                let (dx, dy) = moves.next().expect("more turns than the scenario made");
                motion.moved(dx, dy, true);
                let (dx, dy) = motion.take();
                if (dx, dy) != (0.0, 0.0) {
                    let (xo, yo) = input::turn_amount(dx, dy, &options);
                    p.turn(xo, yo);
                    turns += 1;
                    break;
                }
            }
        }
        assert_eq!(
            (p.y_rot.to_bits(), p.x_rot.to_bits()),
            (want.0.to_bits(), want.1.to_bits()),
            "{} {:?}: got {:?}, vanilla {want:?} after {turns} turns",
            r.kind,
            r.tick(),
            (p.y_rot, p.x_rot)
        );
        checked += 1;
    }
    assert_eq!(turns, 59, "every turn but the first, dropped");
    assert_eq!(moves.next(), None);
    assert!(checked > 100);
}
