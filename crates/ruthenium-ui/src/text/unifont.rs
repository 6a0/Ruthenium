//! The game's GNU Unifont, the menus' last fallback (as it is vanilla's): glyphs are
//! pictures 16 pixels high, so each is drawn from an outline around its set pixels, and
//! set without shaping, at the advance vanilla gives it.
//!
//! Sizes: Unifont is a 16-pixel font, its rows one em, 14 of them above the baseline
//! (the ascent its own outline builds have, and vanilla's: 7 of the 8 units the rows
//! take, on a baseline 7 units down). Next to Nunito that puts its ideographs where a
//! CJK font's are (an em tall, an eighth of it below the baseline) and its capitals at
//! 10/16 = 0.625 em, a little under Nunito's 0.705.

use fdsm::bezier::{Point, Segment};
use fdsm::shape::{Contour, Shape};
use ruthenium_assets::Font;

/// Rows to the em.
pub(crate) const ROWS_PER_EM: f64 = 16.0;
/// Rows above the baseline.
const ASCENT: f64 = 14.0;

/// `c`'s atlas key: its plane (in its [`FaceId::Unifont`](super::FaceId::Unifont)) and
/// its place there.
pub(crate) fn key(c: char) -> (u8, u16) {
    let c = u32::from(c);
    ((c >> 16) as u8, (c & 0xFFFF) as u16)
}

/// The character of an atlas key.
pub(crate) fn char_of(plane: u8, id: u16) -> Option<char> {
    char::from_u32(u32::from(plane) << 16 | u32::from(id))
}

/// `c`'s advance in ems: vanilla's, in its units of two rows.
pub(crate) fn advance(font: &Font, c: char) -> f32 {
    font.glyph(c).advance * 2.0 / ROWS_PER_EM as f32
}

/// Characters left out of a Unifont run: those that only join or select (Unifont draws
/// them as labelled boxes). Combining marks are drawn, at their own advance, as vanilla
/// draws them.
pub(crate) fn invisible(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}'
        | '\u{E0000}'..='\u{E007F}' | '\u{E0100}'..='\u{E01EF}' | '\u{1F3FB}'..='\u{1F3FF}')
}

/// The outline of `c`'s set pixels in rows from its pen position, y up from the baseline,
/// and its box (`x_min, y_min, x_max, y_max`); none for a glyph without ink.
pub(crate) fn outline(font: &Font, c: char) -> Option<(Shape<Contour>, [f64; 4])> {
    if !font.has(c) {
        return None;
    }
    let picture = font.texels(c)?;
    let loops = polygons(picture.width, picture.height, |x, y| {
        picture.get(x, y) >> 24 != 0
    });
    let mut bbox = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut shape = Shape::default();
    for corners in loops {
        let points: Vec<Point> = corners
            .iter()
            .map(|&(x, y)| Point::new(f64::from(x), ASCENT - f64::from(y)))
            .collect();
        for p in &points {
            bbox = [
                bbox[0].min(p.x),
                bbox[1].min(p.y),
                bbox[2].max(p.x),
                bbox[3].max(p.y),
            ];
        }
        let segments = (0..points.len())
            .map(|i| Segment::line(points[i], points[(i + 1) % points.len()]))
            .collect();
        shape.contours.push(Contour { segments });
    }
    (!shape.contours.is_empty()).then_some((shape, bbox))
}

