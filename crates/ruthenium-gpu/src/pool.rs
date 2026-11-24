//! Where byte ranges live inside pages of GPU buffer memory. Two layers, both pure
//! bookkeeping (nothing here talks to the GPU; the caller creates and destroys the pages):
//!
//! * [`RangeAllocator`] hands out byte ranges of one page. Free blocks are kept in bins by
//!   size class with bitmaps over the bins, so finding a block is a few bit operations, and
//!   a freed block joins its free neighbours at once.
//! * [`RangePool`] owns a set of equally sized pages, hands out ranges by generational
//!   handle, and frees a retired range only when the GPU work that used it has finished. It
//!   also says when another page is worth creating and which empty page may go.
//!
//! Ranges never move: there is no compaction, so an offset stays valid until its range is
//! reclaimed.

use crate::users::{Completed, Users};

/// Marks the end of an index-linked list, or a neighbour that does not exist.
const NIL: u32 = u32::MAX;

/// Every power-of-two span of sizes is split into two-to-the-power-of-this many bins.
const SUB_BITS: u32 = 3;
/// How many bins each power-of-two span of sizes has.
const SUB_BINS: u32 = 1 << SUB_BITS;

/// The bin a free block of this many granules is kept in. Sizes below `SUB_BINS` have a bin
/// each; above that every power-of-two span is cut into `SUB_BINS` equal steps, so a bin holds
/// sizes within an eighth of each other.
fn class_floor(granules: u64) -> u32 {
    if granules < u64::from(SUB_BINS) {
        granules as u32
    } else {
        let shift = granules.ilog2() - SUB_BITS;
        let step = (granules >> shift) as u32 & (SUB_BINS - 1);
        ((shift + 1) << SUB_BITS) | step
    }
}

/// The lowest bin in which every block has at least this many granules (which must not be
/// zero). None when the size is too close to the top of `u64` to round up.
fn class_ceil(granules: u64) -> Option<u32> {
    let shift = granules.ilog2().saturating_sub(SUB_BITS);
    let rounded = granules.checked_add((1 << shift) - 1)?;
    Some(class_floor(rounded))
}

/// `size` rounded up to whole granules, or None when that overflows. Panics unless `align`
/// is a non-zero multiple of the granule: offsets are only ever multiples of the granule, so
/// no other alignment can be honoured.
fn whole_granules(size: u64, align: u64, granule: u64) -> Option<u64> {
    assert!(
        align != 0 && align.is_multiple_of(granule),
        "the alignment ({align}) must be a non-zero multiple of the granule ({granule})"
    );
    size.checked_next_multiple_of(granule)
}

/// The index a new entry gets in a slab that has `len` entries. Panics when the slab has
/// outgrown what a `u32` index (with `NIL` set aside) can name.
fn slab_index(len: usize) -> u32 {
    u32::try_from(len)
        .ok()
        .filter(|&index| index != NIL)
        .expect("more entries than a u32 index can name")
}

/// An allocated range of one page, as [`RangeAllocator::alloc`] returned it. Give the same
/// value back to [`RangeAllocator::free`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    /// Where the range starts in the page: a multiple of the alignment asked for.
    pub offset: u64,
    /// The size asked for, rounded up to whole granules.
    pub size: u64,
    /// The allocator's own name for the block; it has no meaning to the caller.
    pub node: u32,
}

/// What a slot of the node slab holds at the moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeState {
    /// Free bytes, listed in the bin for their size.
    Free,
    /// Bytes handed out by `alloc`.
    Allocated,
    /// No bytes: the slot waits on the spare list to be used again.
    Spare,
}

/// One block of the page, free or allocated. The blocks tile the page; each knows its
/// neighbours by offset, and a free one also its neighbours in its bin.
#[derive(Clone, Copy, Debug)]
struct Node {
    offset: u64,
    size: u64,
    /// The block that ends where this one starts.
    prev: u32,
    /// The block that starts where this one ends. For a spare slot, the next spare slot.
    next: u32,
    /// The neighbours in the bin's list; meaningful only while the block is free.
    bin_prev: u32,
    bin_next: u32,
    state: NodeState,
}

/// Hands out byte ranges of one page. Free blocks are binned by size class (see
/// [`class_floor`]) with a bitmap per group of bins and one over the groups, so the first
/// non-empty bin at or above a size is found without walking. A block is split when a range
/// is taken from it and merged with its free neighbours when the range comes back, so two
/// free blocks never touch. Blocks are slots of one vector linked by index, and slots are
/// reused, so nothing is allocated on the heap once the vector has grown to its working size.
#[derive(Debug)]
pub(crate) struct RangeAllocator {
    size: u64,
    granule: u64,
    /// The blocks. Slot 0 is always the block that starts at offset 0: splitting keeps the
    /// front part in the old slot and merging keeps the earlier block's slot.
    nodes: Vec<Node>,
    /// The first spare slot of `nodes`.
    spare: u32,
    /// The first free block of each size class.
    bins: Vec<u32>,
    /// For each group of `SUB_BINS` bins, a bit per bin that has blocks.
    groups: Vec<u8>,
    /// A bit per group that has blocks.
    top: u64,
    free_bytes: u64,
    allocated: u32,
}

impl RangeAllocator {
    /// An allocator over offsets `0..size`, all free. Every offset and size it hands out is
    /// a multiple of `granule`. Panics unless the granule is a power of two and the size a
    /// non-zero multiple of it.
    pub(crate) fn new(size: u64, granule: u64) -> RangeAllocator {
        assert!(
            granule.is_power_of_two(),
            "the granule ({granule}) must be a power of two"
        );
        assert!(
            size != 0 && size.is_multiple_of(granule),
            "the size ({size}) must be a non-zero multiple of the granule ({granule})"
        );
        let classes = class_floor(size / granule) + 1;
        let whole = Node {
            offset: 0,
            size,
            prev: NIL,
            next: NIL,
            bin_prev: NIL,
            bin_next: NIL,
            state: NodeState::Free,
        };
        let mut ranges = RangeAllocator {
            size,
            granule,
            nodes: vec![whole],
            spare: NIL,
            bins: vec![NIL; classes as usize],
            groups: vec![0; classes.div_ceil(SUB_BINS) as usize],
            top: 0,
            free_bytes: size,
            allocated: 0,
        };
        ranges.bin(0);
        ranges
    }

    /// Takes `size` bytes (not zero; rounded up to whole granules) at an offset that is a
    /// multiple of `align`, which may be any non-zero multiple of the granule, not only a
    /// power of two. The bytes of the chosen free block before and after the range stay free
    /// as blocks of their own. None only when no free interval of the page can hold the
    /// aligned range.
    pub(crate) fn alloc(&mut self, size: u64, align: u64) -> Option<Block> {
        let need = self.need(size, align)?;
        let (node, start) = self.find(need, align)?;
        Some(self.carve(node, start, need))
    }

    /// Whether `alloc` with the same arguments would succeed right now.
    #[cfg(test)]
    pub(crate) fn fits(&self, size: u64, align: u64) -> bool {
        self.need(size, align)
            .is_some_and(|need| self.find(need, align).is_some())
    }

    /// Gives a range back. It joins a free neighbour on either side at once. Panics when the
    /// block is not one that is currently allocated here.
    pub(crate) fn free(&mut self, block: Block) {
        let held = self.nodes.get(block.node as usize).is_some_and(|node| {
            node.state == NodeState::Allocated
                && node.offset == block.offset
                && node.size == block.size
        });
        assert!(held, "freeing {block:?}, which is not allocated");
        self.free_bytes += block.size;
        self.allocated -= 1;

        let mut node = block.node;
        let prev = self.nodes[node as usize].prev;
        if prev != NIL && self.nodes[prev as usize].state == NodeState::Free {
            // The block before is free: it grows over this one, whose slot becomes spare.
            self.unbin(prev);
            self.nodes[prev as usize].size += block.size;
            self.remove(node);
            node = prev;
        }
        let next = self.nodes[node as usize].next;
        if next != NIL && self.nodes[next as usize].state == NodeState::Free {
            self.unbin(next);
            self.nodes[node as usize].size += self.nodes[next as usize].size;
            self.remove(next);
        }
        self.bin(node);
    }

    /// How many bytes are free, over all free blocks.
    pub(crate) fn free_bytes(&self) -> u64 {
        self.free_bytes
    }

    /// The size of the largest single free block: the most that an allocation with the
    /// granule as its alignment can get. Walks only the highest bin that has blocks, since
    /// every block in it is larger than any block in a lower bin.
    pub(crate) fn largest_free(&self) -> u64 {
        if self.top == 0 {
            return 0;
        }
        let group = self.top.ilog2();
        let bin = (group << SUB_BITS) | self.groups[group as usize].ilog2();
        let mut largest = 0;
        let mut node = self.bins[bin as usize];
        while node != NIL {
            largest = largest.max(self.nodes[node as usize].size);
            node = self.nodes[node as usize].bin_next;
        }
        largest
    }

    /// The size of the page.
    pub(crate) fn size(&self) -> u64 {
        self.size
    }

    /// Whether nothing is allocated: the whole page is one free block.
    pub(crate) fn is_empty(&self) -> bool {
        self.allocated == 0
    }

    /// How many ranges are allocated.
    #[cfg(test)]
    pub(crate) fn allocated_blocks(&self) -> u32 {
        self.allocated
    }

    /// The bytes a request really takes: its size in whole granules. None when that can never
    /// fit. Panics on a request the allocator cannot make sense of.
    fn need(&self, size: u64, align: u64) -> Option<u64> {
        assert!(size != 0, "a range must have at least one byte");
        whole_granules(size, align, self.granule)
    }

    /// The size of a block in granules, which is what the bins are sorted by.
    fn granules(&self, bytes: u64) -> u64 {
        bytes >> self.granule.trailing_zeros()
    }

