//! Recipe displays and the stonecutter's recipes as the server sends them
//! (`SlotDisplay.STREAM_CODEC`, `SelectableRecipe.SingleInputSet.noRecipeCodec`): what a
//! screen draws for a recipe, and which inputs it takes.

use crate::item::{self, HolderSet, ItemStack, TrimPart};
use crate::wire::{Reader, WireError};

/// How deep displays may nest (a composite of composites...).
const MAX_DEPTH: u32 = 16;

/// `SlotDisplay`: what a recipe's slot shows. Only the parts a screen draws are kept of
/// the pattern of a smithing trim demonstration.
#[derive(Debug, Clone, PartialEq)]
pub enum SlotDisplay {
    Empty,
    AnyFuel,
    WithAnyPotion(Box<SlotDisplay>),
    /// A display filtered to the stacks with a component (its type id).
    OnlyWithComponent(Box<SlotDisplay>, i32),
    /// An item, by registry id.
    Item(i32),
    ItemStack(ItemStack),
    /// The items of a holder set.
    Tag(HolderSet),
    Dyed {
        dye: Box<SlotDisplay>,
        target: Box<SlotDisplay>,
    },
    SmithingTrim {
        base: Box<SlotDisplay>,
        material: Box<SlotDisplay>,
        pattern: TrimPart,
    },
    WithRemainder {
        input: Box<SlotDisplay>,
        remainder: Box<SlotDisplay>,
    },
    Composite(Vec<SlotDisplay>),
}

impl SlotDisplay {
    /// `SlotDisplay.STREAM_CODEC`: the `minecraft:slot_display` registry id, then the
    /// type's own fields.
    pub fn read(r: &mut Reader<'_>) -> Result<SlotDisplay, WireError> {
        read_display(r, 0)
    }
}

fn read_display(r: &mut Reader<'_>, depth: u32) -> Result<SlotDisplay, WireError> {
    if depth >= MAX_DEPTH {
        return Err(WireError::Invalid("slot display nested too deeply"));
    }
    let inner = |r: &mut Reader<'_>| read_display(r, depth + 1).map(Box::new);
    Ok(match r.varint()? {
        0 => SlotDisplay::Empty,
        1 => SlotDisplay::AnyFuel,
        2 => SlotDisplay::WithAnyPotion(inner(r)?),
        3 => {
            let source = inner(r)?;
            SlotDisplay::OnlyWithComponent(source, r.varint()?)
        }
        4 => SlotDisplay::Item(r.varint()?),
        5 => SlotDisplay::ItemStack(ItemStack::read_template(r)?),
        6 => SlotDisplay::Tag(HolderSet::read(r)?),
        7 => SlotDisplay::Dyed {
            dye: inner(r)?,
            target: inner(r)?,
        },
        8 => SlotDisplay::SmithingTrim {
            base: inner(r)?,
            material: inner(r)?,
            pattern: item::read_trim_pattern(r)?,
        },
        9 => SlotDisplay::WithRemainder {
            input: inner(r)?,
            remainder: inner(r)?,
        },
        10 => {
            let n = r.count(usize::MAX)?;
            let mut contents = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                contents.push(read_display(r, depth + 1)?);
            }
            SlotDisplay::Composite(contents)
        }
        _ => return Err(WireError::Invalid("slot display type")),
    })
}

/// One of the stonecutter's recipes as the client gets it
/// (`SelectableRecipe.SingleInputEntry`, without the recipe): the items it takes and
/// the button it shows.
#[derive(Debug, Clone, PartialEq)]
pub struct StonecutterEntry {
    pub input: HolderSet,
    pub display: SlotDisplay,
}

/// `SelectableRecipe.SingleInputSet.noRecipeCodec`: the stonecutter's recipes in the
/// server's order.
pub fn read_stonecutter_recipes(r: &mut Reader<'_>) -> Result<Vec<StonecutterEntry>, WireError> {
    let n = r.count(usize::MAX)?;
    let mut entries = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        let input = HolderSet::read(r)?;
        let display = SlotDisplay::read(r)?;
        entries.push(StonecutterEntry { input, display });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_read_as_vanilla_writes_them() {
        // A composite of an item, a stack of two of item 7 and a tag.
        let mut b = vec![10, 3, 4, 5, 5, 7, 2, 0, 0, 6, 0];
        crate::wire::write_string(&mut b, "minecraft:planks");
        let d = SlotDisplay::read(&mut Reader::new(&b)).unwrap();
        let SlotDisplay::Composite(parts) = d else {
            panic!("{d:?}");
        };
        assert_eq!(parts[0], SlotDisplay::Item(5));
        let SlotDisplay::ItemStack(s) = &parts[1] else {
            panic!();
        };
        assert_eq!((s.item, s.count), (7, 2));
        assert_eq!(
            parts[2],
            SlotDisplay::Tag(HolderSet::Tag("minecraft:planks".into()))
        );
        // The stonecutter's list: (holder set, display) pairs.
        let b = [1, 3, 1, 2, 4, 9];
        let entries = read_stonecutter_recipes(&mut Reader::new(&b)).unwrap();
        assert_eq!(
            entries,
            [StonecutterEntry {
                input: HolderSet::Ids(vec![1, 2]),
                display: SlotDisplay::Item(9),
            }]
        );
        assert!(SlotDisplay::read(&mut Reader::new(&[11])).is_err());
    }
}
