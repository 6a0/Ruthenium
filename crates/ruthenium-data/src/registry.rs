//! Static registries as name tables.

/// A static (built-in) registry: entry names indexed by id.
#[derive(Debug)]
pub struct Registry {
    /// E.g. `minecraft:item`.
    pub name: &'static str,
    /// The registry's own id in the registry of registries.
    pub id: u32,
    /// The entry used for unknown ids, for registries that have one.
    pub default: Option<&'static str>,
    /// Entry names; `entries[id]` is the entry with that id.
    pub entries: &'static [&'static str],
}

impl Registry {
    /// The name of the entry with this id.
    pub fn name_of(&self, id: u32) -> Option<&'static str> {
        self.entries.get(id as usize).copied()
    }

    /// The id of the entry with this name (a linear search; typed registries
    /// have a faster `from_name`).
    pub fn id_of(&self, name: &str) -> Option<u32> {
        self.entries
            .iter()
            .position(|e| *e == name)
            .map(|i| i as u32)
    }
}
