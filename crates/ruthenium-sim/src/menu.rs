//! Menus as the client runs them (`AbstractContainerMenu` and its kinds): the slots
//! with their containers and rules, the state id, the stack on the cursor, and what a
//! click does to them (`clicked`, `doClick`, each menu's `quickMoveStack`), which
//! the client predicts before it tells the server ([`Menu::handle_container_input`],
//! `MultiPlayerGameMode.handleContainerInput`).
//!
//! The player's own menu (`InventoryMenu`, container 0) is always there; another menu
//! opens over it when the server sends `open_screen` and closes when either side says.
//! Kinds: the player's inventory (2x2 grid, armour, off hand), chests of one to six
//! rows (barrels, ender chests, large chests: `generic_9xN`), dispensers and droppers
//! (`generic_3x3`), shulker boxes, the crafting table, the furnaces (furnace, blast
//! furnace, smoker, with their four data slots) and hoppers. The client never works out
//! recipes: a crafting result is whatever the server last put there, and taking it uses
//! up one of each ingredient (`ResultSlot.onTake`, with
//! `CraftingRecipe.defaultCraftingReminder`'s remainders).
//!
//! Not modelled: bundles' own clicks (`BundleItem.overrideStackedOnOther` and
//! `overrideOtherStackedOnMe`: such a click is predicted as if the bundle were a plain
//! item) and the equip sounds of armour slots.

use crate::inventory::Inventory;
use crate::item::{EquipmentSlot, Items, RecipeSets, Stack};

pub mod creative;
pub mod mount;
pub mod screen;
pub mod stations;

/// `AbstractContainerMenu.SLOT_CLICKED_OUTSIDE`.
pub const SLOT_OUTSIDE: i32 = -999;
/// `Container.getMaxStackSize`'s default: every container here holds up to 99.
const CONTAINER_MAX: i32 = 99;

/// `ContainerInput`, by its network id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Input {
    Pickup = 0,
    QuickMove = 1,
    Swap = 2,
    Clone = 3,
    Throw = 4,
    QuickCraft = 5,
    PickupAll = 6,
}

impl Input {
    pub fn id(self) -> i32 {
        self as i32
    }

    pub fn from_id(id: i32) -> Option<Input> {
        Some(match id {
            0 => Input::Pickup,
            1 => Input::QuickMove,
            2 => Input::Swap,
            3 => Input::Clone,
            4 => Input::Throw,
            5 => Input::QuickCraft,
            6 => Input::PickupAll,
            _ => return None,
        })
    }

    pub fn from_name(name: &str) -> Option<Input> {
        Some(match name.to_ascii_lowercase().as_str() {
            "pickup" => Input::Pickup,
            "quick_move" => Input::QuickMove,
            "swap" => Input::Swap,
            "clone" => Input::Clone,
            "throw" => Input::Throw,
            "quick_craft" => Input::QuickCraft,
            "pickup_all" => Input::PickupAll,
            _ => return None,
        })
    }
}

/// The kinds of menu the client runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `InventoryMenu`.
    Inventory,
    /// `ChestMenu` of so many rows.
    Chest(u8),
    /// `DispenserMenu`.
    Dispenser,
    ShulkerBox,
    /// `CraftingMenu`.
    Crafting,
    Furnace,
    BlastFurnace,
    Smoker,
    Hopper,
    /// `BeaconMenu`, `BrewingStandMenu`, `EnchantmentMenu`, `GrindstoneMenu`,
    /// `LecternMenu` ([`stations`]).
    Beacon,
    BrewingStand,
    Enchantment,
    Grindstone,
    Lectern,
    /// `HorseInventoryMenu` or `NautilusInventoryMenu` ([`mount`]).
    Mount(mount::Mount),
    /// A menu the client doesn't run (an anvil, an enchanting table, a horse's
    /// inventory...): it stays open with its container id, so the server's packets for
    /// it land, and its screen is a placeholder that clicks nothing.
    Placeholder(Placeholder),
    /// The creative inventory's `ItemPickerMenu` (see [`creative`]).
    Creative,
}

/// The layout of a menu the client doesn't run, as far as the slots' numbering goes:
/// its own slots before the player's inventory and after it (a crafter's result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placeholder {
    pub before: u8,
    pub after: u8,
    /// Whether the menu shows the player's inventory (a lectern's doesn't).
    pub inventory: bool,
}

impl Placeholder {
    /// The layout of a `minecraft:menu` the client doesn't run, by its slots
    /// (`addSlot` and `addStandardInventorySlots` in each menu's constructor).
    pub fn for_menu(name: &str) -> Placeholder {
        let (before, after, inventory) = match name.trim_start_matches("minecraft:") {
            "crafter_3x3" => (9, 1, true),
            "anvil" | "grindstone" | "merchant" | "cartography_table" => (3, 0, true),
            "beacon" => (1, 0, true),
            "brewing_stand" => (5, 0, true),
            "enchantment" | "stonecutter" => (2, 0, true),
            "loom" | "smithing" => (4, 0, true),
            "lectern" => (1, 0, false),
            // A menu this table doesn't know: no slots, every packet for it ignored.
            _ => (0, 0, false),
        };
        Placeholder {
            before,
            after,
            inventory,
        }
    }

    /// `HorseInventoryMenu` and `NautilusInventoryMenu`: the saddle and the body slots,
    /// then three rows of `columns` (none for a mount without a chest).
    pub fn mount(columns: i32) -> Placeholder {
        let chest = columns.clamp(0, 5) as u8 * 3;
        Placeholder {
            before: 2 + chest,
            after: 0,
            inventory: true,
        }
    }
}

/// Where a placeholder's own slots are: off the screen, never under the pointer.
pub const HIDDEN: i32 = -10_000;

impl Kind {
    /// The kind of a `minecraft:menu` registry id, if the client runs it.
    pub fn from_menu_type(id: i32) -> Option<Kind> {
        use ruthenium_data::v26_3::registries::Menu;
        let name = Menu::from_id(u32::try_from(id).ok()?)?.name();
        Kind::from_menu_name(name)
    }

    pub fn from_menu_name(name: &str) -> Option<Kind> {
        Some(match name.trim_start_matches("minecraft:") {
            "generic_9x1" => Kind::Chest(1),
            "generic_9x2" => Kind::Chest(2),
            "generic_9x3" => Kind::Chest(3),
            "generic_9x4" => Kind::Chest(4),
            "generic_9x5" => Kind::Chest(5),
            "generic_9x6" => Kind::Chest(6),
            "generic_3x3" => Kind::Dispenser,
            "shulker_box" => Kind::ShulkerBox,
            "crafting" => Kind::Crafting,
            "furnace" => Kind::Furnace,
            "blast_furnace" => Kind::BlastFurnace,
            "smoker" => Kind::Smoker,
            "hopper" => Kind::Hopper,
            "beacon" => Kind::Beacon,
            "brewing_stand" => Kind::BrewingStand,
            "enchantment" => Kind::Enchantment,
            "grindstone" => Kind::Grindstone,
            "lectern" => Kind::Lectern,
            _ => return None,
        })
    }

    /// The recipe property set a furnace of this kind smelts (`acceptedInputs`).
    fn smelts(self) -> Option<&'static str> {
        match self {
            Kind::Furnace => Some("minecraft:furnace_input"),
            Kind::BlastFurnace => Some("minecraft:blast_furnace_input"),
            Kind::Smoker => Some("minecraft:smoker_input"),
            _ => None,
        }
    }
}

/// Which container a slot shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Container {
    /// The player's `Inventory` (its numbering: 0 to 35, then the equipment).
    Inventory,
    /// The menu's own: a chest's, a furnace's, or a crafting menu's result.
    Own,
    /// A crafting menu's grid.
    Grid,
}

/// What a slot allows (`Slot` and its subclasses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Plain,
    /// `ArmorSlot`.
    Armor(EquipmentSlot),
    /// The inventory menu's off-hand slot (a plain slot with its own icon).
    Offhand,
    /// `ResultSlot`: a crafting result.
    CraftResult,
    /// `FurnaceFuelSlot`.
    FurnaceFuel,
    /// `FurnaceResultSlot`.
    FurnaceResult,
    /// `ShulkerBoxSlot`.
    ShulkerBox,
    /// The creative inventory's `CONTAINER` (its rows and the destroy slot): nothing is
    /// dragged onto it (`canDragTo`, which the client reads wherever it reads
    /// `mayPlace` for it) or gathered from it (`canTakeItemForPickAll`), and a locked
    /// stack can't be picked up (`CustomCreativeSlot.mayPickup`).
    Creative,
    /// A mount's saddle or body armour (`ArmorSlot` on the mount).
    MountEquipment(EquipmentSlot),
    /// `BeaconMenu.PaymentSlot`; `BrewingStandMenu`'s `PotionSlot`, `IngredientsSlot`
    /// and `FuelSlot`; `EnchantmentMenu`'s item and lapis slots; `GrindstoneMenu`'s
    /// inputs and result ([`stations`]).
    BeaconPayment,
    Potion,
    BrewingIngredient,
    BrewingFuel,
    EnchantItem,
    Lapis,
    GrindstoneInput,
    GrindstoneResult,
}