    /// The first bin at or above `class` that has blocks.
    fn first_bin_from(&self, class: u32) -> Option<u32> {
        let group = class >> SUB_BITS;
        let bits = *self.groups.get(group as usize)?;
        let within = bits & (u8::MAX << (class & (SUB_BINS - 1)));
        if within != 0 {
            return Some((group << SUB_BITS) | within.trailing_zeros());
        }
        // Nothing in the rest of this group: the next group that has anything, if any. There
        // are fewer than 64 groups, so the shift by one more cannot overflow.
        let above = self.top & (u64::MAX << (group + 1));
        if above == 0 {
            return None;
        }
        let group = above.trailing_zeros();
        Some((group << SUB_BITS) | self.groups[group as usize].trailing_zeros())
    }

    /// The free block to take `need` bytes aligned to `align` from, and the offset the
    /// range gets. None only when no free block can hold the aligned range.
    fn find(&self, need: u64, align: u64) -> Option<(u32, u64)> {
        if need > self.free_bytes {
            return None;
        }
        // The quick way. A block starts on a granule, so aligning its start skips at most
        // `align - granule` bytes: a block that much larger than the range fits wherever it
        // starts, and the bitmaps find the first bin that holds only such blocks.
        if let Some(roomy) = need.checked_add(align - self.granule)
            && let Some(class) = class_ceil(self.granules(roomy))
            && let Some(bin) = self.first_bin_from(class)
        {
            let node = self.bins[bin as usize];
            let start = self.nodes[node as usize].offset.next_multiple_of(align);
            return Some((node, start));
        }
        // No block is roomy enough to fit blindly, but a smaller one may still fit: one of
        // exactly the size, or one that happens to start on the alignment. Look at every
        // block from the bin that can hold the bare size upward. They all sit in the few bins
        // below the one the quick way wanted, so this is a short walk.
        let mut class = class_floor(self.granules(need));
        while let Some(bin) = self.first_bin_from(class) {
            let mut node = self.bins[bin as usize];
            while node != NIL {
                let block = &self.nodes[node as usize];
                if let Some(start) = block.offset.checked_next_multiple_of(align)
                    && let Some(end) = start.checked_add(need)
                    && end <= block.offset + block.size
                {
                    return Some((node, start));
                }
                node = block.bin_next;
            }
            class = bin + 1;
        }
        None
    }

    /// Takes `start..start + need` out of the free block `node`, which must contain it. What
    /// the block has before and after the range stays free.
    fn carve(&mut self, node: u32, start: u64, need: u64) -> Block {
        self.unbin(node);
        let Node { offset, size, .. } = self.nodes[node as usize];
        let end = offset + size;
        let mut taken = node;
        if start > offset {
            // The bytes skipped for alignment stay free in the old slot. The block before
            // them is allocated (free blocks never touch), so there is nothing to merge with.
            self.nodes[node as usize].size = start - offset;
            self.bin(node);
            taken = self.insert_after(node, start);
        }
        if start + need < end {
            let rest = self.insert_after(taken, start + need);
            self.nodes[rest as usize].size = end - (start + need);
            self.bin(rest);
        }
        self.nodes[taken as usize].size = need;
        self.nodes[taken as usize].state = NodeState::Allocated;
        self.free_bytes -= need;
        self.allocated += 1;
        Block {
            offset: start,
            size: need,
            node: taken,
        }
    }

    /// Puts a block on the list of the bin for its size and marks it free.
    fn bin(&mut self, node: u32) {
        let class = class_floor(self.granules(self.nodes[node as usize].size));
        let head = self.bins[class as usize];
        let block = &mut self.nodes[node as usize];
        block.state = NodeState::Free;
        block.bin_prev = NIL;
        block.bin_next = head;
        if head != NIL {
            self.nodes[head as usize].bin_prev = node;
        }
        self.bins[class as usize] = node;
        self.groups[(class >> SUB_BITS) as usize] |= 1 << (class & (SUB_BINS - 1));
        self.top |= 1 << (class >> SUB_BITS);
    }

    /// Takes a free block off its bin's list, before its size changes or it is handed out.
    fn unbin(&mut self, node: u32) {
        let Node {
            size,
            bin_prev,
            bin_next,
            ..
        } = self.nodes[node as usize];
        if bin_next != NIL {
            self.nodes[bin_next as usize].bin_prev = bin_prev;
        }
        if bin_prev != NIL {
            self.nodes[bin_prev as usize].bin_next = bin_next;
            return;
        }
        // It was the first of its bin: the bin now starts at the next one, or is empty.
        let class = class_floor(self.granules(size));
        self.bins[class as usize] = bin_next;
        if bin_next == NIL {
            let group = (class >> SUB_BITS) as usize;
            self.groups[group] &= !(1 << (class & (SUB_BINS - 1)));
            if self.groups[group] == 0 {
                self.top &= !(1 << group);
            }
        }
    }

    /// A new block that starts at `offset`, placed right after `before` in offset order. The
    /// caller gives it its size and state. Uses a spare slot when there is one.
    fn insert_after(&mut self, before: u32, offset: u64) -> u32 {
        let next = self.nodes[before as usize].next;
        let block = Node {
            offset,
            size: 0,
            prev: before,
            next,
            bin_prev: NIL,
            bin_next: NIL,
            state: NodeState::Allocated,
        };
        let node = if self.spare == NIL {
            let node = slab_index(self.nodes.len());
            self.nodes.push(block);
            node
        } else {
            let node = self.spare;
            self.spare = self.nodes[node as usize].next;
            self.nodes[node as usize] = block;
            node
        };
        self.nodes[before as usize].next = node;
        if next != NIL {
            self.nodes[next as usize].prev = node;
        }
        node
    }

    /// Takes a block whose bytes a neighbour has absorbed out of the offset order and puts
    /// its slot on the spare list. It always has a block before it: the first block of the
    /// page is never the one absorbed.
    fn remove(&mut self, node: u32) {
        let Node { prev, next, .. } = self.nodes[node as usize];
        self.nodes[prev as usize].next = next;
        if next != NIL {
            self.nodes[next as usize].prev = prev;
        }
        let slot = &mut self.nodes[node as usize];
        slot.state = NodeState::Spare;
        slot.next = self.spare;
        self.spare = node;
    }

    /// Checks everything the allocator relies on: the blocks tile the page in order, no two
    /// free blocks touch, every free block is on the list of the right bin and nothing else
    /// is, the bitmaps say exactly which bins have blocks, and the counters add up.
    #[cfg(test)]
    fn check(&self) {
        let mut free_bytes = 0;
        let mut free_blocks = 0;
        let mut allocated = 0;
        let mut offset = 0;
        let mut prev = NIL;
        let mut node = 0;
        while node != NIL {
            let block = &self.nodes[node as usize];
            assert_eq!(
                block.offset, offset,
                "block {node} leaves a gap or overlaps"
            );
            assert_eq!(
                block.prev, prev,
                "block {node} has the wrong block before it"
            );
            assert!(block.size > 0 && block.size.is_multiple_of(self.granule));
            match block.state {
                NodeState::Free => {
                    assert!(
                        prev == NIL || self.nodes[prev as usize].state != NodeState::Free,
                        "free blocks {prev} and {node} touch"
                    );
                    free_bytes += block.size;
                    free_blocks += 1;
                }
                NodeState::Allocated => allocated += 1,
                NodeState::Spare => panic!("spare slot {node} is among the blocks"),
            }
            offset += block.size;
            prev = node;
            node = block.next;
        }
        assert_eq!(
            offset, self.size,
            "the blocks do not end where the page does"
        );
        assert_eq!(free_bytes, self.free_bytes);
        assert_eq!(allocated, self.allocated);

        let mut binned = 0;
        for (class, &head) in self.bins.iter().enumerate() {
            let bit = self.groups[class >> SUB_BITS] >> (class as u32 & (SUB_BINS - 1)) & 1;
            assert_eq!(
                bit == 1,
                head != NIL,
                "the bitmap is wrong about bin {class}"
            );
            let mut before = NIL;
            let mut node = head;
            while node != NIL {
                let block = &self.nodes[node as usize];
                assert_eq!(
                    block.state,
                    NodeState::Free,
                    "bin {class} lists a used block"
                );
                assert_eq!(class_floor(self.granules(block.size)) as usize, class);
                assert_eq!(block.bin_prev, before);
                binned += 1;
                before = node;
                node = block.bin_next;
            }
        }
        assert_eq!(binned, free_blocks, "a free block is on no bin's list");
        for (group, &bits) in self.groups.iter().enumerate() {
            assert_eq!(self.top >> group & 1 == 1, bits != 0);
        }
        assert_eq!(self.top >> self.groups.len(), 0);
    }
}

/// How a [`RangePool`] sizes itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolConfig {
    /// The size of every page: a non-zero multiple of the granule.
    pub page_bytes: u64,
    /// The most bytes of pages the pool may hold. Growth stops there.
    pub budget_bytes: u64,
    /// When the free bytes are at or below this, another page is wanted ahead of need.
    pub low_watermark: u64,
    /// An empty page is released only if the free bytes left without it stay above this.
    /// Not below the low watermark, or a released page would be asked for again at once.
    pub high_watermark: u64,
    /// ... and only after it has been empty for this many `reclaim` calls in a row.
    pub release_after: u32,
    /// Every offset and rounded size is a multiple of this power of two.
    pub granule: u64,
}

/// Names a range of a [`RangePool`]. It stops resolving when the range is retired, and never
/// resolves to a later range that reuses its slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RangeHandle {
    index: u32,
    generation: u32,
}

impl RangeHandle {
    /// Which use of its slot this handle names. Two handles the pool gave out for the same
    /// slot differ in this.
    pub fn generation(&self) -> u32 {
        self.generation
    }
}

