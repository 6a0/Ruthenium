//! Packing sprites into an atlas the way 26.3 does
//! (`client/renderer/texture/Stitcher.java`, `SpriteLoader.java`).
//!
//! The layout itself is internal (UVs come from it), but matching it keeps
//! atlas sizes, and so texel-size dependent sampling, identical to vanilla.

/// Mip levels an atlas can have: the requested count, lowered so every
/// sprite's size stays a multiple of `2^levels` and no sprite shrinks below
/// one texel.
pub fn mip_levels(requested: u32, sizes: impl IntoIterator<Item = (u32, u32)>) -> u32 {
    let mut lowest = 1u32 << requested;
    let mut min_texel = u32::MAX;
    for (w, h) in sizes {
        min_texel = min_texel.min(w.min(h));
        let bit = (1 << w.trailing_zeros()).min(1 << h.trailing_zeros());
        lowest = lowest.min(bit);
    }
    let min = min_texel.min(lowest);
    requested.min(min.max(1).ilog2())
}

/// Texels of padding around each sprite: one mip block, times the
/// anisotropy factor when anisotropic filtering is on.
pub fn padding(mip_levels: u32, anisotropy_bit: u32) -> u32 {
    (1 << mip_levels) << anisotropy_bit.saturating_sub(1).min(4)
}

/// Where a sprite went: the top-left of its padded slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug)]
struct Region {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    item: Option<usize>,
    /// True once this node or all of its children are completely filled, pruning O(N^2)
    /// walks over full subtrees.
    full: bool,
    children: Option<Vec<Region>>,
}

impl Drop for Region {
    fn drop(&mut self) {
        // Iterative drop so deep narrow chains cannot overflow the stack.
        let mut stack = Vec::new();
        if let Some(c) = self.children.take() {
            stack.extend(c);
        }
        while let Some(mut node) = stack.pop() {
            if let Some(c) = node.children.take() {
                stack.extend(c);
            }
        }
    }
}

impl Region {
    fn new(x: u32, y: u32, w: u32, h: u32) -> Region {
        Region {
            x,
            y,
            w,
            h,
            item: None,
            full: false,
            children: None,
        }
    }

    fn add(&mut self, item: usize, iw: u32, ih: u32) -> bool {
        if self.full || self.item.is_some() || iw > self.w || ih > self.h {
            return false;
        }
        if iw == self.w && ih == self.h {
            self.item = Some(item);
            self.full = true;
            return true;
        }
        let children = self.children.get_or_insert_with(|| {
            let mut c = vec![Region::new(self.x, self.y, iw, ih)];
            let (spare_w, spare_h) = (self.w - iw, self.h - ih);
            if spare_w > 0 && spare_h > 0 {
                if self.h.max(spare_w) >= self.w.max(spare_h) {
                    c.push(Region::new(self.x, self.y + ih, iw, spare_h));
                    c.push(Region::new(self.x + iw, self.y, spare_w, self.h));
                } else {
                    c.push(Region::new(self.x + iw, self.y, spare_w, ih));
                    c.push(Region::new(self.x, self.y + ih, self.w, spare_h));
                }
            } else if spare_w == 0 {
                c.push(Region::new(self.x, self.y + ih, iw, spare_h));
            } else {
                c.push(Region::new(self.x + iw, self.y, spare_w, ih));
            }
            c
        });
        let added = children.iter_mut().any(|c| c.add(item, iw, ih));
        if added && children.iter().all(|c| c.full) {
            self.full = true;
        }
        added
    }

    fn walk(&self, out: &mut [Option<Placement>]) {
        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            if let Some(i) = node.item {
                out[i] = Some(Placement {
                    x: node.x,
                    y: node.y,
                });
            } else if let Some(c) = &node.children {
                for r in c {
                    stack.push(r);
                }
            }
        }
    }
}

/// `Mth.smallestEncompassingPowerOfTwo` (0 stays 0), in 64 bits so that it cannot
/// overflow.
fn pow2_ceil(v: impl Into<u64>) -> u64 {
    let v = v.into();
    if v == 0 { 0 } else { v.next_power_of_two() }
}

/// The finished layout.
#[derive(Debug)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    /// Per input sprite, in input order.
    pub placements: Vec<Placement>,
}

/// A sprite (index `.0`) that did not fit.
#[derive(Debug)]
pub struct DoesNotFit(pub usize);