/// `Slot`: its container and place there, where it is drawn, and its rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub container: Container,
    pub index: usize,
    pub x: i32,
    pub y: i32,
    pub rule: Rule,
}

/// Who clicks: what of the player and the session the rules read.
#[derive(Debug, Clone, Copy)]
pub struct Who<'a> {
    pub items: &'a Items,
    pub recipes: &'a RecipeSets,
    /// `isCreative` (armour slots) and `hasInfiniteMaterials` (cloning, dragging
    /// copies), which are the same for a player.
    pub creative: bool,
}

/// What a click did besides the menu: the stacks the player dropped
/// (`Player.drop`: on the client only the main hand swings, once for each).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub dropped: Vec<Stack>,
}

/// The player's menus: its own (`inventoryMenu`) and the one open over it, if any
/// (`containerMenu` when it isn't the player's own).
#[derive(Debug, Clone, PartialEq)]
pub struct Menus {
    pub inventory: Menu,
    pub open: Option<Menu>,
}

impl Default for Menus {
    fn default() -> Menus {
        Menus {
            inventory: Menu::inventory(),
            open: None,
        }
    }
}

impl Menus {
    /// `containerMenu`.
    pub fn container(&self) -> &Menu {
        self.open.as_ref().unwrap_or(&self.inventory)
    }

    pub fn container_mut(&mut self) -> &mut Menu {
        self.open.as_mut().unwrap_or(&mut self.inventory)
    }

    /// `hasContainerOpen`.
    pub fn has_container_open(&self) -> bool {
        self.open.is_some()
    }
}

/// `AbstractContainerMenu` as the client holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub id: i32,
    pub kind: Kind,
    pub slots: Vec<Slot>,
    /// The menu's own container (a crafting menu's: its result).
    pub own: Vec<Stack>,
    /// A crafting menu's grid, and its width.
    pub grid: Vec<Stack>,
    grid_width: usize,
    /// The data slots (`ContainerData`): a furnace's lit time, lit duration,
    /// cooking progress and cooking time.
    pub data: Vec<i32>,
    pub carried: Stack,
    pub state_id: i32,
    quickcraft_type: i32,
    quickcraft_status: i32,
    /// `quickcraftSlots` (a set), in the order slots joined it.
    quickcraft_slots: Vec<usize>,
    /// The slots' identity hash codes (`System.identityHashCode`, random per object),
    /// which order the sets of them a screen keeps.
    pub slot_hashes: Vec<u32>,
    /// The items a grindstone makes its result with (the session's).
    pub items: stations::MenuItems,
}

/// Identity hash codes as HotSpot's thread-local xor-shift hands them out: 31 bits,
/// random enough.
fn identity_hashes(n: usize) -> Vec<u32> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut s = STATE.load(Ordering::Relaxed);
    if s == 0 {
        s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64)
            | 1;
    }
    let out = (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 33) as u32 & 0x7FFF_FFFF
        })
        .collect();
    STATE.store(s, Ordering::Relaxed);
    out
}

fn slot(container: Container, index: usize, x: i32, y: i32, rule: Rule) -> Slot {
    Slot {
        container,
        index,
        x,
        y,
        rule,
    }
}

/// `addStandardInventorySlots`: the three rows, then the hotbar 58 lower.
fn standard_inventory(slots: &mut Vec<Slot>, left: i32, top: i32) {
    for y in 0..3 {
        for x in 0..9 {
            slots.push(slot(
                Container::Inventory,
                (x + (y + 1) * 9) as usize,
                left + x * 18,
                top + y * 18,
                Rule::Plain,
            ));
        }
    }
    for x in 0..9 {
        slots.push(slot(
            Container::Inventory,
            x as usize,
            left + x * 18,
            top + 58,
            Rule::Plain,
        ));
    }
}

/// `addCraftingGridSlots`.
fn crafting_grid(slots: &mut Vec<Slot>, width: i32, left: i32, top: i32) {
    for y in 0..width {
        for x in 0..width {
            slots.push(slot(
                Container::Grid,
                (x + y * width) as usize,
                left + x * 18,
                top + y * 18,
                Rule::Plain,
            ));
        }
    }
}

/// `Mth.floor` of `(float) count / slots`.
fn floor_div(count: i32, slots: usize) -> i32 {
    (count as f32 / slots as f32).floor() as i32
}

impl Menu {
    /// A menu of `kind` with container id `id`, empty.
    pub fn new(kind: Kind, id: i32) -> Menu {
        let mut slots = Vec::new();
        let (own, grid_width, data) = match kind {
            Kind::Inventory => {
                slots.push(slot(Container::Own, 0, 154, 28, Rule::CraftResult));
                crafting_grid(&mut slots, 2, 98, 18);
                // `SLOT_IDS`: head first, at inventory slots 39 down to 36.
                let armor = [
                    EquipmentSlot::Head,
                    EquipmentSlot::Chest,
                    EquipmentSlot::Legs,
                    EquipmentSlot::Feet,
                ];
                for (i, eq) in armor.into_iter().enumerate() {
                    slots.push(slot(
                        Container::Inventory,
                        39 - i,
                        8,
                        8 + i as i32 * 18,
                        Rule::Armor(eq),
                    ));
                }
                standard_inventory(&mut slots, 8, 84);
                slots.push(slot(
                    Container::Inventory,
                    crate::inventory::SLOT_OFFHAND,
                    77,
                    62,
                    Rule::Offhand,
                ));
                (1, 2, 0)
            }
            Kind::Chest(rows) => {
                let rows = i32::from(rows);
                for y in 0..rows {
                    for x in 0..9 {
                        slots.push(slot(
                            Container::Own,
                            (x + y * 9) as usize,
                            8 + x * 18,
                            18 + y * 18,
                            Rule::Plain,
                        ));
                    }
                }
                standard_inventory(&mut slots, 8, 18 + rows * 18 + 13);
                (rows as usize * 9, 0, 0)
            }
            Kind::Dispenser => {
                for y in 0..3 {
                    for x in 0..3 {
                        slots.push(slot(
                            Container::Own,
                            (x + y * 3) as usize,
                            62 + x * 18,
                            17 + y * 18,
                            Rule::Plain,
                        ));
                    }
                }
                standard_inventory(&mut slots, 8, 84);
                (9, 0, 0)
            }
            Kind::ShulkerBox => {
                for y in 0..3 {
                    for x in 0..9 {
                        slots.push(slot(
                            Container::Own,
                            (x + y * 9) as usize,
                            8 + x * 18,
                            18 + y * 18,
                            Rule::ShulkerBox,
                        ));
                    }
                }
                standard_inventory(&mut slots, 8, 84);
                (27, 0, 0)
            }
            Kind::Crafting => {
                slots.push(slot(Container::Own, 0, 124, 35, Rule::CraftResult));
                crafting_grid(&mut slots, 3, 30, 17);
                standard_inventory(&mut slots, 8, 84);
                (1, 3, 0)
            }
            Kind::Furnace | Kind::BlastFurnace | Kind::Smoker => {
                slots.push(slot(Container::Own, 0, 56, 17, Rule::Plain));
                slots.push(slot(Container::Own, 1, 56, 53, Rule::FurnaceFuel));
                slots.push(slot(Container::Own, 2, 116, 35, Rule::FurnaceResult));
                standard_inventory(&mut slots, 8, 84);
                (3, 0, 4)
            }
            Kind::Beacon
            | Kind::BrewingStand
            | Kind::Enchantment
            | Kind::Grindstone
            | Kind::Lectern
            | Kind::Mount(_) => {
                let (own, data) = stations::layout(kind, &mut slots);
                (own, 0, data)
            }
            Kind::Placeholder(p) => {
                let hidden = |slots: &mut Vec<Slot>, from: u8, n: u8| {
                    for i in from..from + n {
                        slots.push(slot(
                            Container::Own,
                            usize::from(i),
                            HIDDEN,
                            HIDDEN,
                            Rule::Plain,
                        ));
                    }
                };
                hidden(&mut slots, 0, p.before);
                if p.inventory {
                    standard_inventory(&mut slots, 8, 84);
                }
                hidden(&mut slots, p.before, p.after);
                (usize::from(p.before + p.after), 0, 0)
            }
            // Its slots are `creative`'s to lay out.
            Kind::Creative => (0, 0, 0),
            Kind::Hopper => {
                for x in 0..5 {
                    slots.push(slot(
                        Container::Own,
                        x as usize,
                        44 + x * 18,
                        20,
                        Rule::Plain,
                    ));
                }
                standard_inventory(&mut slots, 8, 51);
                (5, 0, 0)
            }
        };
        Menu {
            id,
            kind,
            slots,
            own: vec![Stack::empty(); own],
            grid: vec![Stack::empty(); grid_width * grid_width],
            grid_width,
            data: stations::data(kind, data),
            carried: Stack::empty(),
            state_id: 0,
            quickcraft_type: -1,
            quickcraft_status: 0,
            quickcraft_slots: Vec::new(),
            slot_hashes: Vec::new(),
            items: stations::MenuItems::default(),
        }
        .with_hashes()
    }

