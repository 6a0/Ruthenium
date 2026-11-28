//! Saved hotbars (`HotbarManager`, `Hotbar`): nine of them, each the nine stacks of a
//! hotbar, kept in `hotbar.nbt` in the data folder as vanilla keeps it in the game's
//! folder: an uncompressed compound with `DataVersion` and the hotbars under "0" to
//! "8", each a list of nine stacks, an empty one written `{}`.
//!
//! A stack is written as vanilla's codec writes its item and count (`id`, `count`);
//! its components go as the bytes the server's codec reads (`ruthenium:patch`), with
//! each entry of the server's registries those bytes name (`ruthenium:registries`:
//! registry, network id and key). The client can only re-encode components it can
//! hash (`crate::hashed` walks every component as its persistent codec does), so a
//! stack that can't be hashed is saved empty, as vanilla saves one its codec can't
//! write; and one whose registry entries have another id or name on this server loads
//! empty, as vanilla loads one its codec can't read. **Deviation:** vanilla writes the
//! components themselves (`components`), so a file vanilla wrote loads here only for
//! its stacks without components, and vanilla reads this one's without theirs.
//!
//! The manager is the client's (`Minecraft.getHotbarManager`): loaded the first time
//! a hotbar is asked for, written whenever one is saved.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use ruthenium_protocol::hashed::{SyncedRegistries, try_hash_stack};
use ruthenium_protocol::item::ItemStack;
use ruthenium_protocol::nbt::{self, Compound, Tag};
use ruthenium_protocol::wire::{self, Reader};

/// `Inventory.getSelectionSize` and `HotbarManager.NUM_HOTBAR_GROUPS`.
pub const SIZE: usize = 9;

const PATCH: &str = "ruthenium:patch";
const REGISTRIES: &str = "ruthenium:registries";

/// A saved stack: its compound as written (`{}` for the empty stack).
pub type Saved = Compound;

/// `Hotbar`: nine saved stacks.
#[derive(Debug, Clone, PartialEq)]
pub struct Hotbar {
    items: Vec<Saved>,
}

impl Default for Hotbar {
    fn default() -> Hotbar {
        Hotbar {
            items: vec![Compound::new(); SIZE],
        }
    }
}

/// The registries as hashing reads them, noting each entry asked for.
struct Recording<'a> {
    inner: &'a dyn SyncedRegistries,
    seen: RefCell<Vec<(String, i32, String)>>,
}

impl SyncedRegistries for Recording<'_> {
    fn key(&self, registry: &str, id: i32) -> Option<&str> {
        let key = self.inner.key(registry, id)?;
        let mut seen = self.seen.borrow_mut();
        if !seen.iter().any(|(r, i, _)| r == registry && *i == id) {
            seen.push((registry.to_owned(), id, key.to_owned()));
        }
        Some(key)
    }
}

/// `ItemStack.OPTIONAL_CODEC.encodeStart` as written here; `None` if it can't be.
pub fn save_stack(stack: &ItemStack, registries: &dyn SyncedRegistries) -> Option<Saved> {
    let mut c = Compound::new();
    if stack.is_empty() {
        return Some(c);
    }
    let item = u32::try_from(stack.item)
        .ok()
        .and_then(ruthenium_data::v26_3::registries::Item::from_id)?;
    let rec = Recording {
        inner: registries,
        seen: RefCell::new(Vec::new()),
    };
    try_hash_stack(stack, &rec).ok()?;
    // The patch as the server writes it: the delimited form, its lengths taken out
    // (each value as written, from its bytes when it still reads as them).
    let mut untrusted = Vec::new();
    stack.write_untrusted(&mut untrusted).ok()?;
    let trusted = trusted_patch(&untrusted)?;
    c.put("id", Tag::String(item.name().to_owned()));
    c.put("count", Tag::Int(stack.count));
    if !stack.patch.is_empty() {
        c.put(
            PATCH,
            Tag::ByteArray(trusted.iter().map(|&b| b as i8).collect()),
        );
        let refs = rec
            .seen
            .into_inner()
            .into_iter()
            .map(|(registry, id, key)| {
                let mut e = Compound::new();
                e.put("registry", Tag::String(registry));
                e.put("id", Tag::Int(id));
                e.put("key", Tag::String(key));
                Tag::Compound(e)
            })
            .collect();
        c.put(REGISTRIES, Tag::List(refs));
    }
    Some(c)
}