/// Where a live range is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PooledRange {
    /// The index of its page.
    pub page: u32,
    /// Where it starts in the page.
    pub offset: u64,
    /// Its size as requested, before rounding up to whole granules.
    pub size: u64,
    /// The generation of its handle.
    pub generation: u32,
}

/// Why [`RangePool::alloc`] handed out nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolError {
    /// The request never fits a page (or is empty).
    TooLarge,
    /// Nothing fits now, but the budget allows another page: add one and retry.
    Growing,
    /// Nothing fits and the pool may not grow: reclaim finished ranges or give up.
    Full,
}

/// What a [`RangePool`] holds and what it has been through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    /// How many pages the pool has.
    pub pages: u32,
    /// The bytes of those pages: `pages * page_bytes`. Always the sum of the payload, the
    /// alignment waste, the retiring bytes and the free bytes.
    pub reserved_bytes: u64,
    /// The requested sizes of the live ranges.
    pub payload_bytes: u64,
    /// The rounded sizes of the retired ranges that are not reclaimed yet.
    pub retiring_bytes: u64,
    /// What rounding up to whole granules added to the live ranges.
    pub alignment_waste: u64,
    /// The bytes no range holds, over all pages.
    pub free_bytes: u64,
    /// The largest single free interval of any page.
    pub largest_free: u64,
    /// How many ranges are live.
    pub live: u32,
    /// How many ranges are retired and not reclaimed yet.
    pub retiring: u32,
    /// How many allocations found no room (`Growing` or `Full`; a request that is
    /// `TooLarge` says nothing about the pool and is not counted).
    pub failures: u64,
    /// How many of those failed although the pool had at least the rounded size free in
    /// total: the cost of fragmentation.
    pub failures_with_space: u64,
    /// The most bytes of pages the pool has held at once.
    pub peak_reserved_bytes: u64,
    /// The most payload it has held at once.
    pub peak_payload_bytes: u64,
    /// How many pages were ever added.
    pub pages_added: u32,
    /// How many pages were ever released.
    pub pages_released: u32,
}

/// One page of the pool.
#[derive(Debug)]
struct Page {
    ranges: RangeAllocator,
    /// How many `reclaim` calls in a row have ended with nothing allocated on the page.
    idle_ticks: u32,
}

/// What a slot of the record slab holds at the moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordState {
    /// A range its handle resolves to.
    Live,
    /// A retired range: no handle resolves to it, but its bytes are still allocated.
    Retiring,
    /// No range.
    Vacant,
}

/// One range of the pool, live or retiring, or a vacant slot.
#[derive(Clone, Copy, Debug)]
struct Record {
    /// What a handle must carry to resolve to this slot. Bumped when the range is retired,
    /// so the handle is stale from then on, also after the slot is used again.
    generation: u32,
    state: RecordState,
    page: u32,
    block: Block,
    /// The size as requested.
    requested: u64,
    /// Who must finish before a retired range is freed.
    users: Users,
    /// The next retiring record, or the next spare slot.
    next: u32,
}

/// A set of equally sized pages with ranges handed out by handle. A range is live from
/// `alloc` until `retire`; after that its bytes stay allocated until `reclaim` sees that
/// the GPU work named at retirement has finished. The pool creates no pages itself: it says
/// when one is wanted (`wants_page`), is told when one exists (`add_page`), and offers
/// empty ones back (`take_empty_page`).
#[derive(Debug)]
pub struct RangePool {
    config: PoolConfig,
    /// The pages by index. A released page leaves a hole that the next added page fills.
    pages: Vec<Option<Page>>,
    /// The ranges. Slots are reused, but only after `reclaim` has freed their range.
    records: Vec<Record>,
    /// The first spare slot of `records`.
    spare: u32,
    /// The first retiring record.
    retiring: u32,
    /// The counters, kept up to date by every operation (all but `largest_free`, which
    /// `stats` works out when asked).
    stats: PoolStats,
    /// An allocation failed with `Growing` and nothing has been freed or added since.
    grow_wanted: bool,
    /// A page is being created elsewhere.
    page_pending: bool,
}

impl RangePool {
    /// A pool with no pages yet. Panics on a configuration that cannot work.
    pub fn new(config: PoolConfig) -> RangePool {
        assert!(
            config.granule.is_power_of_two(),
            "the granule ({}) must be a power of two",
            config.granule
        );
        assert!(
            config.page_bytes != 0 && config.page_bytes.is_multiple_of(config.granule),
            "the page size ({}) must be a non-zero multiple of the granule ({})",
            config.page_bytes,
            config.granule
        );
        assert!(
            config.budget_bytes >= config.page_bytes,
            "the budget ({}) must allow at least one page of {} bytes",
            config.budget_bytes,
            config.page_bytes
        );
        assert!(
            config.high_watermark >= config.low_watermark,
            "the high watermark ({}) must not be below the low watermark ({})",
            config.high_watermark,
            config.low_watermark
        );
        RangePool {
            config,
            pages: Vec::new(),
            records: Vec::new(),
            spare: NIL,
            retiring: NIL,
            stats: PoolStats::default(),
            grow_wanted: false,
            page_pending: false,
        }
    }

    /// The configuration the pool was made with.
    pub fn config(&self) -> &PoolConfig {
        &self.config
    }

    /// A range of `size` bytes at an offset that is a multiple of `align`, on the first
    /// page (lowest index) that has room, so that the higher pages drain and can be released.
    /// Panics unless `align` is a non-zero multiple of the granule.
    pub fn alloc(&mut self, size: u64, align: u64) -> Result<RangeHandle, PoolError> {
        let rounded = match whole_granules(size, align, self.config.granule) {
            Some(rounded) if size != 0 && rounded <= self.config.page_bytes => rounded,
            _ => return Err(PoolError::TooLarge),
        };
        let placed = self.pages.iter_mut().enumerate().find_map(|(index, page)| {
            let page = page.as_mut()?;
            let block = page.ranges.alloc(size, align)?;
            // The page is in use again: its wait to be released starts over.
            page.idle_ticks = 0;
            // There are never more pages than a u32 can count; add_page sees to that.
            Some((index as u32, block))
        });
        let Some((page, block)) = placed else {
            self.stats.failures += 1;
            if self.stats.free_bytes >= rounded {
                self.stats.failures_with_space += 1;
            }
            if self.can_grow() {
                self.grow_wanted = true;
                return Err(PoolError::Growing);
            }
            return Err(PoolError::Full);
        };

        let mut record = Record {
            generation: 0,
            state: RecordState::Live,
            page,
            block,
            requested: size,
            users: Users::NONE,
            next: NIL,
        };
        let index = if self.spare == NIL {
            let index = slab_index(self.records.len());
            self.records.push(record);
            index
        } else {
            let index = self.spare;
            let vacant = &mut self.records[index as usize];
            self.spare = vacant.next;
            // The slot keeps the generation its last range was retired with, which no handle
            // given out before carries.
            record.generation = vacant.generation;
            *vacant = record;
            index
        };

        self.stats.live += 1;
        self.stats.free_bytes -= rounded;
        self.stats.payload_bytes += size;
        self.stats.alignment_waste += rounded - size;
        self.stats.peak_payload_bytes = self.stats.peak_payload_bytes.max(self.stats.payload_bytes);
        Ok(RangeHandle {
            index,
            generation: record.generation,
        })
    }

    /// Where the range of a handle is. None for a handle whose range was retired, whether or
    /// not it has been reclaimed and its slot used again since.
    pub fn resolve(&self, handle: RangeHandle) -> Option<PooledRange> {
        let record = self.live_record(handle)?;
        Some(PooledRange {
            page: record.page,
            offset: record.block.offset,
            size: record.requested,
            generation: record.generation,
        })
    }

    /// Ends the life of a range: its handle stops resolving, but its bytes stay allocated
    /// until `reclaim` is told that all of `users` have finished. Returns false, and does
    /// nothing, for a handle that does not resolve.
    pub fn retire(&mut self, handle: RangeHandle, users: Users) -> bool {
        if self.live_record(handle).is_none() {
            return false;
        }
        let record = &mut self.records[handle.index as usize];
        record.state = RecordState::Retiring;
        record.generation += 1;
        record.users = users;
        record.next = self.retiring;
        self.retiring = handle.index;

        let rounded = record.block.size;
        self.stats.live -= 1;
        self.stats.retiring += 1;
        self.stats.payload_bytes -= record.requested;
        self.stats.alignment_waste -= rounded - record.requested;
        self.stats.retiring_bytes += rounded;
        true
    }

    /// Frees every retired range whose users have all finished and returns how many that
    /// was. Their bytes can be allocated again from here on, and not before. Each call also
    /// counts as one tick of the wait before an empty page may be released.
    pub fn reclaim(&mut self, completed: &Completed) -> u32 {
        let mut freed = 0;
        let mut before = NIL;
        let mut index = self.retiring;
        while index != NIL {
            let record = self.records[index as usize];
            if record.users.done(completed) {
                let page = self.pages[record.page as usize]
                    .as_mut()
                    .expect("a retiring range is on a page the pool still has");
                page.ranges.free(record.block);
                self.stats.free_bytes += record.block.size;
                self.stats.retiring_bytes -= record.block.size;
                self.stats.retiring -= 1;
                freed += 1;

                if before == NIL {
                    self.retiring = record.next;
                } else {
                    self.records[before as usize].next = record.next;
                }
                let slot = &mut self.records[index as usize];
                slot.state = RecordState::Vacant;
                // A slot that has run out of generations is left out of use for good: used
                // once more, it would have to repeat a generation an old handle may carry.
                if slot.generation != u32::MAX {
                    slot.next = self.spare;
                    self.spare = index;
                }
            } else {
                before = index;
            }
            index = record.next;
        }
        if freed > 0 {
            // Whatever failed for want of a page may fit now: let it try again first.
            self.grow_wanted = false;
        }
        for page in self.pages.iter_mut().flatten() {
            page.idle_ticks = if page.ranges.is_empty() {
                page.idle_ticks.saturating_add(1)
            } else {
                0
            };
        }
        freed
    }