    fn with_hashes(mut self) -> Menu {
        self.slot_hashes = identity_hashes(self.slots.len());
        self
    }

    /// The player's own menu (`InventoryMenu`, container 0).
    pub fn inventory() -> Menu {
        Menu::new(Kind::Inventory, 0)
    }

    /// The stack slot `i` shows (`Slot.getItem`).
    pub fn item<'a>(&'a self, inv: &'a Inventory, i: usize) -> &'a Stack {
        let s = &self.slots[i];
        match s.container {
            Container::Inventory => inv.get(s.index),
            Container::Own => &self.own[s.index],
            Container::Grid => &self.grid[s.index],
        }
    }

    fn get(&self, inv: &Inventory, i: usize) -> Stack {
        self.item(inv, i).clone()
    }

    fn has_item(&self, inv: &Inventory, i: usize) -> bool {
        !self.item(inv, i).is_empty()
    }

    /// `Slot.set` (and `setByPlayer`): the container's `setItem`, and its
    /// `setChanged`.
    fn set(&mut self, inv: &mut Inventory, i: usize, stack: Stack) {
        self.set_quiet(inv, i, stack);
        self.touched(i);
    }

    /// The container's `setItem` alone (a stack changed in place).
    fn set_quiet(&mut self, inv: &mut Inventory, i: usize, stack: Stack) {
        let s = self.slots[i];
        match s.container {
            Container::Inventory => inv.set(s.index, stack),
            Container::Own => {
                self.own[s.index] = stack;
                // `SimpleContainer.setItem`'s `limitSize`: a menu's own container holds
                // no more than its most (99, a beacon's 1) and the stack's; a result's
                // and a mount's equipment aren't such containers.
                if !matches!(
                    s.rule,
                    Rule::CraftResult | Rule::GrindstoneResult | Rule::MountEquipment(_)
                ) {
                    let most = if self.kind == Kind::Beacon {
                        1
                    } else {
                        CONTAINER_MAX
                    };
                    let here = &mut self.own[s.index];
                    let max = most.min(here.max_stack_size());
                    if !here.is_empty() && here.count > max {
                        here.count = max;
                    }
                }
            }
            Container::Grid => self.grid[s.index] = stack,
        }
    }

    /// `Slot.setChanged`: a menu's own container may make something of it
    /// (`slotsChanged`: a grindstone's result).
    fn touched(&mut self, i: usize) {
        let s = self.slots[i];
        if s.container == Container::Own {
            self.station_slot_changed(s.index);
        }
    }

    /// `setItem` (`ClientboundContainerSetSlotPacket`): a slot out of range is
    /// left alone (vanilla's client would throw).
    pub fn set_item(&mut self, inv: &mut Inventory, slot: i32, state_id: i32, stack: Stack) {
        if let Some(i) = usize::try_from(slot).ok().filter(|&i| i < self.slots.len()) {
            self.set(inv, i, stack);
        }
        self.state_id = state_id;
    }

    /// `initializeContents` (`ClientboundContainerSetContentPacket`).
    pub fn initialize(
        &mut self,
        inv: &mut Inventory,
        state_id: i32,
        items: Vec<Stack>,
        carried: Stack,
    ) {
        for (i, s) in items.into_iter().enumerate().take(self.slots.len()) {
            self.set(inv, i, s);
        }
        self.carried = carried;
        self.state_id = state_id;
    }

    /// `setData` (`ClientboundContainerSetDataPacket`).
    pub fn set_data(&mut self, id: i32, value: i32) {
        if let Some(d) = usize::try_from(id).ok().and_then(|i| self.data.get_mut(i)) {
            *d = value;
        }
    }

    /// `removed` on the client: a crafting menu's result goes (`InventoryMenu`:
    /// `resultSlots.clearContent`); the grid and the cursor stay until the server
    /// says otherwise.
    pub fn removed(&mut self) {
        if self.kind == Kind::Inventory {
            self.own[0] = Stack::empty();
        }
    }

    /// The quick-craft drag under way, for the screen: its type and slots.
    pub fn quickcraft(&self) -> (i32, i32, &[usize]) {
        (
            self.quickcraft_status,
            self.quickcraft_type,
            &self.quickcraft_slots,
        )
    }

    // --- Slot rules ---------------------------------------------------------------

    /// `Slot.mayPlace`.
    pub fn may_place(&self, i: usize, stack: &Stack, _who: &Who) -> bool {
        match self.slots[i].rule {
            Rule::Plain | Rule::Offhand => true,
            // `isEquippableInSlot`: an item without `equippable` goes only in the main
            // hand.
            Rule::Armor(eq) => stack.c.equippable.is_some_and(|e| e.slot == eq),
            Rule::CraftResult | Rule::FurnaceResult => false,
            Rule::FurnaceFuel => stack.c.cooking_fuel || is_bucket(stack),
            // `canFitInsideContainerItems`: not a shulker box.
            Rule::ShulkerBox => !is_shulker_box(stack),
            Rule::Creative => false,
            r => stations::may_place(self, r, stack, _who),
        }
    }

    /// `Slot.mayPickup`: armour cursed with binding stays on a player not in creative.
    pub fn may_pickup(&self, inv: &Inventory, i: usize, who: &Who) -> bool {
        match self.slots[i].rule {
            Rule::Armor(_) | Rule::MountEquipment(_) => {
                let s = self.item(inv, i);
                s.is_empty() || who.creative || !s.c.prevent_armor_change
            }
            Rule::Creative => !creative::locked(self.item(inv, i)),
            _ => true,
        }
    }

    /// `Slot.getMaxStackSize(stack)`.
    pub fn max_stack_size(&self, i: usize, stack: &Stack, _who: &Who) -> i32 {
        rule_max_stack_size(self.slots[i].rule, stack)
    }

    /// `Slot.allowModification`.
    fn allow_modification(&self, inv: &Inventory, i: usize, who: &Who) -> bool {
        self.may_pickup(inv, i, who) && self.may_place(i, self.item(inv, i), who)
    }

    /// `Slot.remove`: the container's `removeItem` (a crafting result is taken whole).
    fn remove(&mut self, inv: &mut Inventory, i: usize, amount: i32) -> Stack {
        if matches!(
            self.slots[i].rule,
            Rule::CraftResult | Rule::GrindstoneResult
        ) {
            let taken = self.get(inv, i);
            self.set(inv, i, Stack::empty());
            return taken;
        }
        let mut stack = self.get(inv, i);
        if stack.is_empty() || amount <= 0 {
            return Stack::empty();
        }
        let taken = stack.split(amount);
        self.set(inv, i, stack);
        taken
    }

    /// `Slot.tryRemove`.
    fn try_remove(
        &mut self,
        inv: &mut Inventory,
        i: usize,
        amount: i32,
        max: i32,
        who: &Who,
    ) -> Option<Stack> {
        if !self.may_pickup(inv, i, who) {
            return None;
        }
        if !self.allow_modification(inv, i, who) && max < self.item(inv, i).count {
            return None;
        }
        let result = self.remove(inv, i, amount.min(max));
        if result.is_empty() {
            return None;
        }
        if self.item(inv, i).is_empty() {
            self.set(inv, i, Stack::empty());
        }
        Some(result)
    }

    /// `Slot.safeTake`.
    fn safe_take(
        &mut self,
        inv: &mut Inventory,
        i: usize,
        amount: i32,
        max: i32,
        who: &Who,
        out: &mut Outcome,
    ) -> Stack {
        match self.try_remove(inv, i, amount, max, who) {
            Some(taken) => {
                self.on_take(inv, i, who, out);
                taken
            }
            None => Stack::empty(),
        }
    }