/// The patch of a stack written untrusted (`count item added removed (type length
/// value)* removed*`), as the server sends a patch (no lengths).
fn trusted_patch(untrusted: &[u8]) -> Option<Vec<u8>> {
    let mut r = Reader::new(untrusted);
    let _count = r.varint().ok()?;
    let _item = r.varint().ok()?;
    let added = r.varint().ok()?;
    let removed = r.varint().ok()?;
    let mut out = Vec::new();
    wire::write_varint(&mut out, added);
    wire::write_varint(&mut out, removed);
    for _ in 0..added {
        let kind = r.varint().ok()?;
        let len = usize::try_from(r.varint().ok()?).ok()?;
        wire::write_varint(&mut out, kind);
        out.extend_from_slice(r.bytes(len).ok()?);
    }
    for _ in 0..removed {
        wire::write_varint(&mut out, r.varint().ok()?);
    }
    r.finish().ok()?;
    Some(out)
}

/// `ItemStack.OPTIONAL_CODEC.parse` of a saved stack: empty if it can't be read here.
pub fn load_stack(c: &Saved, registries: &dyn SyncedRegistries) -> ItemStack {
    if c.is_empty() {
        return ItemStack::EMPTY;
    }
    let read = || -> Option<ItemStack> {
        let item = ruthenium_data::v26_3::registries::Item::from_name(c.string("id")?)?;
        let count = c.int("count")?;
        if count <= 0 || count > 99 {
            return None;
        }
        let mut body = Vec::new();
        wire::write_varint(&mut body, count);
        wire::write_varint(&mut body, item.id() as i32);
        match c.get(PATCH) {
            Some(Tag::ByteArray(b)) => body.extend(b.iter().map(|&x| x as u8)),
            Some(_) => return None,
            // Components only in vanilla's own form: not read here.
            None if c.get("components").is_some() => return None,
            None => body.extend_from_slice(&[0, 0]),
        }
        for e in c.list(REGISTRIES) {
            let e = e.as_compound()?;
            let here = registries.key(e.string("registry")?, e.int("id")?);
            if here != Some(e.string("key")?) {
                return None;
            }
        }
        let mut r = Reader::new(&body);
        let stack = ItemStack::read_optional(&mut r).ok()?;
        r.finish().ok()?;
        Some(stack)
    };
    read().unwrap_or(ItemStack::EMPTY)
}

impl Hotbar {
    /// `Hotbar.isEmpty`: every stack the empty one.
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(Compound::is_empty)
    }

    /// `Hotbar.load`.
    pub fn load(&self, registries: &dyn SyncedRegistries) -> Vec<ItemStack> {
        self.items
            .iter()
            .map(|c| load_stack(c, registries))
            .collect()
    }

    /// `Hotbar.storeFrom`: the inventory's first nine.
    pub fn store_from(&mut self, stacks: &[ItemStack], registries: &dyn SyncedRegistries) {
        self.items = (0..SIZE)
            .map(|i| {
                stacks
                    .get(i)
                    .and_then(|s| save_stack(s, registries))
                    .unwrap_or_default()
            })
            .collect();
    }

    fn from_tag(tag: Option<&Tag>) -> Hotbar {
        // `Util.fixedSize`: nine or it doesn't parse.
        match tag {
            Some(Tag::List(items)) if items.len() == SIZE => {
                let items: Option<Vec<Compound>> =
                    items.iter().map(|t| t.as_compound().cloned()).collect();
                items.map_or_else(Hotbar::default, |items| Hotbar { items })
            }
            _ => Hotbar::default(),
        }
    }
}

/// `HotbarManager`.
#[derive(Debug, Default)]
pub struct HotbarManager {
    path: Option<PathBuf>,
    hotbars: Vec<Hotbar>,
    loaded: bool,
}

impl HotbarManager {
    pub fn new(path: Option<PathBuf>) -> HotbarManager {
        HotbarManager {
            path,
            hotbars: vec![Hotbar::default(); SIZE],
            loaded: false,
        }
    }