    /// Whether another page should be created now: the free bytes are down to the low
    /// watermark, or an allocation failed with `Growing` and nothing has been freed or added
    /// since (so it would fail again); and the budget allows one; and none is on its way.
    /// A later allocation that succeeds does not withdraw the ask: the one that failed still
    /// has nowhere to go.
    pub fn wants_page(&self) -> bool {
        !self.page_pending
            && self.can_grow()
            && (self.grow_wanted || self.stats.free_bytes <= self.config.low_watermark)
    }

    /// A page is being created elsewhere: `wants_page` is false until `add_page` or
    /// `page_failed`, so that one need is not answered with two pages.
    pub fn page_requested(&mut self) {
        self.page_pending = true;
    }

    /// A page now exists. Returns its index, which is the lowest one no page has (so the
    /// indices of released pages are used again). Panics if it would exceed the budget.
    pub fn add_page(&mut self) -> u32 {
        assert!(
            self.can_grow(),
            "another page of {} bytes would exceed the budget of {} bytes",
            self.config.page_bytes,
            self.config.budget_bytes
        );
        let page = Some(Page {
            ranges: RangeAllocator::new(self.config.page_bytes, self.config.granule),
            idle_ticks: 0,
        });
        let index = match self.pages.iter().position(Option::is_none) {
            Some(hole) => {
                self.pages[hole] = page;
                hole as u32
            }
            None => {
                let index = slab_index(self.pages.len());
                self.pages.push(page);
                index
            }
        };
        self.page_pending = false;
        self.grow_wanted = false;
        self.stats.pages += 1;
        self.stats.pages_added += 1;
        self.stats.reserved_bytes += self.config.page_bytes;
        self.stats.free_bytes += self.config.page_bytes;
        self.stats.peak_reserved_bytes = self
            .stats
            .peak_reserved_bytes
            .max(self.stats.reserved_bytes);
        index
    }

    /// The page that was being created could not be: `wants_page` answers for itself again.
    pub fn page_failed(&mut self) {
        self.page_pending = false;
    }

    /// Removes one page the pool can do without and returns its index, for the caller to
    /// destroy: a page with no live or retiring range at the end of at least `release_after`
    /// `reclaim` calls in a row, the highest such index first, and only if the free bytes left
    /// without it stay above the high watermark. The waiting and the margin keep a pool
    /// whose load swings from destroying a page it is about to ask for again.
    pub fn take_empty_page(&mut self) -> Option<u32> {
        let left = self.stats.free_bytes.checked_sub(self.config.page_bytes)?;
        if left <= self.config.high_watermark {
            return None;
        }
        let index = self.pages.iter().rposition(|page| {
            page.as_ref().is_some_and(|page| {
                page.ranges.is_empty() && page.idle_ticks >= self.config.release_after
            })
        })?;
        self.pages[index] = None;
        self.stats.pages -= 1;
        self.stats.pages_released += 1;
        self.stats.reserved_bytes -= self.config.page_bytes;
        self.stats.free_bytes = left;
        Some(index as u32)
    }

    /// How many pages the pool has.
    pub fn page_count(&self) -> u32 {
        self.stats.pages
    }

    /// Whether the pool has a page with this index.
    pub fn has_page(&self, index: u32) -> bool {
        self.pages.get(index as usize).is_some_and(Option::is_some)
    }

    /// The counters as they stand.
    pub fn stats(&self) -> PoolStats {
        let largest_free = self
            .pages
            .iter()
            .flatten()
            .map(|page| page.ranges.largest_free())
            .max();
        PoolStats {
            largest_free: largest_free.unwrap_or(0),
            ..self.stats
        }
    }

    /// Whether the budget allows one more page.
    fn can_grow(&self) -> bool {
        self.stats
            .reserved_bytes
            .checked_add(self.config.page_bytes)
            .is_some_and(|reserved| reserved <= self.config.budget_bytes)
    }