    /// `Slot.safeInsert`: as much of `input` as fits and `amount` allows goes in;
    /// what is left of it comes back.
    fn safe_insert(
        &mut self,
        inv: &mut Inventory,
        i: usize,
        mut input: Stack,
        amount: i32,
        who: &Who,
    ) -> Stack {
        if input.is_empty() || !self.may_place(i, &input, who) {
            return input;
        }
        let mut here = self.get(inv, i);
        let n = amount
            .min(input.count)
            .min(self.max_stack_size(i, &input, who) - here.count);
        if n <= 0 {
            return input;
        }
        if here.is_empty() {
            let part = input.split(n);
            self.set(inv, i, part);
        } else if here.same_item_same_components(&input) {
            input.shrink(n);
            here.grow(n);
            self.set(inv, i, here);
        }
        input
    }

    /// `Slot.onTake`: taking a crafting result uses one of each ingredient.
    fn on_take(&mut self, inv: &mut Inventory, i: usize, who: &Who, out: &mut Outcome) {
        match self.slots[i].rule {
            Rule::CraftResult => self.take_craft(inv, who, out),
            // `GrindstoneMenu`'s result slot: the inputs go.
            Rule::GrindstoneResult => {
                self.set(inv, 0, Stack::empty());
                self.set(inv, 1, Stack::empty());
            }
            _ => {}
        }
    }

