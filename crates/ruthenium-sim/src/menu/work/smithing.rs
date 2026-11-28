//! `SmithingMenu` on the client: the template, base and addition slots take what the
//! server's recipe property sets say; the client never finds a recipe, so any change to
//! the inputs clears the result until the server sends it; the error arrow is the
//! server's data slot. The screen's cycling empty-slot icons (`CyclingSlotBackground`)
//! tick here.

use super::KindState;
use crate::item::Stack;
use crate::menu::Menu;

/// `CyclingSlotBackground.ICON_CHANGE_TICK_RATE`.
pub const ICON_CHANGE_TICK_RATE: i32 = 30;

/// One slot's cycling icons.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cycling {
    pub icons: Vec<&'static str>,
    pub tick: i32,
    pub index: usize,
}

impl Cycling {
    /// `CyclingSlotBackground.tick`.
    pub fn tick(&mut self, icons: &[&'static str]) {
        if self.icons != icons {
            self.icons = icons.to_vec();
            self.index = 0;
        }
        self.tick += 1;
        if !self.icons.is_empty() && self.tick % ICON_CHANGE_TICK_RATE == 0 {
            self.index = (self.index + 1) % self.icons.len();
        }
    }

    /// What to draw in the empty slot: (icon, alpha) for the one fading out and the one
    /// shown (`extractRenderState`, `getIconTransitionTransparency`).
    pub fn shown(&self, partial: f32) -> Vec<(&'static str, f32)> {
        if self.icons.is_empty() {
            return Vec::new();
        }
        let transition = self.icons.len() > 1 && self.tick >= ICON_CHANGE_TICK_RATE;
        let alpha = if transition {
            ((self.tick % ICON_CHANGE_TICK_RATE) as f32 + partial).min(4.0) / 4.0
        } else {
            1.0
        };
        let mut out = Vec::new();
        if alpha < 1.0 {
            let prev = (self.index + self.icons.len() - 1) % self.icons.len();
            out.push((self.icons[prev], 1.0 - alpha));
        }
        out.push((self.icons[self.index], alpha));
        out
    }
}

/// The smithing screen's icons.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Smithing {
    pub template: Cycling,
    pub base: Cycling,
    pub addition: Cycling,
}

/// `EMPTY_SLOT_SMITHING_TEMPLATES`.
pub const TEMPLATE_ICONS: [&str; 2] = [
    "container/slot/smithing_template_armor_trim",
    "container/slot/smithing_template_netherite_upgrade",
];

/// `createTrimmableArmorIconList`, `createTrimmableMaterialIconList`.
pub const TRIM_BASE_ICONS: [&str; 4] = [
    "container/slot/helmet",
    "container/slot/chestplate",
    "container/slot/leggings",
    "container/slot/boots",
];
pub const TRIM_ADDITION_ICONS: [&str; 7] = [
    "container/slot/ingot",
    "container/slot/redstone_dust",
    "container/slot/lapis_lazuli",
    "container/slot/quartz",
    "container/slot/diamond",
    "container/slot/emerald",
    "container/slot/amethyst_shard",
];
/// `createNetheriteUpgradeIconList`, `createNetheriteUpgradeMaterialList`.
pub const UPGRADE_BASE_ICONS: [&str; 11] = [
    "container/slot/helmet",
    "container/slot/sword",
    "container/slot/chestplate",
    "container/slot/pickaxe",
    "container/slot/leggings",
    "container/slot/axe",
    "container/slot/boots",
    "container/slot/hoe",
    "container/slot/shovel",
    "container/slot/nautilus_armor",
    "container/slot/spear",
];
pub const UPGRADE_ADDITION_ICONS: [&str; 1] = ["container/slot/ingot"];

/// A template's kind: what it applies to (its icons and slot descriptions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    ArmorTrim,
    NetheriteUpgrade,
}

impl Template {
    /// The `SmithingTemplateItem` a stack is, if any (vanilla's two kinds, by name).
    pub fn of(s: &Stack) -> Option<Template> {
        if s.is_empty() || !s.c.is("SmithingTemplateItem") {
            return None;
        }
        Some(
            if s.c.name == "minecraft:netherite_upgrade_smithing_template" {
                Template::NetheriteUpgrade
            } else {
                Template::ArmorTrim
            },
        )
    }

    pub fn base_icons(self) -> &'static [&'static str] {
        match self {
            Template::ArmorTrim => &TRIM_BASE_ICONS,
            Template::NetheriteUpgrade => &UPGRADE_BASE_ICONS,
        }
    }

    pub fn addition_icons(self) -> &'static [&'static str] {
        match self {
            Template::ArmorTrim => &TRIM_ADDITION_ICONS,
            Template::NetheriteUpgrade => &UPGRADE_ADDITION_ICONS,
        }
    }

    /// `getBaseSlotDescription`, `getAdditionSlotDescription`: translation keys.
    pub fn base_description(self) -> &'static str {
        match self {
            Template::ArmorTrim => {
                "item.minecraft.smithing_template.armor_trim.base_slot_description"
            }
            Template::NetheriteUpgrade => {
                "item.minecraft.smithing_template.netherite_upgrade.base_slot_description"
            }
        }
    }

    pub fn addition_description(self) -> &'static str {
        match self {
            Template::ArmorTrim => {
                "item.minecraft.smithing_template.armor_trim.additions_slot_description"
            }
            Template::NetheriteUpgrade => {
                "item.minecraft.smithing_template.netherite_upgrade.additions_slot_description"
            }
        }
    }
}

/// The inputs' `setChanged`: the client finds no recipe (`createResult` on a
/// `ClientLevel`), so the result goes. The result's own changes do nothing here.
pub(super) fn changed(m: &mut Menu, index: usize) {
    if index < 3 {
        m.own[3] = Stack::empty();
    }
}

/// `SmithingMenu.onTake`: each input loses one (each `setItem` clears the result again).
pub(super) fn on_take(m: &mut Menu, taken: &mut Stack) {
    if let Some(w) = m.work.as_ref() {
        let items = w.data.items.clone();
        super::crafted_by(&items, taken);
    }
    for i in 0..3 {
        if !m.own[i].is_empty() {
            let mut s = m.own[i].clone();
            s.shrink(1);
            if s.count <= 0 {
                s = Stack::empty();
            }
            m.own[i] = s;
            changed(m, i);
        }
    }
}

/// `containerTick`: the icons cycle.
pub fn tick(m: &mut Menu) {
    let template = Template::of(&m.own[0]);
    let Some(KindState::Smithing(s)) = m.work.as_mut().map(|w| &mut w.kind) else {
        return;
    };
    s.template.tick(&TEMPLATE_ICONS);
    s.base.tick(template.map_or(&[][..], Template::base_icons));
    s.addition
        .tick(template.map_or(&[][..], Template::addition_icons));
}
