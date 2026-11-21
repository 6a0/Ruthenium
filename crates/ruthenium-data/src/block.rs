//! Version-neutral block-state layout.
//!
//! A block's states have consecutive ids starting at `first_state`. Within a
//! block, a state's offset is a mixed-radix number over the block's
//! properties in name order, the last property varying fastest; each digit
//! is the index of the state's value in [`Property::values`]. This is the
//! order vanilla numbers states in.

use core::ops::Range;

/// A block-state property and its possible values, in order.
#[derive(Debug)]
pub struct Property {
    pub name: &'static str,
    pub values: &'static [&'static str],
}

impl Property {
    /// The index of a value in [`Property::values`].
    pub fn index_of(&self, value: &str) -> Option<usize> {
        self.values.iter().position(|v| *v == value)
    }
}

/// A block's states.
#[derive(Debug)]
pub struct BlockInfo {
    pub first_state: u32,
    pub state_count: u32,
    pub default_state: u32,
    /// In name order (the order states are numbered in).
    pub properties: &'static [&'static Property],
}

impl BlockInfo {
    /// The block's state ids.
    pub const fn states(&self) -> Range<u32> {
        self.first_state..self.first_state + self.state_count
    }

    /// A property by name, with its position.
    pub fn property(&self, name: &str) -> Option<(usize, &'static Property)> {
        self.properties
            .iter()
            .enumerate()
            .find(|(_, p)| p.name == name)
            .map(|(i, p)| (i, *p))
    }

    /// How many states apart two values of property `index` are.
    fn stride(&self, index: usize) -> u32 {
        self.properties[index + 1..]
            .iter()
            .map(|p| p.values.len() as u32)
            .product()
    }

    /// The index of `state`'s value for the property at `index`.
    /// `state` must be one of this block's states.
    pub fn value_index(&self, state: u32, index: usize) -> usize {
        debug_assert!(self.states().contains(&state));
        let len = self.properties[index].values.len() as u32;
        ((state - self.first_state) / self.stride(index) % len) as usize
    }

    /// `state`'s value for a property, if the block has it.
    pub fn value(&self, state: u32, property: &str) -> Option<&'static str> {
        let (i, p) = self.property(property)?;
        Some(p.values[self.value_index(state, i)])
    }

    /// The state that differs from `state` only in one property's value.
    pub fn with_value(&self, state: u32, property: &str, value: &str) -> Option<u32> {
        let (i, p) = self.property(property)?;
        let new = p.index_of(value)? as u32;
        let old = self.value_index(state, i) as u32;
        let stride = self.stride(i);
        Some(state - old * stride + new * stride)
    }

    /// Every `(property, value)` of `state`, in property-name order.
    pub fn values(&'static self, state: u32) -> impl Iterator<Item = (&'static str, &'static str)> {
        self.properties
            .iter()
            .enumerate()
            .map(move |(i, p)| (p.name, p.values[self.value_index(state, i)]))
    }
}

/// Builds the state-id → block-index table at compile time. The blocks'
/// state ranges must cover `0..N` (the generator checks this).
pub const fn state_to_block<const N: usize>(blocks: &[BlockInfo]) -> [u16; N] {
    let mut table = [0u16; N];
    let mut b = 0;
    while b < blocks.len() {
        let mut s = blocks[b].first_state as usize;
        let end = s + blocks[b].state_count as usize;
        while s < end {
            table[s] = b as u16;
            s += 1;
        }
        b += 1;
    }
    table
}