    /// `ResultSlot.onTake` on the client: within the grid's filled rectangle each
    /// ingredient loses one, and what it leaves behind (a bucket, a bottle) takes its
    /// place, joins it, or goes to the inventory (else it is dropped).
    fn take_craft(&mut self, inv: &mut Inventory, who: &Who, out: &mut Outcome) {
        let w = self.grid_width;
        if w == 0 {
            return;
        }
        // `CraftingInput.ofPositioned`.
        let (mut left, mut right, mut top, mut bottom) = (w - 1, 0, w - 1, 0);
        let mut any = false;
        for y in 0..w {
            let mut row = false;
            for x in 0..w {
                if !self.grid[x + y * w].is_empty() {
                    left = left.min(x);
                    right = right.max(x);
                    row = true;
                    any = true;
                }
            }
            if row {
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
        if !any {
            return;
        }
        let (width, height) = (right - left + 1, bottom - top + 1);
        let remaining: Vec<Stack> = (0..width * height)
            .map(|k| {
                let (x, y) = (k % width, k / width);
                crafting_remainder(&self.grid[x + left + (y + top) * w], who.items)
            })
            .collect();
        for y in 0..height {
            for x in 0..width {
                let at = x + left + (y + top) * w;
                let mut replacement = remaining[x + y * width].clone();
                if !self.grid[at].is_empty() {
                    let _ = self.grid[at].split(1);
                }
                let here = self.grid[at].clone();
                if replacement.is_empty() {
                    continue;
                }
                if here.is_empty() {
                    self.grid[at] = replacement;
                } else if here.same_item_same_components(&replacement) {
                    replacement.grow(here.count);
                    self.grid[at] = replacement;
                } else if !inv.add_stack(&mut replacement, who.creative) {
                    drop_stack(out, replacement);
                }
            }
        }
    }

    /// `canTakeItemForPickAll`: not from a crafting result.
    fn can_take_for_pick_all(&self, i: usize) -> bool {
        !(matches!(self.kind, Kind::Inventory | Kind::Crafting)
            && self.slots[i].rule == Rule::CraftResult)
            && self.slots[i].rule != Rule::Creative
    }

    /// Public for the screen.
    pub fn can_take_item_for_pick_all(&self, i: usize) -> bool {
        self.can_take_for_pick_all(i)
    }

    // --- Clicks -------------------------------------------------------------------

    fn reset_quickcraft(&mut self) {
        self.quickcraft_status = 0;
        self.quickcraft_slots.clear();
    }

    /// `clicked` (`doClick`). A slot number the menu doesn't have does nothing (the
    /// client's own screens never click one).
    pub fn clicked(
        &mut self,
        inv: &mut Inventory,
        who: &Who,
        slot: i32,
        button: i32,
        input: Input,
    ) -> Outcome {
        let mut out = Outcome::default();
        self.do_click(inv, who, slot, button, input, &mut out);
        out
    }

    fn index(&self, slot: i32) -> Option<usize> {
        usize::try_from(slot).ok().filter(|&i| i < self.slots.len())
    }

    fn do_click(
        &mut self,
        inv: &mut Inventory,
        who: &Who,
        slot: i32,
        button: i32,
        input: Input,
        out: &mut Outcome,
    ) {
        if input == Input::QuickCraft {
            let expected = self.quickcraft_status;
            self.quickcraft_status = quickcraft_header(button);
            // A header out of turn, or nothing on the cursor, ends the drag.
            if (expected != 1 || self.quickcraft_status != 2) && expected != self.quickcraft_status
                || self.carried.is_empty()
            {
                self.reset_quickcraft();
            } else if self.quickcraft_status == 0 {
                self.quickcraft_type = quickcraft_type(button);
                if valid_quickcraft_type(self.quickcraft_type, who) {
                    self.quickcraft_status = 1;
                    self.quickcraft_slots.clear();
                } else {
                    self.reset_quickcraft();
                }
            } else if self.quickcraft_status == 1 {
                let Some(i) = self.index(slot) else {
                    return;
                };
                let carried = &self.carried;
                if can_item_quick_replace(Some(self.item(inv, i)), carried, true)
                    && self.may_place(i, carried, who)
                    && (self.quickcraft_type == 2
                        || carried.count > self.quickcraft_slots.len() as i32)
                    && !self.quickcraft_slots.contains(&i)
                {
                    self.quickcraft_slots.push(i);
                }
            } else if self.quickcraft_status == 2 {
                if !self.quickcraft_slots.is_empty() {
                    if self.quickcraft_slots.len() == 1 {
                        let only = self.quickcraft_slots[0] as i32;
                        self.reset_quickcraft();
                        self.do_click(inv, who, only, self.quickcraft_type, Input::Pickup, out);
                        return;
                    }
                    let mut source = self.carried.copy();
                    if source.is_empty() {
                        self.reset_quickcraft();
                        return;
                    }
                    let mut remaining = self.carried.count;
                    let n = self.quickcraft_slots.len();
                    for i in self.quickcraft_slots.clone() {
                        let carried = &self.carried;
                        if can_item_quick_replace(Some(self.item(inv, i)), carried, true)
                            && self.may_place(i, carried, who)
                            && (self.quickcraft_type == 2 || carried.count >= n as i32)
                        {
                            let here = self.item(inv, i);
                            let carry = if here.is_empty() { 0 } else { here.count };
                            let max = source
                                .max_stack_size()
                                .min(self.max_stack_size(i, &source, who));
                            let count = (quick_craft_place_count(n, self.quickcraft_type, &source)
                                + carry)
                                .min(max);
                            remaining -= count - carry;
                            let placed = source.copy_with_count(count);
                            self.set(inv, i, placed);
                        }
                    }
                    source.count = remaining;
                    self.carried = source;
                }
                self.reset_quickcraft();
            } else {
                self.reset_quickcraft();
            }
        } else if self.quickcraft_status != 0 {
            self.reset_quickcraft();
        } else if matches!(input, Input::Pickup | Input::QuickMove) && (button == 0 || button == 1)
        {
            let primary = button == 0;
            if slot == SLOT_OUTSIDE {
                if !self.carried.is_empty() {
                    if primary {
                        let all = std::mem::take(&mut self.carried);
                        drop_stack(out, all);
                    } else {
                        let one = self.carried.split(1);
                        drop_stack(out, one);
                    }
                }
            } else if input == Input::QuickMove {
                let Some(i) = self.index(slot) else {
                    return;
                };
                if !self.may_pickup(inv, i, who) {
                    return;
                }
                let mut clicked = self.quick_move_stack(inv, i, who, out);
                while !clicked.is_empty() && self.item(inv, i).same_item(&clicked) {
                    clicked = self.quick_move_stack(inv, i, who, out);
                }
            } else {
                let Some(i) = self.index(slot) else {
                    return;
                };
                let clicked = self.get(inv, i);
                let carried = self.carried.clone();
                if clicked.is_empty() {
                    if !carried.is_empty() {
                        let amount = if primary { carried.count } else { 1 };
                        self.carried = self.safe_insert(inv, i, carried, amount, who);
                    }
                } else if self.may_pickup(inv, i, who) {
                    if carried.is_empty() {
                        let amount = if primary {
                            clicked.count
                        } else {
                            (clicked.count + 1) / 2
                        };
                        if let Some(taken) = self.try_remove(inv, i, amount, i32::MAX, who) {
                            self.carried = taken;
                            self.on_take(inv, i, who, out);
                        }
                    } else if self.may_place(i, &carried, who) {
                        if clicked.same_item_same_components(&carried) {
                            let amount = if primary { carried.count } else { 1 };
                            self.carried = self.safe_insert(inv, i, carried, amount, who);
                        } else if carried.count <= self.max_stack_size(i, &carried, who) {
                            self.carried = clicked;
                            self.set(inv, i, carried);
                        }
                    } else if clicked.same_item_same_components(&carried) {
                        let max = carried.max_stack_size() - carried.count;
                        if let Some(taken) = self.try_remove(inv, i, clicked.count, max, who) {
                            self.carried.grow(taken.count);
                            self.on_take(inv, i, who, out);
                        }
                    }
                }
                // `slot.setChanged()`, whatever happened.
                self.touched(i);
            }
        } else if input == Input::Swap && ((0..9).contains(&button) || button == 40) {
            let Some(i) = self.index(slot) else {
                return;
            };
            let b = button as usize;
            let mut source = inv.get(b).clone();
            let target = self.get(inv, i);
            if source.is_empty() && target.is_empty() {
                return;
            }
            if source.is_empty() {
                if self.may_pickup(inv, i, who) {
                    inv.set(b, target);
                    self.set(inv, i, Stack::empty());
                    self.on_take(inv, i, who, out);
                }
            } else if target.is_empty() {
                if self.may_place(i, &source, who) {
                    let max = self.max_stack_size(i, &source, who);
                    if source.count > max {
                        let part = source.split(max);
                        inv.set(b, source);
                        self.set(inv, i, part);
                    } else {
                        inv.set(b, Stack::empty());
                        self.set(inv, i, source);
                    }
                }
            } else if self.may_pickup(inv, i, who) && self.may_place(i, &source, who) {
                let max = self.max_stack_size(i, &source, who);
                if source.count > max {
                    let part = source.split(max);
                    inv.set(b, source);
                    self.set(inv, i, part);
                    self.on_take(inv, i, who, out);
                    let mut back = target;
                    if !inv.add_stack(&mut back, who.creative) {
                        drop_stack(out, back);
                    }
                } else {
                    inv.set(b, target);
                    self.set(inv, i, source);
                    self.on_take(inv, i, who, out);
                }
            }
        } else if input == Input::Clone && who.creative && self.carried.is_empty() && slot >= 0 {
            let Some(i) = self.index(slot) else {
                return;
            };
            let here = self.item(inv, i);
            if !here.is_empty() {
                // `safeClone`.
                self.carried = here.copy_with_count(here.max_stack_size());
            }
        } else if input == Input::Throw && self.carried.is_empty() && slot >= 0 {
            let Some(i) = self.index(slot) else {
                return;
            };
            let amount = if button == 0 {
                1
            } else {
                self.item(inv, i).count
            };
            let mut stack = self.safe_take(inv, i, amount, i32::MAX, who, out);
            drop_stack(out, stack.clone());
            if button == 1 {
                while !stack.is_empty() && self.item(inv, i).same_item(&stack) {
                    stack = self.safe_take(inv, i, amount, i32::MAX, who, out);
                    drop_stack(out, stack.clone());
                }
            }
        } else if input == Input::PickupAll && slot >= 0 {
            let Some(i) = self.index(slot) else {
                return;
            };
            if !self.carried.is_empty() && (!self.has_item(inv, i) || !self.may_pickup(inv, i, who))
            {
                let n = self.slots.len() as i32;
                let (start, step) = if button == 0 { (0, 1) } else { (n - 1, -1) };
                for pass in 0..2 {
                    let mut t = start;
                    while t >= 0 && t < n && self.carried.count < self.carried.max_stack_size() {
                        let ti = t as usize;
                        if self.has_item(inv, ti)
                            && can_item_quick_replace(Some(self.item(inv, ti)), &self.carried, true)
                            && self.may_pickup(inv, ti, who)
                            && self.can_take_for_pick_all(ti)
                        {
                            let here = self.item(inv, ti);
                            if pass != 0 || here.count != here.max_stack_size() {
                                let count = here.count;
                                let max = self.carried.max_stack_size() - self.carried.count;
                                let removed = self.safe_take(inv, ti, count, max, who, out);
                                self.carried.grow(removed.count);
                            }
                        }
                        t += step;
                    }
                }
            }
        }
    }

    /// `moveItemStackTo`: `stack` (the source slot's, changed in place) goes into
    /// slots `start` to `end`, joining like stacks first, then into the first
    /// empty one that takes it. Whether anything moved.
    fn move_item_stack_to(
        &mut self,
        inv: &mut Inventory,
        stack: &mut Stack,
        start: usize,
        end: usize,
        backwards: bool,
        who: &Who,
    ) -> bool {
        let mut changed = false;
        let order: Vec<usize> = if backwards {
            (start..end).rev().collect()
        } else {
            (start..end).collect()
        };
        if stack.is_stackable() {
            for &d in &order {
                if stack.is_empty() {
                    break;
                }
                let mut target = self.get(inv, d);
                if !target.is_empty() && stack.same_item_same_components(&target) {
                    let total = target.count + stack.count;
                    let max = self.max_stack_size(d, &target, who);
                    if total <= max {
                        stack.count = 0;
                        target.count = total;
                        self.set(inv, d, target);
                        changed = true;
                    } else if target.count < max {
                        stack.shrink(max - target.count);
                        target.count = max;
                        self.set(inv, d, target);
                        changed = true;
                    }
                }
            }
        }
        if !stack.is_empty() {
            for &d in &order {
                if self.item(inv, d).is_empty() && self.may_place(d, stack, who) {
                    let max = self.max_stack_size(d, stack, who);
                    let part = stack.split(stack.count.min(max));
                    self.set(inv, d, part);
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    /// `quickMoveStack` of each kind (shift-clicking slot `i`): the stack moves to
    /// the other part of the menu; what was there is returned (empty when nothing more
    /// can move).
    fn quick_move_stack(
        &mut self,
        inv: &mut Inventory,
        i: usize,
        who: &Who,
        out: &mut Outcome,
    ) -> Stack {
        if !self.has_item(inv, i) {
            return Stack::empty();
        }
        let mut stack = self.get(inv, i);
        let clicked = stack.copy();
        let n = self.slots.len();
        // `ItemPickerMenu.quickMoveStack`: a hotbar slot is emptied, nothing moves.
        if self.kind == Kind::Creative {
            if i + 9 >= n {
                self.set(inv, i, Stack::empty());
            }
            return Stack::empty();
        }
        // (Each branch: where to, and whether failing to move ends it.)
        let moved = match self.kind {
            // Its screen clicks nothing; the creative picker's is above.
            Kind::Placeholder(_) | Kind::Creative => false,

            Kind::Beacon
            | Kind::BrewingStand
            | Kind::Enchantment
            | Kind::Grindstone
            | Kind::Lectern
            | Kind::Mount(_) => self.station_quick_move(inv, &mut stack, i, who),
            Kind::Inventory => {
                let eq = equipment_slot_for_item(&clicked);
                if i == 0 {
                    self.move_item_stack_to(inv, &mut stack, 9, 45, true, who)
                } else if (1..9).contains(&i) {
                    self.move_item_stack_to(inv, &mut stack, 9, 45, false, who)
                } else if let Some(pos) =
                    humanoid_armor_slot(eq).filter(|&p| !self.has_item(inv, p))
                {
                    self.move_item_stack_to(inv, &mut stack, pos, pos + 1, false, who)
                } else if eq == EquipmentSlot::OffHand && !self.has_item(inv, 45) {
                    self.move_item_stack_to(inv, &mut stack, 45, 46, false, who)
                } else if (9..36).contains(&i) {
                    self.move_item_stack_to(inv, &mut stack, 36, 45, false, who)
                } else if (36..45).contains(&i) {
                    self.move_item_stack_to(inv, &mut stack, 9, 36, false, who)
                } else {
                    self.move_item_stack_to(inv, &mut stack, 9, 45, false, who)
                }
            }
            Kind::Chest(_) | Kind::ShulkerBox | Kind::Hopper | Kind::Dispenser => {
                let own = self.own.len();
                if i < own {
                    self.move_item_stack_to(inv, &mut stack, own, n, true, who)
                } else {
                    self.move_item_stack_to(inv, &mut stack, 0, own, false, who)
                }
            }
            Kind::Crafting => {
                if i == 0 {
                    self.move_item_stack_to(inv, &mut stack, 10, 46, true, who)
                } else if (10..46).contains(&i) {
                    self.move_item_stack_to(inv, &mut stack, 1, 10, false, who)
                        || if i < 37 {
                            self.move_item_stack_to(inv, &mut stack, 37, 46, false, who)
                        } else {
                            self.move_item_stack_to(inv, &mut stack, 10, 37, false, who)
                        }
                } else {
                    self.move_item_stack_to(inv, &mut stack, 10, 46, false, who)
                }
            }
            Kind::Furnace | Kind::BlastFurnace | Kind::Smoker => {
                if i == 2 {
                    self.move_item_stack_to(inv, &mut stack, 3, 39, true, who)
                } else if i != 1 && i != 0 {
                    let smelts = self
                        .kind
                        .smelts()
                        .is_some_and(|k| who.recipes.contains(k, &stack));
                    if smelts {
                        self.move_item_stack_to(inv, &mut stack, 0, 1, false, who)
                    } else if stack.c.cooking_fuel {
                        self.move_item_stack_to(inv, &mut stack, 1, 2, false, who)
                    } else if (3..30).contains(&i) {
                        self.move_item_stack_to(inv, &mut stack, 30, 39, false, who)
                    } else {
                        // (Slots 30 to 38: to the inventory; nothing else is left.)
                        self.move_item_stack_to(inv, &mut stack, 3, 30, false, who)
                    }
                } else {
                    self.move_item_stack_to(inv, &mut stack, 3, 39, false, who)
                }
            }
        };
        // The source slot holds the stack as moving left it (changed in place: its
        // container hears of it only below).
        self.set_quiet(inv, i, stack.clone());
        if !moved {
            return Stack::empty();
        }
        // `setByPlayer(EMPTY)`, or `setChanged`.
        if stack.is_empty() {
            self.set(inv, i, Stack::empty());
        } else {
            self.touched(i);
        }
        // Chests, shulker boxes, hoppers and mounts stop here.
        if matches!(
            self.kind,
            Kind::Chest(_) | Kind::ShulkerBox | Kind::Hopper | Kind::Mount(_)
        ) {
            return clicked;
        }
        if stack.count == clicked.count {
            return Stack::empty();
        }
        self.on_take(inv, i, who, out);
        if i == 0 && matches!(self.kind, Kind::Inventory | Kind::Crafting) {
            drop_stack(out, stack);
        }
        clicked
    }

    /// `MultiPlayerGameMode.handleContainerInput`: the click runs here first, then the
    /// server is told of it with the slots it changed and the cursor, hashed, and the
    /// menu's state id. `None` if the click is not for this menu (vanilla logs it and
    /// sends nothing).
    #[allow(clippy::too_many_arguments)]
    pub fn handle_container_input(
        &mut self,
        inv: &mut Inventory,
        who: &Who,
        container_id: i32,
        slot: i32,
        button: i32,
        input: Input,
        registries: &dyn ruthenium_protocol::hashed::SyncedRegistries,
    ) -> Option<(Vec<u8>, Outcome)> {
        if container_id != self.id {
            eprintln!(
                "[menu] ignoring a click in container {container_id}: the player has {}",
                self.id
            );
            return None;
        }
        let before: Vec<Stack> = (0..self.slots.len())
            .map(|i| self.item(inv, i).copy())
            .collect();
        let outcome = self.clicked(inv, who, slot, button, input);
        let changed: Vec<(i16, Option<ruthenium_protocol::hashed::HashedStack>)> = before
            .iter()
            .enumerate()
            .filter(|(i, b)| !b.matches(self.item(inv, *i)))
            .map(|(i, _)| (i as i16, hash(self.item(inv, i), registries)))
            .collect();
        let carried = hash(&self.carried, registries);
        if log_enabled() {
            let slots: Vec<String> = changed
                .iter()
                .map(|(i, _)| format!("{i}={}", describe(self.item(inv, *i as usize))))
                .collect();
            eprintln!(
                "[menu] sent click container {container_id} state {} slot {slot} button {button} {input:?}: changed [{}] carried {}",
                self.state_id,
                slots.join(" "),
                describe(&self.carried)
            );
        }
        let packet = ruthenium_protocol::packets::play::container_click(
            container_id,
            self.state_id,
            slot as i16,
            button as i8,
            input.id(),
            &changed,
            carried.as_ref(),
        );
        Some((packet, outcome))
    }
}

/// Whether the menus' packets are written to stderr (`RUTHENIUM_MENU_LOG` set):
/// every click sent and every container packet that arrives, for live checks.
pub fn log_enabled() -> bool {
    static ON: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("RUTHENIUM_MENU_LOG").is_some());
    *ON
}

/// A stack as `id*count` (`-` when empty), with `+patch` when its patch isn't
/// empty, for the log.
pub fn describe(stack: &Stack) -> String {
    if stack.is_empty() {
        return "-".into();
    }
    let patch = if stack.patch.is_empty() { "" } else { "+patch" };
    format!(
        "{}*{}{patch}",
        stack.c.name.trim_start_matches("minecraft:"),
        stack.count
    )
}

/// `HashedStack.create` of a stack.
fn hash(
    stack: &Stack,
    registries: &dyn ruthenium_protocol::hashed::SyncedRegistries,
) -> Option<ruthenium_protocol::hashed::HashedStack> {
    if stack.is_empty() {
        return None;
    }
    let s = ruthenium_protocol::item::ItemStack {
        item: stack.item,
        count: stack.count,
        patch: (*stack.patch).clone(),
    };
    ruthenium_protocol::hashed::hash_stack(&s, registries)
}

/// `Player.drop` on the client: nothing lands; a stack that isn't empty swings the
/// main hand.
fn drop_stack(out: &mut Outcome, stack: Stack) {
    if !stack.is_empty() {
        out.dropped.push(stack);
    }
}

/// `getQuickcraftHeader`.
pub fn quickcraft_header(mask: i32) -> i32 {
    mask & 3
}

/// `getQuickcraftType`.
pub fn quickcraft_type(mask: i32) -> i32 {
    mask >> 2 & 3
}

/// `getQuickcraftMask`.
pub fn quickcraft_mask(header: i32, kind: i32) -> i32 {
    header & 3 | (kind & 3) << 2
}

/// `isValidQuickcraftType`: copies (2) only with infinite materials.
fn valid_quickcraft_type(kind: i32, who: &Who) -> bool {
    kind == 0 || kind == 1 || kind == 2 && who.creative
}

/// `canItemQuickReplace`: an empty slot, or one of the same stack with room (with
/// `ignore_size`, the carried count doesn't count).
pub fn can_item_quick_replace(here: Option<&Stack>, stack: &Stack, ignore_size: bool) -> bool {
    let empty = here.is_none_or(Stack::is_empty);
    match here {
        Some(h) if !empty && stack.same_item_same_components(h) => {
            h.count + if ignore_size { 0 } else { stack.count } <= stack.max_stack_size()
        }
        _ => empty,
    }
}

/// `getQuickCraftPlaceCount`.
pub fn quick_craft_place_count(slots: usize, kind: i32, stack: &Stack) -> i32 {
    match kind {
        0 => floor_div(stack.count, slots),
        1 => 1,
        2 => stack.max_stack_size(),
        _ => stack.count,
    }
}

/// `LivingEntity.getEquipmentSlotForItem` for a player: its `equippable` slot, else
/// the main hand.
fn equipment_slot_for_item(stack: &Stack) -> EquipmentSlot {
    stack
        .c
        .equippable
        .map_or(EquipmentSlot::MainHand, |e| e.slot)
}

/// The inventory menu's slot of a humanoid armour slot (`8 - getIndex()`).
fn humanoid_armor_slot(eq: EquipmentSlot) -> Option<usize> {
    match eq {
        EquipmentSlot::Feet => Some(8),
        EquipmentSlot::Legs => Some(7),
        EquipmentSlot::Chest => Some(6),
        EquipmentSlot::Head => Some(5),
        _ => None,
    }
}

/// `Slot.getMaxStackSize(stack)` of a slot of `rule`: an armour slot holds one, a
/// fuel slot one bucket, any other as many as the stack allows up to 99.
pub fn rule_max_stack_size(rule: Rule, stack: &Stack) -> i32 {
    let slot = match rule {
        Rule::Armor(_)
        | Rule::MountEquipment(_)
        | Rule::BeaconPayment
        | Rule::Potion
        | Rule::EnchantItem => 1,
        Rule::FurnaceFuel if is_bucket(stack) => return 1,
        _ => CONTAINER_MAX,
    };
    slot.min(stack.max_stack_size())
}

fn is_bucket(stack: &Stack) -> bool {
    !stack.is_empty() && stack.c.name == "minecraft:bucket"
}

/// A shulker box's item (`BlockItem` of a `ShulkerBoxBlock`).
fn is_shulker_box(stack: &Stack) -> bool {
    stack.c.name.ends_with("shulker_box")
}

/// `Item.getCraftingRemainder`: buckets of water, lava and milk leave a bucket,
/// dragon's breath and honey a glass bottle (`Items`' `craftRemainder`).
fn crafting_remainder(stack: &Stack, items: &Items) -> Stack {
    let left = match stack.c.name {
        "minecraft:water_bucket" | "minecraft:lava_bucket" | "minecraft:milk_bucket" => {
            "minecraft:bucket"
        }
        "minecraft:dragon_breath" | "minecraft:honey_bottle" => "minecraft:glass_bottle",
        _ => return Stack::empty(),
    };
    match ruthenium_data::v26_3::registries::Item::from_name(left) {
        Some(item) => items.new_stack(item.id() as i32, 1),
        None => Stack::empty(),
    }
}

#[cfg(test)]
mod golden;

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_data::v26_3::registries::DataComponentType as ComponentKind;
    use ruthenium_data::v26_3::registries::Item;
    use ruthenium_prepare::exporters::ItemClassData;
    use ruthenium_protocol::item::{Component, ComponentPatch, ItemStack};
    use std::collections::HashMap;

    /// No synced registries (the stub hashes need none).
    struct NoRegistries;
    impl ruthenium_protocol::hashed::SyncedRegistries for NoRegistries {
        fn key(&self, _: &str, _: i32) -> Option<&str> {
            None
        }
    }

    /// Items as vanilla has them, for the golden pool.
    fn items() -> Items {
        let mut classes = Vec::new();
        let mut reports = HashMap::new();
        for &(id, max, damage, equip, fuel) in golden::POOL {
            let name = Item::from_id(id as u32).unwrap().name();
            classes.push(ItemClassData {
                id: id as u32,
                key: name.to_owned(),
                classes: vec!["Item".to_owned()],
                flags: 0,
                block: None,
                fields: Vec::new(),
            });
            let mut c = serde_json::Map::new();
            c.insert("minecraft:max_stack_size".into(), max.into());
            if damage > 0 {
                c.insert("minecraft:max_damage".into(), damage.into());
                c.insert("minecraft:damage".into(), 0.into());
            }
            if equip >= 0 {
                let slot = ["mainhand", "feet", "legs", "chest", "head", "offhand"][equip as usize];
                c.insert(
                    "minecraft:equippable".into(),
                    serde_json::json!({ "slot": slot }),
                );
            }
            if fuel {
                c.insert("minecraft:cooking_fuel".into(), serde_json::json!({}));
            }
            reports.insert(name.to_owned(), serde_json::json!({ "components": c }));
        }
        Items::new(&classes, &reports)
    }

    fn id(name: &str) -> i32 {
        Item::from_name(name).unwrap().id() as i32
    }

    fn recipes() -> RecipeSets {
        let mut r = RecipeSets::new();
        r.set(
            "minecraft:furnace_input",
            [id("minecraft:iron_ore"), id("minecraft:oak_log")],
        );
        r.set("minecraft:blast_furnace_input", [id("minecraft:iron_ore")]);
        r.set("minecraft:smoker_input", [id("minecraft:beef")]);
        r
    }

    /// The tool's generator (a 64-bit LCG).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, n: i32) -> i32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as i32
        }
    }

    fn damaged(items: &Items, item: i32, count: i32, damage: i32) -> Stack {
        items.stack(&ItemStack {
            item,
            count,
            patch: ComponentPatch {
                added: vec![(ComponentKind::Damage.id() as i32, Component::Damage(damage))],
                ..ComponentPatch::default()
            },
        })
    }

    fn random_stack(r: &mut Lcg, items: &Items) -> Stack {
        let (item, max, damage, _, _) = golden::POOL[r.next(golden::POOL.len() as i32) as usize];
        let count = 1 + r.next(max);
        if damage > 0 && r.next(2) == 0 {
            let d = (1 + r.next(20)).min(damage);
            return damaged(items, item, count, d);
        }
        items.new_stack(item, count)
    }

    fn describe_golden(s: &Stack) -> String {
        if s.is_empty() {
            return "-".into();
        }
        let p = if s.patch.is_empty() {
            String::new()
        } else {
            format!("p{}", s.c.damage.unwrap_or(0))
        };
        format!("{}*{}{p}", s.item, s.count)
    }

    fn contents(m: &Menu, inv: &Inventory) -> String {
        (0..m.slots.len())
            .filter(|&i| !m.item(inv, i).is_empty())
            .map(|i| format!("{i}={};", describe_golden(m.item(inv, i))))
            .collect()
    }

    const KINDS: [&str; 9] = [
        "inventory",
        "chest",
        "dispenser",
        "shulker",
        "crafting",
        "furnace",
        "blast",
        "smoker",
        "hopper",
    ];

    fn run_case(c: u64, items: &Items, recipes: &RecipeSets) -> String {
        let mut r = Lcg(0x5EED_0000 + c);
        let kind = r.next(9);
        let rows = 1 + r.next(6);
        let creative = r.next(5) == 0;
        let (k, id) = match kind {
            0 => (Kind::Inventory, 0),
            1 => (Kind::Chest(rows as u8), 1),
            2 => (Kind::Dispenser, 1),
            3 => (Kind::ShulkerBox, 1),
            4 => (Kind::Crafting, 1),
            5 => (Kind::Furnace, 1),
            6 => (Kind::BlastFurnace, 1),
            7 => (Kind::Smoker, 1),
            _ => (Kind::Hopper, 1),
        };
        let mut m = Menu::new(k, id);
        let mut inv = Inventory::default();
        let who = Who {
            items,
            recipes,
            creative,
        };
        let n = m.slots.len() as i32;
        let mut out = format!(
            "{}{}{}\n",
            KINDS[kind as usize],
            if kind == 1 { rows } else { 0 },
            if creative { " creative" } else { "" }
        );
        for i in 0..m.slots.len() {
            if r.next(3) == 0 {
                let s = random_stack(&mut r, items);
                m.set(&mut inv, i, s);
            }
        }
        inv.selected = r.next(9);
        if r.next(3) == 0 {
            m.carried = random_stack(&mut r, items);
        }
        m.state_id = r.next(100);
        out += &format!(
            "selected {} state {}\n{}\n",
            inv.selected,
            m.state_id,
            contents(&m, &inv)
        );
        let mut clicks = Vec::new();
        for _ in 0..14 {
            let input = r.next(7);
            if input == 5 && r.next(10) < 7 {
                let t = r.next(3);
                clicks.push((-999, quickcraft_mask(0, t), 5));
                let count = 1 + r.next(4);
                for _ in 0..count {
                    clicks.push((r.next(n), quickcraft_mask(1, t), 5));
                }
                clicks.push((-999, quickcraft_mask(2, t), 5));
                continue;
            }
            let (slot, button) = match input {
                2 => {
                    let slot = r.next(n);
                    let b = r.next(10);
                    (slot, if b == 9 { 40 } else { b })
                }
                5 => {
                    let h = r.next(3);
                    let t = r.next(3);
                    let slot = if h == 1 { r.next(n) } else { -999 };
                    (slot, quickcraft_mask(h, t))
                }
                _ => {
                    let x = r.next(20);
                    let slot = if x == 0 {
                        -999
                    } else if x == 1 && input == 0 {
                        -1
                    } else {
                        r.next(n)
                    };
                    let button = match input {
                        0 | 1 => [0, 0, 1, 1, 2][r.next(5) as usize],
                        3 => r.next(3),
                        _ => r.next(2),
                    };
                    (slot, button)
                }
            };
            clicks.push((slot, button, input));
        }
        for (slot, button, input) in clicks {
            let i = Input::from_id(input).unwrap();
            let (packet, outcome) = m
                .handle_container_input(&mut inv, &who, id, slot, button, i, &NoRegistries)
                .unwrap();
            let hex: String = zero_hashes(&packet[1..])
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            out += &format!(
                "{input}:{slot}:{button}|{}|c={}|d={}|p={hex}\n",
                contents(&m, &inv),
                describe_golden(&m.carried),
                outcome.dropped.len()
            );
        }
        out
    }

    /// The click's body with every component hash 0, as the tool wrote it: the cases
    /// check the packet's layout and order, whatever hashes the stacks get.
    fn zero_hashes(body: &[u8]) -> Vec<u8> {
        let mut out = body.to_vec();
        let mut at = 0;
        let varint = |at: &mut usize| {
            let mut v = 0u32;
            for shift in (0..35).step_by(7) {
                let b = body[*at];
                *at += 1;
                v |= u32::from(b & 0x7F) << shift;
                if b & 0x80 == 0 {
                    break;
                }
            }
            v
        };
        let stack = |at: &mut usize, out: &mut Vec<u8>| {
            let present = body[*at] != 0;
            *at += 1;
            if !present {
                return;
            }
            varint(at);
            varint(at);
            for _ in 0..varint(at) {
                varint(at);
                out[*at..*at + 4].fill(0);
                *at += 4;
            }
            for _ in 0..varint(at) {
                varint(at);
            }
        };
        varint(&mut at);
        varint(&mut at);
        at += 3;
        varint(&mut at);
        for _ in 0..varint(&mut at) {
            at += 2;
            stack(&mut at, &mut out);
        }
        stack(&mut at, &mut out);
        assert_eq!(at, body.len());
        out
    }

    fn fnv(s: &str) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in s.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    #[test]
    fn clicks_do_what_vanillas_menus_do() {
        let (items, recipes) = (items(), recipes());
        // The pool's items are as vanilla has them.
        for &(id, max, damage, equip, fuel) in golden::POOL {
            let s = items.new_stack(id, 1);
            assert_eq!(s.max_stack_size(), max);
            assert_eq!(s.c.max_damage.unwrap_or(0), damage);
            assert_eq!(
                s.c.equippable.map_or(-1, |e| e.slot as i32),
                equip,
                "{}",
                s.c.name
            );
            assert_eq!(s.c.cooking_fuel, fuel);
        }
        let mut wrong = Vec::new();
        for (c, &want) in golden::CASES.iter().enumerate() {
            let got = run_case(c as u64, &items, &recipes);
            if fnv(&got) != want {
                wrong.push(c);
                if wrong.len() == 1 {
                    eprintln!(
                        "case {c} differs (the tool prints vanilla's with its number):\n{got}"
                    );
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{} of {} cases differ: {:?}",
            wrong.len(),
            golden::CASES.len(),
            &wrong[..wrong.len().min(20)]
        );
    }
    #[test]
    fn slots_keep_their_rules() {
        let (items, recipes) = (items(), recipes());
        let who = Who {
            items: &items,
            recipes: &recipes,
            creative: false,
        };
        let mut inv = Inventory::default();
        // A shulker box doesn't go into a shulker box; a bucket is a furnace's fuel, one
        // at a time.
        let shulker = Menu::new(Kind::ShulkerBox, 1);
        let boxed = items.new_stack(id("minecraft:shulker_box"), 1);
        assert!(!shulker.may_place(0, &boxed, &who));
        assert!(shulker.may_place(0, &items.new_stack(id("minecraft:stone"), 1), &who));
        let furnace = Menu::new(Kind::Furnace, 1);
        let buckets = items.new_stack(id("minecraft:bucket"), 16);
        assert!(furnace.may_place(1, &buckets, &who));
        assert_eq!(furnace.max_stack_size(1, &buckets, &who), 1);
        assert!(!furnace.may_place(1, &items.new_stack(id("minecraft:stone"), 1), &who));
        assert!(furnace.may_place(1, &items.new_stack(id("minecraft:coal"), 1), &who));
        assert!(!furnace.may_place(2, &items.new_stack(id("minecraft:coal"), 1), &who));
        // Armour: only its own slot, one at a time; bound armour stays on but in creative.
        let mut m = Menu::inventory();
        let helmet = items.new_stack(id("minecraft:diamond_helmet"), 1);
        assert!(m.may_place(5, &helmet, &who));
        assert!(!m.may_place(6, &helmet, &who));
        assert_eq!(
            m.max_stack_size(5, &items.new_stack(id("minecraft:stone"), 64), &who),
            1
        );
        let mut bound = helmet.clone();
        std::sync::Arc::make_mut(&mut bound.c).prevent_armor_change = true;
        m.set(&mut inv, 5, bound);
        assert!(!m.may_pickup(&inv, 5, &who));
        let creative = Who {
            creative: true,
            ..who
        };
        assert!(m.may_pickup(&inv, 5, &creative));
        m.clicked(&mut inv, &who, 5, 0, Input::Pickup);
        assert!(m.carried.is_empty());
        // Shift-clicking armour puts it on, and the off hand takes a shield.
        let mut m = Menu::inventory();
        let mut inv = Inventory::default();
        m.set(
            &mut inv,
            36,
            items.new_stack(id("minecraft:diamond_boots"), 1),
        );
        m.set(&mut inv, 37, items.new_stack(id("minecraft:shield"), 1));
        m.clicked(&mut inv, &who, 36, 0, Input::QuickMove);
        m.clicked(&mut inv, &who, 37, 0, Input::QuickMove);
        assert_eq!(inv.get(36).c.name, "minecraft:diamond_boots");
        assert_eq!(inv.get(40).c.name, "minecraft:shield");
        // Shift-clicking stone in a 3-row chest's inventory part fills the chest; out
        // of it, the hotbar's last slot first (backwards).
        let mut chest = Menu::new(Kind::Chest(3), 2);
        let mut inv = Inventory::default();
        chest.set(&mut inv, 0, items.new_stack(id("minecraft:stone"), 10));
        chest.clicked(&mut inv, &who, 0, 0, Input::QuickMove);
        assert_eq!(inv.items[8].count, 10);
        chest.clicked(&mut inv, &who, 62, 0, Input::QuickMove);
        assert_eq!(chest.own[0].count, 10);
    }

    #[test]
    fn taking_a_crafting_result_uses_the_grid() {
        let (items, recipes) = (items(), recipes());
        let who = Who {
            items: &items,
            recipes: &recipes,
            creative: false,
        };
        let mut inv = Inventory::default();
        let mut m = Menu::new(Kind::Crafting, 3);
        m.grid[4] = items.new_stack(id("minecraft:milk_bucket"), 1);
        m.grid[5] = items.new_stack(id("minecraft:stone"), 3);
        m.own[0] = items.new_stack(id("minecraft:snowball"), 4);
        m.clicked(&mut inv, &who, 0, 0, Input::Pickup);
        assert_eq!(m.carried.count, 4);
        // The milk leaves its bucket; the stone one less.
        assert_eq!(m.grid[4].c.name, "minecraft:bucket");
        assert_eq!(m.grid[5].count, 2);
        assert!(m.own[0].is_empty());
        // Dragging copies needs infinite materials.
        let mut m = Menu::new(Kind::Chest(1), 1);
        m.carried = items.new_stack(id("minecraft:stone"), 2);
        for (slot, mask) in [
            (-999, quickcraft_mask(0, 2)),
            (0, quickcraft_mask(1, 2)),
            (1, quickcraft_mask(1, 2)),
            (-999, quickcraft_mask(2, 2)),
        ] {
            m.clicked(&mut inv, &who, slot, mask, Input::QuickCraft);
        }
        assert!(m.own[0].is_empty());
        let creative = Who {
            creative: true,
            ..who
        };
        for (slot, mask) in [
            (-999, quickcraft_mask(0, 2)),
            (0, quickcraft_mask(1, 2)),
            (1, quickcraft_mask(1, 2)),
            (-999, quickcraft_mask(2, 2)),
        ] {
            m.clicked(&mut inv, &creative, slot, mask, Input::QuickCraft);
        }
        // (What is left goes below nothing: the cursor is empty.)
        assert_eq!((m.own[0].count, m.own[1].count), (64, 64));
        assert!(m.carried.is_empty());
    }

    #[test]
    fn swaps_and_gathers_mind_their_slots() {
        let (items, recipes) = (items(), recipes());
        let who = Who {
            items: &items,
            recipes: &recipes,
            creative: false,
        };
        // A gather leaves a crafting result alone.
        let mut inv = Inventory::default();
        let mut m = Menu::inventory();
        m.own[0] = items.new_stack(id("minecraft:stone"), 5);
        m.set(&mut inv, 9, items.new_stack(id("minecraft:stone"), 4));
        m.carried = items.new_stack(id("minecraft:stone"), 3);
        m.clicked(&mut inv, &who, 10, 0, Input::PickupAll);
        assert_eq!((m.carried.count, m.own[0].count), (7, 5));
        // Swapping sixteen pumpkins with a helmet on: one goes on, the helmet into the
        // inventory.
        let mut inv = Inventory::default();
        let mut m = Menu::inventory();
        m.set(
            &mut inv,
            5,
            items.new_stack(id("minecraft:diamond_helmet"), 1),
        );
        inv.items[0] = items.new_stack(id("minecraft:carved_pumpkin"), 16);
        m.clicked(&mut inv, &who, 5, 0, Input::Swap);
        assert_eq!(inv.get(39).c.name, "minecraft:carved_pumpkin");
        assert_eq!(inv.items[0].count, 15);
        assert_eq!(inv.items[1].c.name, "minecraft:diamond_helmet");
    }
}