    fn load(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        match nbt::read(&bytes) {
            Ok(root) => {
                for (i, h) in self.hotbars.iter_mut().enumerate() {
                    *h = Hotbar::from_tag(root.get(&i.to_string()));
                }
            }
            Err(e) => eprintln!("[creative] couldn't read {}: {e:?}", path.display()),
        }
    }

    /// `save`.
    pub fn save(&mut self) {
        if self.path.is_none() {
            return;
        }
        let mut root = Compound::new();
        root.put(
            "DataVersion",
            Tag::Int(ruthenium_data::v26_3::version::WORLD_VERSION),
        );
        for i in 0..SIZE {
            let items = self
                .get(i)
                .items
                .iter()
                .cloned()
                .map(Tag::Compound)
                .collect();
            root.put(&i.to_string(), Tag::List(items));
        }
        let Some(path) = &self.path else {
            return;
        };
        match nbt::write(&root) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(path, bytes) {
                    eprintln!("[creative] couldn't save {}: {e}", path.display());
                }
            }
            Err(e) => eprintln!("[creative] couldn't write the hotbars: {e:?}"),
        }
    }

    /// `get`: loaded the first time.
    pub fn get(&mut self, i: usize) -> &mut Hotbar {
        if !self.loaded {
            self.load();
            self.loaded = true;
        }
        &mut self.hotbars[i.min(SIZE - 1)]
    }
}

/// The client's hotbar manager.
pub static HOTBARS: Mutex<Option<HotbarManager>> = Mutex::new(None);

/// Where the saved hotbars are kept (`<data folder>/hotbar.nbt`); until this is called
/// they are kept for the run only.
pub fn set_path(path: Option<PathBuf>) {
    *HOTBARS.lock().unwrap_or_else(|e| e.into_inner()) = Some(HotbarManager::new(path));
}

/// Runs `f` on the client's hotbar manager.
pub fn with<R>(f: impl FnOnce(&mut HotbarManager) -> R) -> R {
    let mut m = HOTBARS.lock().unwrap_or_else(|e| e.into_inner());
    f(m.get_or_insert_with(|| HotbarManager::new(None)))
}

/// Keys by network id, from the configuration's registries.
pub type Keys = HashMap<String, Vec<String>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Keys {
        let mut k = Keys::new();
        k.insert(
            "minecraft:enchantment".into(),
            vec!["minecraft:sharpness".into(), "minecraft:protection".into()],
        );
        k
    }

    fn book(level: u8) -> ItemStack {
        // An enchanted book (1395) with stored_enchantments (45): sharpness (0).
        let body = [1, 0xF3, 0x0A, 1, 0, 45, 1, 0, level];
        ItemStack::read_optional(&mut Reader::new(&body)).expect("reads")
    }

    #[test]
    fn a_stack_saves_and_loads_while_its_registry_entries_stay() {
        let k = keys();
        let saved = save_stack(&book(5), &k).expect("saves");
        assert_eq!(saved.string("id"), Some("minecraft:enchanted_book"));
        assert_eq!(load_stack(&saved, &k), book(5));
        // Another server's sharpness has another id: the stack loads empty.
        let mut other = Keys::new();
        other.insert(
            "minecraft:enchantment".into(),
            vec!["minecraft:protection".into(), "minecraft:sharpness".into()],
        );
        assert!(load_stack(&saved, &other).is_empty());
        assert!(save_stack(&ItemStack::EMPTY, &k).expect("empty").is_empty());
    }

    #[test]
    fn hotbars_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("ruthenium-hotbars-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("hotbar.nbt");
        let k = keys();
        let mut m = HotbarManager::new(Some(path.clone()));
        assert!(m.get(3).is_empty());
        m.get(3).store_from(&[book(2), ItemStack::EMPTY], &k);
        m.save();
        let mut again = HotbarManager::new(Some(path.clone()));
        let loaded = again.get(3).load(&k);
        assert_eq!(loaded.len(), SIZE);
        assert_eq!(loaded[0], book(2));
        assert!(loaded[1..].iter().all(ItemStack::is_empty));
        assert!(again.get(4).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
