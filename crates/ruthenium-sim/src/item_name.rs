//! What a stack is called (`ItemStack.getHoverName`, and the items' own `getName`), as a
//! component in its NBT form: what draws it has the language.

use ruthenium_protocol::nbt::{Compound, Tag};

use crate::item::{DYE_NAMES, Stack};

/// The colour of each rarity's names.
const RARITY_COLOURS: [&str; 4] = ["white", "yellow", "aqua", "light_purple"];

/// A translation, filled in with `with`.
pub(crate) fn translation(key: &str, with: Vec<Tag>) -> Tag {
    let mut c = Compound::new();
    c.put("translate", Tag::String(key.to_owned()));
    if !with.is_empty() {
        c.put("with", Tag::List(with));
    }
    Tag::Compound(c)
}

/// `Item.getName`, with what some items make of their components.
fn item_name(stack: &Stack) -> Tag {
    let c = &stack.c;
    let is = |class: &str| c.classes.iter().any(|n| n == class);
    let own = || {
        c.item_name
            .as_deref()
            .cloned()
            .unwrap_or_else(|| Tag::String(String::new()))
    };
    let Some(description) = &c.description else {
        return own();
    };
    if is("CompassItem") && c.lodestone {
        return translation("item.minecraft.lodestone_compass", Vec::new());
    }
    if is("ShieldItem")
        && let Some(dye) = c.base_color.and_then(|d| DYE_NAMES.get(usize::from(d)))
    {
        return translation(&format!("{description}.{dye}"), Vec::new());
    }
    if is("PlayerHeadItem")
        && let Some(name) = c.profile.as_ref().and_then(|p| p.name.clone())
    {
        return translation(&format!("{description}.named"), vec![Tag::String(name)]);
    }
    if (is("PotionItem") || is("TippedArrowItem"))
        && let Some(potion) = &c.potion_name
    {
        return translation(&format!("{description}.effect.{potion}"), Vec::new());
    }
    own()
}

/// `getHoverName`: the name it was given, a written book's title, or its item's name.
pub fn hover_name(stack: &Stack) -> Tag {
    let c = &stack.c;
    if let Some(name) = &c.custom_name {
        return (**name).clone();
    }
    if let Some(title) = c.book_title.as_deref().filter(|t| !t.trim().is_empty()) {
        return Tag::String(title.to_owned());
    }
    item_name(stack)
}

/// `getStyledHoverName`: in its rarity's colour, and slanted if it was given.
pub fn styled_name(stack: &Stack) -> Tag {
    let c = &stack.c;
    // `getRarity`: an enchanted one's is higher.
    let rarity = match (c.rarity, c.enchantments.is_empty()) {
        (r, true) => r,
        (0 | 1, false) => 2,
        (_, false) => 3,
    };
    let mut name = Compound::new();
    name.put("text", Tag::String(String::new()));
    let colour = RARITY_COLOURS[usize::from(rarity.min(3))];
    name.put("color", Tag::String(colour.to_owned()));
    if c.custom_name.is_some() {
        name.put("italic", Tag::Byte(1));
    }
    name.put("extra", Tag::List(vec![hover_name(stack)]));
    Tag::Compound(name)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::item::Components;

    fn stack(c: Components) -> Stack {
        Stack {
            identity: 1,
            item: 1,
            count: 1,
            c: Arc::new(c),
            ..Stack::empty()
        }
    }

    fn of(class: &str, description: &str) -> Components {
        Components {
            classes: vec![class.to_owned(), "Item".to_owned()].into(),
            description: Some(description.into()),
            item_name: Some(Arc::new(translation(description, Vec::new()))),
            ..Components::default()
        }
    }

    fn key(tag: &Tag) -> String {
        tag.as_compound()
            .and_then(|c| c.string("translate"))
            .unwrap_or("")
            .to_owned()
    }

    #[test]
    fn items_are_named_by_what_they_are() {
        let name = |c: Components| key(&hover_name(&stack(c)));
        assert_eq!(name(of("Item", "item.x")), "item.x");
        let compass = |lodestone| Components {
            lodestone,
            ..of("CompassItem", "item.compass")
        };
        assert_eq!(name(compass(false)), "item.compass");
        assert_eq!(name(compass(true)), "item.minecraft.lodestone_compass");
        let shield = |base_color| Components {
            base_color,
            ..of("ShieldItem", "item.shield")
        };
        assert_eq!(name(shield(None)), "item.shield");
        assert_eq!(name(shield(Some(15))), "item.shield.black");
        for class in ["PotionItem", "TippedArrowItem"] {
            let c = Components {
                potion_name: Some("swiftness".into()),
                ..of(class, "item.p")
            };
            assert_eq!(name(c), "item.p.effect.swiftness");
        }
        // Others are not named by what would name those.
        let c = Components {
            potion_name: Some("swiftness".into()),
            base_color: Some(3),
            lodestone: true,
            ..of("Item", "item.x")
        };
        assert_eq!(name(c), "item.x");
        let head = Components {
            profile: Some(Arc::new(ruthenium_protocol::profile::ResolvableProfile {
                name: Some("Kai".into()),
                ..Default::default()
            })),
            ..of("PlayerHeadItem", "block.head")
        };
        let named = hover_name(&stack(head));
        assert_eq!(key(&named), "block.head.named");
        assert_eq!(
            named.as_compound().unwrap().list("with"),
            [Tag::String("Kai".into())]
        );
        // A patched name counts, but not over what some items make of theirs.
        let patched = |class| Components {
            item_name: Some(Arc::new(Tag::String("Patched".into()))),
            potion_name: Some("luck".into()),
            ..of(class, "item.p")
        };
        assert_eq!(
            hover_name(&stack(patched("Item"))),
            Tag::String("Patched".into())
        );
        assert_eq!(name(patched("PotionItem")), "item.p.effect.luck");
    }

    #[test]
    fn given_names_come_first_and_lean() {
        let given = |title: Option<&str>, custom: Option<&str>| {
            stack(Components {
                book_title: title.map(Arc::from),
                custom_name: custom.map(|n| Arc::new(Tag::String(n.into()))),
                ..of("Item", "item.book")
            })
        };
        assert_eq!(
            hover_name(&given(Some("Title"), Some("Mine"))),
            Tag::String("Mine".into())
        );
        assert_eq!(
            hover_name(&given(Some("Title"), None)),
            Tag::String("Title".into())
        );
        assert_eq!(key(&hover_name(&given(Some("  "), None))), "item.book");
        let italic = |s: &Stack| styled_name(s).as_compound().unwrap().boolean("italic");
        assert_eq!(italic(&given(None, Some("Mine"))), Some(true));
        assert_eq!(italic(&given(Some("Title"), None)), None);
    }

    #[test]
    fn rarer_names_have_a_colour_and_enchanted_ones_a_rarer() {
        let colour = |rarity, enchanted: bool| {
            let c = Components {
                rarity,
                enchantments: if enchanted { vec![(1, 1)] } else { Vec::new() },
                ..of("Item", "item.x")
            };
            let name = styled_name(&stack(c));
            let colour = name.as_compound().unwrap().string("color").unwrap();
            colour.to_owned()
        };
        let plain = [0, 1, 2, 3].map(|r| colour(r, false));
        assert_eq!(plain, ["white", "yellow", "aqua", "light_purple"]);
        let enchanted = [0, 1, 2, 3].map(|r| colour(r, true));
        assert_eq!(enchanted, ["aqua", "aqua", "light_purple", "light_purple"]);
    }
}