/// Closed polygons (corners only) around the set pixels of a `width` x `height` picture,
/// in pixel corners, y down. Each one keeps its pixels on its left: as seen, a region's
/// edge goes round counterclockwise and a hole's clockwise, which once y points up is
/// TrueType's way round (outer edges clockwise), so the non-zero rule fills the pixels.
/// Pixels that touch only at a corner get an edge each.
pub(crate) fn polygons(
    width: u32,
    height: u32,
    set: impl Fn(u32, u32) -> bool,
) -> Vec<Vec<(i32, i32)>> {
    let (w, h) = (width as i32, height as i32);
    let ink = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && set(x as u32, y as u32);
    // Every unit edge between a set pixel and an unset one, by its start.
    let mut edges: std::collections::BTreeMap<(i32, i32), Vec<(i32, i32)>> = Default::default();
    let mut add = |from: (i32, i32), to: (i32, i32)| edges.entry(from).or_default().push(to);
    for y in 0..h {
        for x in 0..w {
            if !ink(x, y) {
                continue;
            }
            if !ink(x - 1, y) {
                add((x, y), (x, y + 1));
            }
            if !ink(x, y + 1) {
                add((x, y + 1), (x + 1, y + 1));
            }
            if !ink(x + 1, y) {
                add((x + 1, y + 1), (x + 1, y));
            }
            if !ink(x, y - 1) {
                add((x + 1, y), (x, y));
            }
        }
    }
    let mut out = Vec::new();
    while let Some((&start, _)) = edges.iter().next() {
        let mut at = start;
        let mut heading: Option<(i32, i32)> = None;
        let mut path = vec![start];
        loop {
            let nexts = edges
                .get_mut(&at)
                .expect("an edge goes on from every corner");
            // Where two pixels touch at a corner, turn left: around the pixel just
            // followed, so that each keeps its own edge.
            let pick = match heading {
                Some((dx, dy)) if nexts.len() > 1 => nexts
                    .iter()
                    .position(|&(x, y)| (x - at.0, y - at.1) == (dy, -dx))
                    .unwrap_or(0),
                _ => 0,
            };
            let next = nexts.swap_remove(pick);
            if nexts.is_empty() {
                edges.remove(&at);
            }
            heading = Some((next.0 - at.0, next.1 - at.1));
            at = next;
            if at == start {
                break;
            }
            path.push(at);
        }
        // Corners only: drop the points the edge goes straight through.
        let n = path.len();
        let corners: Vec<(i32, i32)> = (0..n)
            .filter(|&i| {
                let (p, c, q) = (path[(i + n - 1) % n], path[i], path[(i + 1) % n]);
                (c.0 - p.0, c.1 - p.1) != (q.0 - c.0, q.1 - c.1)
            })
            .map(|i| path[i])
            .collect();
        out.push(corners);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture<'a>(rows: &'a [&'a str]) -> impl Fn(u32, u32) -> bool + 'a {
        |x, y| rows[y as usize].as_bytes()[x as usize] == b'#'
    }

    /// Twice the signed area (positive: counterclockwise as seen, y down).
    fn area(p: &[(i32, i32)]) -> i32 {
        (0..p.len())
            .map(|i| {
                let (a, b) = (p[i], p[(i + 1) % p.len()]);
                a.1 * b.0 - a.0 * b.1
            })
            .sum()
    }

    #[test]
    fn pixels_merge_into_one_polygon_and_holes_go_the_other_way() {
        let ring = ["####", "#..#", "####"];
        let loops = polygons(4, 3, picture(&ring));
        assert_eq!(loops.len(), 2, "{loops:?}");
        let mut areas: Vec<i32> = loops.iter().map(|l| area(l)).collect();
        areas.sort();
        // The hole (two pixels) one way, the outer edge (twelve) the other.
        assert_eq!(areas, [-4, 24]);
        // Corners only.
        assert!(loops.iter().all(|l| l.len() == 4), "{loops:?}");
        // An L: six corners.
        let l = polygons(2, 2, picture(&["#.", "##"]));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].len(), 6);
        assert_eq!(area(&l[0]), 6);
    }

    #[test]
    fn pixels_touching_at_a_corner_keep_their_own_edges() {
        let loops = polygons(2, 2, picture(&["#.", ".#"]));
        assert_eq!(loops.len(), 2, "{loops:?}");
        assert!(loops.iter().all(|l| l.len() == 4 && area(l) == 2));
        let loops = polygons(2, 2, picture(&[".#", "#."]));
        assert_eq!(loops.len(), 2, "{loops:?}");
        assert!(polygons(3, 1, picture(&["..."])).is_empty());
    }

    #[test]
    fn keys_keep_the_plane() {
        for c in ['A', '\u{FFFD}', '\u{1E5D0}', '\u{2A6D6}'] {
            let (plane, id) = key(c);
            assert_eq!(char_of(plane, id), Some(c));
        }
        assert_eq!(key('\u{1E5D0}'), (1, 0xE5D0));
    }
}