    /// The record a handle names, if its range is still live.
    fn live_record(&self, handle: RangeHandle) -> Option<&Record> {
        self.records.get(handle.index as usize).filter(|record| {
            record.state == RecordState::Live && record.generation == handle.generation
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A xorshift64 generator: the same trace on every run and every machine.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        /// A number below `bound`.
        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }
    }

    /// Whether the free interval `start..end` can hold `need` bytes at a multiple of `align`.
    fn holds(start: u64, end: u64, need: u64, align: u64) -> bool {
        start.next_multiple_of(align) + need <= end
    }

    /// The free intervals of a page whose granules are marked used or not.
    fn free_runs(used: &[bool], granule: u64) -> Vec<(u64, u64)> {
        let mut runs = Vec::new();
        let mut start = None;
        for (cell, &taken) in used.iter().enumerate() {
            let offset = cell as u64 * granule;
            match (taken, start) {
                (false, None) => start = Some(offset),
                (true, Some(from)) => {
                    runs.push((from, offset));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(from) = start {
            runs.push((from, used.len() as u64 * granule));
        }
        runs
    }

    #[test]
    fn size_classes_are_ordered_and_exact_where_it_matters() {
        let mut last = 0;
        for granules in 1..5000u64 {
            let floor = class_floor(granules);
            let ceil = class_ceil(granules).unwrap();
            assert!(floor >= last, "a larger size never goes into a lower bin");
            assert!(ceil == floor || ceil == floor + 1);
            // The bin above the one a size falls in holds only larger sizes, and the first
            // size of a bin is the bin's own answer both ways.
            assert_eq!(
                ceil == floor,
                granules == 1 || class_floor(granules - 1) < floor
            );
            last = floor;
        }
        assert_eq!(class_floor(7), 7);
        assert_eq!(class_floor(8), 8);
        assert_eq!(class_floor(16), 16);
        assert_eq!(class_floor(17), 16);
        assert_eq!(class_floor(18), 17);
        assert_eq!(class_ceil(17), Some(17));
        assert_eq!(class_ceil(u64::MAX), None);
        assert_eq!(class_floor(u64::MAX), 495);
    }

    #[test]
    fn a_new_allocator_is_one_free_block() {
        let ranges = RangeAllocator::new(4096, 4);
        ranges.check();
        assert_eq!(ranges.size(), 4096);
        assert_eq!(ranges.free_bytes(), 4096);
        assert_eq!(ranges.largest_free(), 4096);
        assert!(ranges.is_empty());
        assert_eq!(ranges.allocated_blocks(), 0);
        assert!(ranges.fits(4096, 4));
        assert!(!ranges.fits(4097, 4));
    }

    #[test]
    #[should_panic(expected = "power of two")]
    fn the_granule_must_be_a_power_of_two() {
        RangeAllocator::new(1200, 12);
    }

    #[test]
    #[should_panic(expected = "multiple of the granule")]
    fn the_size_must_be_whole_granules() {
        RangeAllocator::new(1022, 4);
    }

    #[test]
    #[should_panic(expected = "multiple of the granule")]
    fn the_alignment_must_be_whole_granules() {
        RangeAllocator::new(1024, 4).alloc(16, 6);
    }

    #[test]
    fn an_allocation_splits_its_block_and_loses_nothing_to_alignment() {
        let mut ranges = RangeAllocator::new(1024, 4);
        let a = ranges.alloc(10, 4).unwrap();
        assert_eq!(
            (a.offset, a.size),
            (0, 12),
            "the size is rounded up to granules"
        );
        let b = ranges.alloc(40, 20).unwrap();
        assert_eq!((b.offset, b.size), (20, 40));
        ranges.check();
        // The eight bytes skipped to reach offset 20 are still free, as is all after 60.
        assert_eq!(ranges.free_bytes(), 1024 - 12 - 40);
        assert_eq!(ranges.largest_free(), 1024 - 60);
        assert_eq!(ranges.allocated_blocks(), 2);
        assert!(!ranges.is_empty());
        let c = ranges.alloc(8, 4).unwrap();
        assert_eq!(
            c.offset, 12,
            "the skipped bytes are handed out like any others"
        );
        let d = ranges.alloc(964, 4).unwrap();
        assert_eq!(d.offset, 60);
        ranges.check();
        assert_eq!(ranges.free_bytes(), 0);
        assert_eq!(ranges.largest_free(), 0);
        assert_eq!(ranges.alloc(4, 4), None);
    }

    #[test]
    fn a_freed_block_joins_free_neighbours_on_both_sides() {
        let mut ranges = RangeAllocator::new(400, 4);
        let blocks: Vec<Block> = (0..4).map(|_| ranges.alloc(100, 4).unwrap()).collect();
        assert_eq!(
            blocks.iter().map(|b| b.offset).collect::<Vec<_>>(),
            [0, 100, 200, 300]
        );
        ranges.free(blocks[0]);
        ranges.free(blocks[2]);
        ranges.check();
        assert_eq!(ranges.free_bytes(), 200);
        assert_eq!(
            ranges.largest_free(),
            100,
            "the two free blocks do not touch"
        );
        assert!(!ranges.fits(104, 4));

        ranges.free(blocks[1]);
        ranges.check();
        assert_eq!(
            ranges.largest_free(),
            300,
            "one block where there were three"
        );
        let whole = ranges.alloc(300, 4).unwrap();
        assert_eq!(whole.offset, 0);
        ranges.free(whole);
        ranges.free(blocks[3]);
        ranges.check();
        assert!(ranges.is_empty());
        assert_eq!(ranges.largest_free(), 400);
    }

    #[test]
    fn slots_of_freed_blocks_are_used_again() {
        let mut ranges = RangeAllocator::new(4096, 4);
        let mut most = 0;
        for round in 0..50 {
            let blocks: Vec<Block> = (0..16).map(|_| ranges.alloc(200, 20).unwrap()).collect();
            for block in blocks
                .iter()
                .step_by(2)
                .chain(blocks.iter().skip(1).step_by(2))
            {
                ranges.free(*block);
            }
            ranges.check();
            assert!(ranges.is_empty());
            if round == 0 {
                most = ranges.nodes.len();
            }
            assert_eq!(
                ranges.nodes.len(),
                most,
                "the slab stops growing after one round"
            );
        }
    }

    #[test]
    fn an_alignment_need_not_be_a_power_of_two() {
        let mut ranges = RangeAllocator::new(4096, 4);
        let mut blocks = Vec::new();
        for size in [36, 4, 100, 20, 8, 60] {
            // Something small in between, so that the next start is not aligned by chance.
            ranges.alloc(4, 4).unwrap();
            let block = ranges.alloc(size, 20).unwrap();
            assert_eq!(block.offset % 20, 0);
            assert_eq!(block.size, size);
            blocks.push(block);
        }
        ranges.check();
        assert!(blocks.iter().any(|block| block.offset % 16 != 0));
    }

    #[test]
    fn the_thorough_search_finds_a_fit_the_quick_one_misses() {
        // The only free interval is 40..80. Forty bytes aligned to 20 fit it exactly, but
        // only because it starts on a multiple of 20: a block that fits wherever it starts
        // would need 56 bytes, and there is none.
        let mut ranges = RangeAllocator::new(240, 4);
        ranges.alloc(40, 4).unwrap();
        let hole = ranges.alloc(40, 4).unwrap();
        ranges.alloc(160, 4).unwrap();
        ranges.free(hole);
        assert!(ranges.fits(40, 20));
        let block = ranges.alloc(40, 20).unwrap();
        assert_eq!((block.offset, block.size), (40, 40));
        ranges.check();
        assert_eq!(ranges.free_bytes(), 0);

        // The same sizes with the interval at 44..84: no multiple of 20 has 40 bytes after it
        // in there, so this time there is truly no room, though 40 bytes are free.
        let mut ranges = RangeAllocator::new(240, 4);
        ranges.alloc(44, 4).unwrap();
        let hole = ranges.alloc(40, 4).unwrap();
        ranges.alloc(156, 4).unwrap();
        ranges.free(hole);
        assert_eq!(ranges.free_bytes(), 40);
        assert!(!ranges.fits(40, 20));
        assert_eq!(ranges.alloc(40, 20), None);
        assert!(ranges.fits(40, 4));
        // 24 bytes do fit, at 60, and the 16 bytes before them stay free.
        let block = ranges.alloc(24, 20).unwrap();
        assert_eq!(block.offset, 60);
        ranges.check();
        assert_eq!(ranges.largest_free(), 16);
        assert_eq!(ranges.alloc(16, 4).unwrap().offset, 44);
    }

    #[test]
    #[should_panic(expected = "not allocated")]
    fn freeing_a_block_twice_panics() {
        let mut ranges = RangeAllocator::new(1024, 4);
        let block = ranges.alloc(64, 4).unwrap();
        ranges.alloc(64, 4).unwrap();
        ranges.free(block);
        ranges.free(block);
    }

    #[test]
    #[should_panic(expected = "not allocated")]
    fn freeing_a_block_with_the_wrong_offset_panics() {
        let mut ranges = RangeAllocator::new(1024, 4);
        let block = ranges.alloc(64, 4).unwrap();
        ranges.free(Block {
            offset: block.offset + 4,
            ..block
        });
    }

    #[test]
    fn alloc_fails_only_when_no_free_interval_fits() {
        const SIZE: u64 = 2048;
        const GRANULE: u64 = 4;
        let mut ranges = RangeAllocator::new(SIZE, GRANULE);
        let mut used = vec![false; (SIZE / GRANULE) as usize];
        let mut blocks: Vec<Block> = Vec::new();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut refused, mut refused_with_space) = (0, 0);
        for _ in 0..8000 {
            if rng.below(100) < 55 {
                let size = 1 + rng.below(160);
                let align = [4, 8, 12, 20, 64, 100][rng.below(6) as usize];
                let need = size.next_multiple_of(GRANULE);
                let runs = free_runs(&used, GRANULE);
                let room = runs
                    .iter()
                    .any(|&(start, end)| holds(start, end, need, align));
                assert_eq!(ranges.fits(size, align), room);
                match ranges.alloc(size, align) {
                    Some(block) => {
                        assert!(room);
                        assert_eq!(block.offset % align, 0);
                        assert_eq!(block.size, need);
                        let cells = (block.offset / GRANULE) as usize
                            ..((block.offset + need) / GRANULE) as usize;
                        for cell in &mut used[cells] {
                            assert!(!*cell, "handed out bytes that were taken");
                            *cell = true;
                        }
                        blocks.push(block);
                    }
                    None => {
                        assert!(!room, "refused {size} bytes aligned to {align} that fit");
                        refused += 1;
                        if runs.iter().any(|&(start, end)| end - start >= need) {
                            refused_with_space += 1;
                        }
                    }
                }
            } else if !blocks.is_empty() {
                let block = blocks.swap_remove(rng.below(blocks.len() as u64) as usize);
                ranges.free(block);
                let cells = (block.offset / GRANULE) as usize
                    ..((block.offset + block.size) / GRANULE) as usize;
                used[cells].fill(false);
            }
            ranges.check();
            let runs = free_runs(&used, GRANULE);
            assert_eq!(
                ranges.free_bytes(),
                runs.iter().map(|&(start, end)| end - start).sum::<u64>()
            );
            assert_eq!(
                ranges.largest_free(),
                runs.iter()
                    .map(|&(start, end)| end - start)
                    .max()
                    .unwrap_or(0)
            );
            assert_eq!(ranges.allocated_blocks() as usize, blocks.len());
        }
        assert!(
            refused > 100,
            "the page was rarely full: {refused} refusals"
        );
        assert!(
            refused_with_space > 10,
            "alignment alone rarely decided: {refused_with_space}"
        );
    }

    /// A small pool: pages of 1024 bytes, four of them at most.
    fn small() -> PoolConfig {
        PoolConfig {
            page_bytes: 1024,
            budget_bytes: 4096,
            low_watermark: 128,
            high_watermark: 256,
            release_after: 2,
            granule: 4,
        }
    }

    /// A pool with this many pages already.
    fn pool_with_pages(config: PoolConfig, pages: u32) -> RangePool {
        let mut pool = RangePool::new(config);
        for index in 0..pages {
            assert_eq!(pool.add_page(), index);
        }
        pool
    }

    /// Completion up to this frame and this copy batch.
    fn completed(frame: u64, copy_batch: u64) -> Completed {
        Completed {
            frame,
            frame_batch: 0,
            copy_batch,
        }
    }

    #[test]
    #[should_panic(expected = "high watermark")]
    fn the_watermarks_must_be_in_order() {
        RangePool::new(PoolConfig {
            low_watermark: 512,
            ..small()
        });
    }

    #[test]
    #[should_panic(expected = "at least one page")]
    fn the_budget_must_allow_a_page() {
        RangePool::new(PoolConfig {
            budget_bytes: 1000,
            ..small()
        });
    }

    #[test]
    fn a_range_resolves_to_where_it_was_put() {
        let mut pool = pool_with_pages(small(), 1);
        assert_eq!(pool.config(), &small());
        let a = pool.alloc(10, 4).unwrap();
        let b = pool.alloc(30, 20).unwrap();
        assert_eq!(
            pool.resolve(a),
            Some(PooledRange {
                page: 0,
                offset: 0,
                size: 10,
                generation: a.generation(),
            })
        );
        let range = pool.resolve(b).unwrap();
        assert_eq!((range.page, range.offset, range.size), (0, 20, 30));
        assert_ne!(a, b);
    }

    #[test]
    fn a_request_that_never_fits_a_page_is_too_large() {
        let mut pool = RangePool::new(small());
        assert_eq!(pool.alloc(0, 4), Err(PoolError::TooLarge));
        assert_eq!(pool.alloc(1025, 4), Err(PoolError::TooLarge));
        assert_eq!(
            pool.alloc(1021, 4),
            Err(PoolError::Growing),
            "1024 after rounding"
        );
        assert_eq!(pool.alloc(u64::MAX, 4), Err(PoolError::TooLarge));
        pool.add_page();
        assert_eq!(pool.alloc(1025, 4), Err(PoolError::TooLarge));
        assert!(pool.alloc(1024, 4).is_ok());
        assert_eq!(
            pool.stats().failures,
            1,
            "an impossible request is not a failure of the pool"
        );
    }

    #[test]
    fn the_pool_grows_only_within_its_budget() {
        let mut pool = RangePool::new(small());
        assert_eq!(pool.alloc(1024, 4), Err(PoolError::Growing), "no page yet");
        for pages in 1..=4 {
            assert!(pool.wants_page());
            assert_eq!(pool.add_page(), pages - 1);
            assert_eq!(pool.page_count(), pages);
            assert!(pool.alloc(1024, 4).is_ok());
            let expected = if pages < 4 {
                PoolError::Growing
            } else {
                PoolError::Full
            };
            assert_eq!(pool.alloc(4, 4), Err(expected));
        }
        assert!(
            !pool.wants_page(),
            "nothing is free, but the budget is spent"
        );
        let stats = pool.stats();
        assert_eq!((stats.pages, stats.reserved_bytes), (4, 4096));
        assert_eq!((stats.failures, stats.failures_with_space), (5, 0));
        assert_eq!((stats.pages_added, stats.peak_reserved_bytes), (4, 4096));
    }

    #[test]
    #[should_panic(expected = "exceed the budget")]
    fn adding_a_page_beyond_the_budget_panics() {
        let mut pool = pool_with_pages(small(), 4);
        pool.add_page();
    }

    #[test]
    fn allocations_go_to_the_lowest_page_with_room() {
        let mut pool = pool_with_pages(small(), 3);
        let a = pool.alloc(600, 4).unwrap();
        let b = pool.alloc(600, 4).unwrap();
        let c = pool.alloc(400, 4).unwrap();
        let d = pool.alloc(424, 4).unwrap();
        let page = |handle| pool.resolve(handle).unwrap().page;
        assert_eq!([page(a), page(b), page(c), page(d)], [0, 1, 0, 1]);
        // Fragmentation: 16 bytes are free on the only page, enough for 12, but they hold no
        // multiple of 20 to start at. That failure is counted as one that had the space.
        let mut pool = pool_with_pages(
            PoolConfig {
                budget_bytes: 1024,
                ..small()
            },
            1,
        );
        pool.alloc(4, 4).unwrap();
        let hole = pool.alloc(16, 4).unwrap();
        pool.alloc(1004, 4).unwrap();
        assert!(pool.retire(hole, Users::NONE));
        assert_eq!(pool.reclaim(&Completed::default()), 1);
        assert_eq!(pool.alloc(20, 4), Err(PoolError::Full));
        assert_eq!(
            pool.alloc(12, 20),
            Err(PoolError::Full),
            "4..20 holds no multiple of 20"
        );
        let stats = pool.stats();
        assert_eq!((stats.failures, stats.failures_with_space), (2, 1));
        assert_eq!((stats.free_bytes, stats.largest_free), (16, 16));
    }

    #[test]
    fn a_retired_handle_is_stale_for_good() {
        let mut pool = pool_with_pages(small(), 1);
        let old = pool.alloc(100, 4).unwrap();
        assert!(pool.retire(old, Users::until_frame(1)));
        assert_eq!(
            pool.resolve(old),
            None,
            "retired, though its bytes are still held"
        );
        assert!(!pool.retire(old, Users::NONE), "it cannot be retired twice");
        assert_eq!(pool.reclaim(&completed(1, 0)), 1);
        assert_eq!(pool.resolve(old), None);

        // The next range takes the same slot and the same bytes, under another generation.
        let new = pool.alloc(100, 4).unwrap();
        assert_eq!(new.index, old.index);
        assert_ne!(new.generation(), old.generation());
        assert_eq!(
            pool.resolve(old),
            None,
            "the old handle never names the new range"
        );
        assert!(!pool.retire(old, Users::NONE));
        let range = pool.resolve(new).unwrap();
        assert_eq!(
            (range.offset, range.size, range.generation),
            (0, 100, new.generation())
        );
        let unknown = RangeHandle {
            index: 77,
            generation: 0,
        };
        assert_eq!(pool.resolve(unknown), None);
        assert!(!pool.retire(unknown, Users::NONE));
    }

    #[test]
    fn a_slot_out_of_generations_is_not_used_again() {
        let mut pool = pool_with_pages(small(), 1);
        let first = pool.alloc(8, 4).unwrap();
        // As if the slot had been through all but one of its generations already.
        pool.records[first.index as usize].generation = u32::MAX - 1;
        let last = RangeHandle {
            index: first.index,
            generation: u32::MAX - 1,
        };
        assert_eq!(pool.resolve(first), None);
        assert!(pool.resolve(last).is_some());
        assert!(pool.retire(last, Users::NONE));
        assert_eq!(pool.reclaim(&Completed::default()), 1);
        let next = pool.alloc(8, 4).unwrap();
        assert_ne!(next.index, first.index);
        assert_eq!(pool.resolve(last), None);
        assert_eq!(pool.resolve(first), None);
        assert_eq!(
            pool.resolve(next).unwrap().offset,
            0,
            "the bytes themselves are reused"
        );
    }

    #[test]
    fn a_retired_range_keeps_its_bytes_until_frame_and_copy_are_both_done() {
        let mut pool = pool_with_pages(
            PoolConfig {
                budget_bytes: 1024,
                ..small()
            },
            1,
        );
        let range = pool.alloc(1024, 4).unwrap();
        let users = Users::until_frame(5).and(Users {
            copy_batch: 3,
            ..Users::NONE
        });
        assert!(pool.retire(range, users));
        assert_eq!(
            pool.alloc(4, 4),
            Err(PoolError::Full),
            "retired is not free"
        );
        assert_eq!(
            pool.reclaim(&completed(5, 2)),
            0,
            "the copy has not finished"
        );
        assert_eq!(pool.alloc(4, 4), Err(PoolError::Full));
        assert_eq!(pool.reclaim(&completed(4, 3)), 0, "a frame still reads it");
        assert_eq!(pool.alloc(4, 4), Err(PoolError::Full));
        let stats = pool.stats();
        assert_eq!(
            (stats.live, stats.retiring, stats.retiring_bytes),
            (0, 1, 1024)
        );
        assert_eq!(pool.reclaim(&completed(5, 3)), 1);
        assert_eq!(pool.reclaim(&completed(9, 9)), 0, "it is freed once only");
        let again = pool.alloc(1024, 4).unwrap();
        assert_eq!(pool.resolve(again).unwrap().offset, 0);
    }

    #[test]
    fn the_low_watermark_asks_for_a_page_before_allocations_fail() {
        let mut pool = pool_with_pages(small(), 1);
        pool.alloc(800, 4).unwrap();
        assert!(!pool.wants_page(), "224 bytes free is above the watermark");
        pool.alloc(96, 4).unwrap();
        assert!(pool.wants_page(), "128 bytes free is at the watermark");
        assert!(pool.alloc(100, 4).is_ok(), "and yet there was room");
        assert_eq!(pool.stats().failures, 0);
        pool.add_page();
        assert!(!pool.wants_page());
    }

    #[test]
    fn a_failed_allocation_asks_for_a_page_until_something_changes() {
        let mut pool = pool_with_pages(small(), 1);
        let first = pool.alloc(600, 4).unwrap();
        assert!(!pool.wants_page());
        assert_eq!(pool.alloc(600, 4), Err(PoolError::Growing));
        assert!(
            pool.wants_page(),
            "424 bytes are free, but the failed request needs a page"
        );
        pool.alloc(100, 4).unwrap();
        assert!(
            pool.wants_page(),
            "a smaller success does not help the one that failed"
        );

        // Freeing something may make room: the ask is withdrawn until it fails again.
        assert!(pool.retire(first, Users::NONE));
        assert_eq!(pool.reclaim(&Completed::default()), 1);
        assert!(!pool.wants_page());
        pool.alloc(600, 4).unwrap();
        assert_eq!(pool.alloc(600, 4), Err(PoolError::Growing));
        assert!(pool.wants_page());
        pool.add_page();
        assert!(!pool.wants_page());
        assert!(pool.alloc(600, 4).is_ok());
    }

    #[test]
    fn a_requested_page_suppresses_the_ask() {
        let mut pool = RangePool::new(small());
        assert!(pool.wants_page(), "an empty pool has nothing free");
        pool.page_requested();
        assert!(!pool.wants_page(), "one is on its way");
        assert_eq!(pool.alloc(8, 4), Err(PoolError::Growing));
        assert!(!pool.wants_page(), "still only one page is asked for");
        pool.page_failed();
        assert!(pool.wants_page(), "it never came");
        pool.page_requested();
        assert_eq!(pool.add_page(), 0);
        assert!(!pool.wants_page(), "it came, and it is enough");
        pool.alloc(1000, 4).unwrap();
        assert!(pool.wants_page(), "the next one is asked for afresh");
    }

    #[test]
    fn an_empty_page_is_released_only_after_its_wait_and_above_the_high_watermark() {
        let config = PoolConfig {
            high_watermark: 1024,
            ..small()
        };
        let mut pool = pool_with_pages(config, 3);
        let done = Completed::default();
        assert_eq!(
            pool.take_empty_page(),
            None,
            "empty, but for no time at all"
        );
        pool.reclaim(&done);
        assert_eq!(pool.take_empty_page(), None, "one tick of the two");
        pool.reclaim(&done);
        assert_eq!(
            pool.take_empty_page(),
            Some(2),
            "the highest index goes first"
        );
        assert!(!pool.has_page(2));
        assert_eq!(pool.page_count(), 2);
        // Two empty pages are left, both waited long enough, but without one of them the
        // free bytes would be at the high watermark, not above it.
        pool.reclaim(&done);
        assert_eq!(pool.take_empty_page(), None);
        assert_eq!(pool.page_count(), 2);
        let stats = pool.stats();
        assert_eq!((stats.pages_added, stats.pages_released), (3, 1));
        assert_eq!((stats.reserved_bytes, stats.free_bytes), (2048, 2048));
        assert_eq!(stats.peak_reserved_bytes, 3072);
    }

    #[test]
    fn an_allocation_restarts_the_wait_of_its_page() {
        let mut pool = pool_with_pages(small(), 2);
        let done = Completed::default();
        pool.alloc(512, 4).unwrap();
        let on_second = pool.alloc(1024, 4).unwrap();
        pool.reclaim(&done);
        pool.reclaim(&done);
        assert_eq!(pool.take_empty_page(), None, "neither page is empty");

        assert!(pool.retire(on_second, Users::until_frame(1)));
        pool.reclaim(&done);
        pool.reclaim(&done);
        assert_eq!(
            pool.take_empty_page(),
            None,
            "a retiring range still occupies the page"
        );
        assert_eq!(pool.reclaim(&completed(1, 0)), 1);
        assert_eq!(pool.take_empty_page(), None, "empty for one tick");

        // Used and emptied again between two ticks: the wait starts over all the same.
        let again = pool.alloc(1024, 4).unwrap();
        assert_eq!(pool.resolve(again).unwrap().page, 1);
        assert!(pool.retire(again, Users::NONE));
        assert_eq!(pool.reclaim(&completed(1, 0)), 1);
        assert_eq!(pool.take_empty_page(), None, "one tick again, not two");
        pool.reclaim(&done);
        assert_eq!(pool.take_empty_page(), Some(1));
        assert_eq!(pool.take_empty_page(), None, "page 0 is in use");
    }

    #[test]
    fn a_released_index_is_reused_and_no_range_moves() {
        let mut pool = pool_with_pages(small(), 3);
        let a = pool.alloc(1024, 4).unwrap();
        let b = pool.alloc(1024, 4).unwrap();
        let c = pool.alloc(400, 4).unwrap();
        let (range_a, range_c) = (pool.resolve(a), pool.resolve(c));
        assert_eq!(range_c.unwrap().page, 2);
        assert!(pool.retire(b, Users::NONE));
        pool.reclaim(&Completed::default());
        pool.reclaim(&Completed::default());
        assert_eq!(
            pool.take_empty_page(),
            Some(1),
            "the page in the middle is the empty one"
        );
        assert!(pool.has_page(0) && !pool.has_page(1) && pool.has_page(2));
        assert!(!pool.has_page(3));
        assert_eq!((pool.resolve(a), pool.resolve(c)), (range_a, range_c));
        assert_eq!(pool.add_page(), 1);
        assert!(pool.has_page(1));
        let d = pool.alloc(1024, 4).unwrap();
        assert_eq!(pool.resolve(d).unwrap().page, 1);
        assert_eq!((pool.resolve(a), pool.resolve(c)), (range_a, range_c));
    }

    #[test]
    fn the_stats_follow_a_range_through_its_life() {
        let mut pool = pool_with_pages(small(), 1);
        let range = pool.alloc(10, 4).unwrap();
        pool.alloc(100, 20).unwrap();
        let live = pool.stats();
        assert_eq!((live.live, live.retiring), (2, 0));
        assert_eq!((live.payload_bytes, live.alignment_waste), (110, 2));
        assert_eq!((live.free_bytes, live.largest_free), (912, 904));
        assert!(pool.retire(range, Users::until_frame(2)));
        let retired = pool.stats();
        assert_eq!((retired.live, retired.retiring), (1, 1));
        assert_eq!((retired.payload_bytes, retired.alignment_waste), (100, 0));
        assert_eq!((retired.retiring_bytes, retired.free_bytes), (12, 912));
        assert_eq!(pool.reclaim(&completed(2, 0)), 1);
        let reclaimed = pool.stats();
        assert_eq!((reclaimed.retiring, reclaimed.retiring_bytes), (0, 0));
        assert_eq!((reclaimed.free_bytes, reclaimed.largest_free), (924, 904));
        assert_eq!(
            (reclaimed.peak_payload_bytes, reclaimed.peak_reserved_bytes),
            (110, 1024)
        );
    }

    /// What the churn test believes about one range that holds bytes.
    #[derive(Clone, Copy, Debug)]
    struct Held {
        handle: RangeHandle,
        offset: u64,
        rounded: u64,
        requested: u64,
        align: u64,
        /// Set once the range is retired: who must finish before its bytes are free.
        users: Option<Users>,
    }

    /// What the churn test believes about one page.
    #[derive(Debug, Default)]
    struct ModelPage {
        /// The ranges that hold bytes, live and retiring, in offset order.
        held: Vec<Held>,
        /// How many reclaims in a row have left the page empty.
        idle: u32,
    }

    impl ModelPage {
        /// The free intervals: whatever the held ranges leave of the page.
        fn gaps(&self, page_bytes: u64) -> Vec<(u64, u64)> {
            let mut gaps = Vec::new();
            let mut cursor = 0;
            for held in &self.held {
                if held.offset > cursor {
                    gaps.push((cursor, held.offset));
                }
                cursor = held.offset + held.rounded;
            }
            if cursor < page_bytes {
                gaps.push((cursor, page_bytes));
            }
            gaps
        }

        fn fits(&self, page_bytes: u64, need: u64, align: u64) -> bool {
            self.gaps(page_bytes)
                .iter()
                .any(|&(start, end)| holds(start, end, need, align))
        }
    }

    /// The churn test's own account of the pool, kept the slow and obvious way.
    #[derive(Debug)]
    struct Model {
        config: PoolConfig,
        pages: Vec<Option<ModelPage>>,
        /// Handles of ranges that were reclaimed: none may ever resolve again.
        dead: Vec<RangeHandle>,
        /// The counters the pool must agree with. The bytes and counts of ranges and pages
        /// are worked out afresh at every comparison; the rest is counted here as it happens.
        stats: PoolStats,
        /// Times a reclaim left a range alone whose frame was done but whose copy was not.
        waited_for_copy: u32,
    }

    impl Model {
        fn page_count(&self) -> u64 {
            self.pages.iter().flatten().count() as u64
        }

        fn free_bytes(&self) -> u64 {
            let held = self.pages.iter().flatten().flat_map(|page| &page.held);
            self.page_count() * self.config.page_bytes - held.map(|held| held.rounded).sum::<u64>()
        }

        fn fits_anywhere(&self, need: u64, align: u64) -> bool {
            let mut pages = self.pages.iter().flatten();
            pages.any(|page| page.fits(self.config.page_bytes, need, align))
        }

        /// A page the pool says it added under this index.
        fn add_page(&mut self, index: u32) {
            let lowest_hole = self.pages.iter().position(Option::is_none);
            assert_eq!(index as usize, lowest_hole.unwrap_or(self.pages.len()));
            if index as usize == self.pages.len() {
                self.pages.push(None);
            }
            self.pages[index as usize] = Some(ModelPage::default());
            self.stats.pages_added += 1;
        }

        /// A range the pool has just handed out: it must lie in free bytes of the first
        /// page that had room.
        fn record(&mut self, pool: &RangePool, handle: RangeHandle, requested: u64, align: u64) {
            let range = pool.resolve(handle).expect("a fresh handle resolves");
            let page_bytes = self.config.page_bytes;
            let rounded = requested.next_multiple_of(self.config.granule);
            assert_eq!(
                (range.size, range.generation),
                (requested, handle.generation())
            );
            assert_eq!(range.offset % align, 0);
            for lower in self.pages[..range.page as usize].iter().flatten() {
                assert!(
                    !lower.fits(page_bytes, rounded, align),
                    "a lower page had room"
                );
            }
            let page = self.pages[range.page as usize]
                .as_mut()
                .expect("the page exists");
            let end = range.offset + rounded;
            assert!(
                page.gaps(page_bytes)
                    .iter()
                    .any(|&(from, to)| from <= range.offset && end <= to),
                "{range:?} is not within free bytes"
            );
            let at = page.held.partition_point(|held| held.offset < range.offset);
            let held = Held {
                handle,
                offset: range.offset,
                rounded,
                requested,
                align,
                users: None,
            };
            page.held.insert(at, held);
            page.idle = 0;
        }

        /// A failure the pool has just reported: there must truly be no room.
        fn failed(&mut self, error: PoolError, requested: u64, align: u64) {
            let rounded = requested.next_multiple_of(self.config.granule);
            assert!(
                !self.fits_anywhere(rounded, align),
                "{requested} aligned to {align} fits"
            );
            let grown = (self.page_count() + 1) * self.config.page_bytes;
            let expected = if grown <= self.config.budget_bytes {
                PoolError::Growing
            } else {
                PoolError::Full
            };
            assert_eq!(error, expected);
            self.stats.failures += 1;
            if self.free_bytes() >= rounded {
                self.stats.failures_with_space += 1;
            }
        }

        /// The live range number `pick`, counted over all pages.
        fn live_mut(&mut self, pick: u64) -> &mut Held {
            let held = self
                .pages
                .iter_mut()
                .flatten()
                .flat_map(|page| &mut page.held);
            held.filter(|held| held.users.is_none())
                .nth(pick as usize)
                .expect("that many are live")
        }

        /// What a reclaim must do: drop the retired ranges whose users are done, and count
        /// a tick for every page that is empty afterwards. Returns how many it dropped.
        fn reclaim(&mut self, completed: &Completed) -> u32 {
            let mut freed = 0;
            for page in self.pages.iter_mut().flatten() {
                page.held.retain(|held| match held.users {
                    Some(users) if users.done(completed) => {
                        self.dead.push(held.handle);
                        freed += 1;
                        false
                    }
                    Some(users) => {
                        if users.frame <= completed.frame {
                            self.waited_for_copy += 1;
                        }
                        true
                    }
                    None => true,
                });
                page.idle = if page.held.is_empty() {
                    page.idle + 1
                } else {
                    0
                };
            }
            freed
        }

        /// What take_empty_page must return, and the page is gone from the model too.
        fn take_empty_page(&mut self) -> Option<u32> {
            let left = self.free_bytes().checked_sub(self.config.page_bytes)?;
            if left <= self.config.high_watermark {
                return None;
            }
            let after = self.config.release_after;
            let ready = |page: &ModelPage| page.held.is_empty() && page.idle >= after;
            let index = self
                .pages
                .iter()
                .rposition(|page| page.as_ref().is_some_and(ready))?;
            self.pages[index] = None;
            self.stats.pages_released += 1;
            Some(index as u32)
        }

        /// Compares the pool with the model, range by range and counter by counter.
        /// `all_dead` also tries every handle ever reclaimed, not only the latest.
        fn verify(&mut self, pool: &RangePool, all_dead: bool) {
            let PoolConfig {
                page_bytes,
                budget_bytes,
                granule,
                ..
            } = self.config;
            let mut stats = self.stats;
            for index in 0..self.pages.len().max(pool.pages.len()) {
                let page = self.pages.get(index).and_then(Option::as_ref);
                assert_eq!(pool.has_page(index as u32), page.is_some());
                let Some(page) = page else { continue };
                let mut cursor = 0;
                for held in &page.held {
                    assert!(
                        held.offset >= cursor,
                        "{held:?} overlaps the range before it"
                    );
                    assert_eq!(held.offset % held.align, 0);
                    assert_eq!(held.offset % granule, 0);
                    cursor = held.offset + held.rounded;
                    assert!(cursor <= page_bytes, "{held:?} runs off its page");
                    let range = PooledRange {
                        page: index as u32,
                        offset: held.offset,
                        size: held.requested,
                        generation: held.handle.generation(),
                    };
                    if held.users.is_none() {
                        assert_eq!(pool.resolve(held.handle), Some(range));
                        stats.live += 1;
                        stats.payload_bytes += held.requested;
                        stats.alignment_waste += held.rounded - held.requested;
                    } else {
                        assert_eq!(pool.resolve(held.handle), None, "a retired range resolves");
                        stats.retiring += 1;
                        stats.retiring_bytes += held.rounded;
                    }
                }
                let gaps = page.gaps(page_bytes);
                let free: u64 = gaps.iter().map(|&(start, end)| end - start).sum();
                let largest = gaps
                    .iter()
                    .map(|&(start, end)| end - start)
                    .max()
                    .unwrap_or(0);
                let ranges = &pool.pages[index].as_ref().unwrap().ranges;
                ranges.check();
                assert_eq!(
                    ranges.free_bytes(),
                    free,
                    "bytes of page {index} are held or not"
                );
                assert_eq!(ranges.largest_free(), largest);
                assert_eq!(ranges.allocated_blocks() as usize, page.held.len());
                stats.pages += 1;
                stats.reserved_bytes += page_bytes;
                stats.free_bytes += free;
                stats.largest_free = stats.largest_free.max(largest);
            }
            assert!(
                stats.reserved_bytes <= budget_bytes,
                "more pages than the budget allows"
            );
            // Every step changes the pages and the payload in one direction only, so the
            // largest values seen here are the largest there have been.
            self.stats.peak_reserved_bytes = stats.peak_reserved_bytes.max(stats.reserved_bytes);
            self.stats.peak_payload_bytes = stats.peak_payload_bytes.max(stats.payload_bytes);
            stats.peak_reserved_bytes = self.stats.peak_reserved_bytes;
            stats.peak_payload_bytes = self.stats.peak_payload_bytes;
            assert_eq!(pool.stats(), stats);
            assert_eq!(pool.page_count(), stats.pages);

            let recent = if all_dead {
                0
            } else {
                self.dead.len().saturating_sub(32)
            };
            for &handle in &self.dead[recent..] {
                assert_eq!(pool.resolve(handle), None, "a reclaimed handle resolves");
            }
        }
    }

    /// A mesh-like size: mostly 256 bytes to 64 KiB, now and then up to 512 KiB.
    fn mesh_size(rng: &mut Rng) -> u64 {
        const KIB: u64 = 1024;
        if rng.below(16) == 0 {
            64 * KIB + rng.below(448 * KIB + 1)
        } else {
            256 + rng.below(64 * KIB - 255)
        }
    }

    #[test]
    fn churn_keeps_every_invariant() {
        const MIB: u64 = 1024 * 1024;
        // Fifty-eight phases of the load's swing: the trace ends holding a full pool.
        const STEPS: u32 = 29_000;
        const PHASE: u32 = 500;
        let config = PoolConfig {
            page_bytes: MIB,
            budget_bytes: 8 * MIB,
            low_watermark: 256 * 1024,
            high_watermark: MIB + MIB / 2,
            release_after: 3,
            granule: 4,
        };
        let mut pool = RangePool::new(config);
        let mut model = Model {
            config,
            pages: Vec::new(),
            dead: Vec::new(),
            stats: PoolStats::default(),
            waited_for_copy: 0,
        };
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        let mut done = Completed::default();
        let mut page_on_its_way = false;
        let (mut grew_on_failure, mut grew_ahead, mut was_full) = (0, 0, 0);
        let (mut requests_failed, mut stale_retires) = (0, 0);

        for step in 0..STEPS {
            // A page asked for in the step before arrives, or fails to.
            if page_on_its_way {
                page_on_its_way = false;
                if rng.below(3) == 0 {
                    pool.page_failed();
                    requests_failed += 1;
                } else {
                    model.add_page(pool.add_page());
                }
            }

            // The load swings: it grows to the budget, holds, drains, and holds again.
            let (alloc_share, retire_share) = match step / PHASE % 4 {
                0 => (55, 25),
                2 => (22, 55),
                _ => (40, 40),
            };
            let live = pool.stats().live;
            let roll = rng.below(100);
            if roll < alloc_share {
                let size = mesh_size(&mut rng);
                let align = if rng.below(2) == 0 { 16 } else { 20 };
                match pool.alloc(size, align) {
                    Ok(handle) => model.record(&pool, handle, size, align),
                    Err(error) => {
                        model.failed(error, size, align);
                        if error == PoolError::Full {
                            was_full += 1;
                            assert!(!pool.wants_page());
                        } else {
                            grew_on_failure += 1;
                            assert!(pool.wants_page(), "a failure that can grow asks for a page");
                            // Half the time the page comes at once, and with it the request
                            // fits. Otherwise the ask stands until the end of the step.
                            if rng.below(2) == 0 {
                                model.add_page(pool.add_page());
                                let handle = pool.alloc(size, align).expect("a new page has room");
                                model.record(&pool, handle, size, align);
                            }
                        }
                    }
                }
            } else if roll < alloc_share + retire_share {
                // The frame that uses it last is a little ahead; for half of them a copy
                // batch as well, which completes later than that frame (copies advance at
                // half the pace of frames here).
                let mut users = Users::until_frame(done.frame + 1 + rng.below(3));
                if rng.below(2) == 0 {
                    users.copy_batch = done.copy_batch + 3 + rng.below(2);
                }
                if live > 0 {
                    let held = model.live_mut(rng.below(u64::from(live)));
                    held.users = Some(users);
                    let handle = held.handle;
                    assert!(pool.retire(handle, users));
                    assert!(!pool.retire(handle, users), "retired twice");
                }
                if !model.dead.is_empty() {
                    let dead = model.dead[rng.below(model.dead.len() as u64) as usize];
                    assert!(!pool.retire(dead, users), "a reclaimed handle was retired");
                    stale_retires += 1;
                }
            } else if roll < 95 {
                done.frame += 1;
                done.copy_batch = done.frame / 2;
                assert_eq!(pool.reclaim(&done), model.reclaim(&done));
            } else {
                loop {
                    let released = pool.take_empty_page();
                    assert_eq!(released, model.take_empty_page());
                    if released.is_none() {
                        break;
                    }
                }
            }

            if pool.wants_page() {
                if !pool.grow_wanted {
                    grew_ahead += 1;
                }
                if rng.below(2) == 0 {
                    pool.page_requested();
                    assert!(
                        !pool.wants_page(),
                        "a page on its way is not asked for again"
                    );
                    page_on_its_way = true;
                } else {
                    model.add_page(pool.add_page());
                }
            }
            model.verify(&pool, step % 512 == 0);
        }

        // The trace must have been through everything it is meant to test.
        let stats = pool.stats();
        assert!(grew_on_failure > 5, "{grew_on_failure} failures could grow");
        assert!(grew_ahead > 5, "the low watermark asked {grew_ahead} times");
        assert!(was_full > 100, "was full {was_full} times");
        assert!(
            stats.failures_with_space > 50,
            "fragmentation rarely decided"
        );
        assert!(
            stats.pages_released > 5,
            "released {} pages",
            stats.pages_released
        );
        assert!(
            requests_failed > 3,
            "{requests_failed} page requests failed"
        );
        assert!(stale_retires > 1000, "tried {stale_retires} stale handles");
        assert!(
            model.waited_for_copy > 1000,
            "copies rarely outlived their frame"
        );
        assert!(
            model.dead.len() > 4000,
            "reclaimed {} ranges",
            model.dead.len()
        );

        // Wind down: retire everything, let every user finish, and reclaim.
        if page_on_its_way {
            pool.page_failed();
        }
        for _ in 0..stats.live {
            let users = Users::until_frame(done.frame + 2).and(Users {
                copy_batch: done.copy_batch + 4,
                ..Users::NONE
            });
            let held = model.live_mut(0);
            held.users = Some(users);
            assert!(pool.retire(held.handle, users));
        }
        model.verify(&pool, false);
        assert!(
            pool.stats().retiring > 100,
            "the trace ended with the pool in use"
        );
        done.frame += 8;
        assert_eq!(pool.reclaim(&done), 0, "the copies are not done");
        done.copy_batch += 8;
        assert_eq!(pool.reclaim(&done), model.reclaim(&done));
        model.verify(&pool, true);
        for page in pool.pages.iter().flatten() {
            assert!(page.ranges.is_empty());
            assert_eq!(
                page.ranges.largest_free(),
                MIB,
                "the page is one free interval"
            );
        }

        // The empty pages go once they have waited, down to the fewest that keep more than
        // the high watermark free: with one and a half pages as the watermark, two.
        let before = pool.page_count();
        assert!(before > 2, "only {before} pages were left to release");
        assert_eq!(pool.take_empty_page(), model.take_empty_page());
        for _ in 0..config.release_after {
            assert_eq!(pool.reclaim(&done), model.reclaim(&done));
        }
        for released in 1..=before - 2 {
            let index = pool.take_empty_page();
            assert_eq!(index, model.take_empty_page());
            assert_eq!(
                index,
                Some(before - released),
                "the highest index goes first"
            );
        }
        assert_eq!(pool.take_empty_page(), None);
        assert_eq!(pool.page_count(), 2);
        model.verify(&pool, true);
        assert_eq!(pool.stats().free_bytes, 2 * MIB);
    }
}