/// Packs sprites of the given sizes. `order` must sort the inputs the way
/// vanilla breaks ties (by identifier); size ordering is applied here.
pub fn stitch(
    sizes: &[(u32, u32)],
    mut order: impl FnMut(usize, usize) -> std::cmp::Ordering,
    mip_levels: u32,
    padding: u32,
    max_width: u32,
    max_height: u32,
) -> Result<Layout, DoesNotFit> {
    // In 64 bits: a slot of a sprite as large as a pack may claim cannot overflow, and
    // one larger than the atlas does not fit.
    let round = |v: u64| v.div_ceil(1 << mip_levels.min(31)) << mip_levels.min(31);
    let pad = 2 * u64::from(padding);
    let slots: Vec<(u64, u64)> = sizes
        .iter()
        .map(|&(w, h)| (round(u64::from(w) + pad), round(u64::from(h) + pad)))
        .collect();
    if let Some(i) = slots
        .iter()
        .position(|&(w, h)| w > u64::from(max_width) || h > u64::from(max_height))
    {
        return Err(DoesNotFit(i));
    }
    // Both now fit in 32 bits.
    let slots: Vec<(u32, u32)> = slots.iter().map(|&(w, h)| (w as u32, h as u32)).collect();
    let mut items: Vec<usize> = (0..sizes.len()).collect();
    items.sort_by(|&a, &b| {
        slots[b]
            .1
            .cmp(&slots[a].1)
            .then(slots[b].0.cmp(&slots[a].0))
            .then_with(|| order(a, b))
    });

    let (mut width, mut height) = (0u32, 0u32);
    let mut storage: Vec<Region> = Vec::new();
    for &i in &items {
        let (w, h) = slots[i];
        if storage.iter_mut().any(|r| r.add(i, w, h)) {
            continue;
        }
        let (cur_x, cur_y) = (pow2_ceil(width), pow2_ceil(height));
        let grown = |at: u32, by: u32| pow2_ceil(u64::from(at) + u64::from(by));
        let (new_x, new_y) = (grown(width, w), grown(height, h));
        let (can_x, can_y) = (
            new_x <= u64::from(max_width),
            new_y <= u64::from(max_height),
        );
        if !can_x && !can_y {
            return Err(DoesNotFit(i));
        }
        // What can grow is within the maximum, so in 32 bits.
        let (new_x, new_y) = (
            new_x.min(u64::from(u32::MAX)) as u32,
            new_y.min(u64::from(u32::MAX)) as u32,
        );
        let (cur_x, cur_y) = (cur_x as u32, cur_y as u32);
        let (will_x, will_y) = (can_x && cur_x != new_x, can_y && cur_y != new_y);
        let grow_x = if will_x ^ will_y {
            will_x
        } else {
            can_x && cur_x <= cur_y
        };
        let mut region = if grow_x {
            if height == 0 {
                height = new_y;
            }
            let r = Region::new(width, 0, new_x - width, height);
            width = new_x;
            r
        } else {
            let r = Region::new(0, height, width, new_y - height);
            height = new_y;
            r
        };
        // Vanilla ignores whether this fits; with power-of-two growth it does
        // except in degenerate cases, which then fail the gather below.
        region.add(i, w, h);
        storage.push(region);
    }
    let mut placed = vec![None; sizes.len()];
    for r in &storage {
        r.walk(&mut placed);
    }
    let placements = placed
        .into_iter()
        .enumerate()
        .map(|(i, p)| p.ok_or(DoesNotFit(i)))
        .collect::<Result<_, _>>()?;
    Ok(Layout {
        width,
        height,
        placements,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_levels_follow_smallest_sprite() {
        assert_eq!(mip_levels(4, [(16, 16), (32, 16)]), 4);
        assert_eq!(mip_levels(4, [(16, 16), (8, 8)]), 3);
        assert_eq!(mip_levels(4, [(16, 16), (24, 16)]), 3);
        assert_eq!(mip_levels(4, [(16, 16), (16, 1)]), 0);
        assert_eq!(mip_levels(0, [(16, 16)]), 0);
    }

    #[test]
    fn padding_and_sizes() {
        assert_eq!(padding(0, 0), 1);
        assert_eq!(padding(4, 0), 16);
        assert_eq!(padding(4, 1), 16);
        assert_eq!(padding(4, 3), 64);
        assert_eq!(padding(2, 9), 64);
    }

    #[test]
    fn packs_without_overlap() {
        let sizes: Vec<(u32, u32)> = (0..40)
            .map(|i| (8 + (i % 5) * 4, 8 + (i % 3) * 8))
            .collect();
        let layout = stitch(&sizes, |a, b| a.cmp(&b), 0, 1, 4096, 4096).unwrap();
        assert!(layout.width.is_power_of_two() && layout.height.is_power_of_two());
        let rects: Vec<_> = layout
            .placements
            .iter()
            .zip(&sizes)
            .map(|(p, &(w, h))| (p.x, p.y, w + 2, h + 2))
            .collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a.0 + a.2 <= layout.width && a.1 + a.3 <= layout.height);
            for b in &rects[i + 1..] {
                let apart =
                    a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn sizes_a_pack_may_claim_do_not_overflow() {
        let order = |a: usize, b: usize| a.cmp(&b);
        for sizes in [
            vec![(u32::MAX, 1)],
            vec![(1, u32::MAX)],
            vec![(u32::MAX - 1, u32::MAX - 1); 3],
        ] {
            assert!(stitch(&sizes, order, 4, 1 << 8, 16384, 16384).is_err());
            assert!(stitch(&sizes, order, 4, 1 << 8, u32::MAX, u32::MAX).is_err());
        }
        // Many that each fit and together do not.
        let many = vec![(4096, 4096); 64];
        assert!(stitch(&many, order, 0, 1, 16384, 16384).is_err());
    }

    #[test]
    fn first_sprite_sets_the_size() {
        // One 16x16 sprite with padding 1: an 18x18 slot in a 32x32 atlas.
        let layout = stitch(&[(16, 16)], |a, b| a.cmp(&b), 0, 1, 1024, 1024).unwrap();
        assert_eq!((layout.width, layout.height), (32, 32));
        assert_eq!(layout.placements[0], Placement { x: 0, y: 0 });
        assert!(stitch(&[(2000, 16)], |a, b| a.cmp(&b), 0, 1, 1024, 1024).is_err());
    }
}
