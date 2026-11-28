//! The creative inventory's tabs (`creative_tabs` v1, `CreativeTabsExporter`): what
//! the client's `CreativeModeTabs` put in each tab that doesn't come from the server's
//! registries, with the places where those parts go, and vanilla's own entries of the
//! registries they read, for those the server sends without data.

use super::Reader;

/// One tab of `BuiltInRegistries.CREATIVE_MODE_TAB`, in registry order.
#[derive(Debug, Clone, PartialEq)]
pub struct CreativeTabData {
    /// e.g. `minecraft:building_blocks`.
    pub key: String,
    /// 0 the top row, 1 the bottom one.
    pub row: u8,
    pub column: u8,
    /// `CreativeModeTab.Type`: 0 category, 1 inventory, 2 hotbar, 3 search.
    pub kind: u8,
    /// [`Self::ALIGNED_RIGHT`], [`Self::SHOWS_TITLE`], [`Self::SCROLLS`].
    pub flags: u8,
    /// The title, a text component as network NBT.
    pub title: Vec<u8>,
    /// `textures/gui/container/creative_inventory/tab_<name>.png`, as an identifier.
    pub background: String,
    /// The icon's item (a registry id).
    pub icon: u32,
    /// The entries are there only with the operator's permission
    /// (`ItemDisplayParameters.hasPermissions`).
    pub operator: bool,
    pub entries: Vec<CreativeEntryData>,
}

impl CreativeTabData {
    pub const ALIGNED_RIGHT: u8 = 1;
    pub const SHOWS_TITLE: u8 = 2;
    pub const SCROLLS: u8 = 4;
}

/// What a tab's generator gives, in its order.
#[derive(Debug, Clone, PartialEq)]
pub enum CreativeEntryData {
    /// A stack of one: the item, its patch (`DataComponentPatch.STREAM_CODEC`), where it
    /// shows (`TabVisibility`: 0 both, 1 the tab, 2 the search tab), the feature flags
    /// it needs besides its item's (a potion's), and its tooltip lines (network NBT).
    Stack {
        item: u32,
        patch: Vec<u8>,
        visibility: u8,
        features: Vec<String>,
        lines: Vec<Vec<u8>>,
    },
    /// Every enchantment's book at its highest level for the tab, and at every level
    /// for the search tab.
    EnchantedBooks,
    /// The paintings of the painting variant registry: those of `#placeable`, or the
    /// others.
    Paintings { placeable: bool },
    /// The instruments of a tag on an item (goat horns).
    Instruments { item: u32, tag: String },
    /// The ominous banner: its patch without the patterns, where they go in it, where
    /// it shows, its tooltip lines and its patterns (banner pattern key, dye colour id).
    OminousBanner {
        item: u32,
        patch: Vec<u8>,
        at: u8,
        visibility: u8,
        lines: Vec<Vec<u8>>,
        patterns: Vec<(String, u8)>,
    },
}

/// A painting variant as vanilla's data pack has it.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintingData {
    pub key: String,
    pub width: u8,
    pub height: u8,
    pub title: Option<Vec<u8>>,
    pub author: Option<Vec<u8>>,
}

/// The whole export.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CreativeTabsData {
    /// Each item's required feature flags, by registry id.
    pub item_features: Vec<Vec<String>>,
    pub tabs: Vec<CreativeTabData>,
    /// Vanilla's enchantments: key, highest level, description (network NBT).
    pub enchantments: Vec<(String, u32, Vec<u8>)>,
    pub paintings: Vec<PaintingData>,
    /// Vanilla's instruments: key, description (network NBT).
    pub instruments: Vec<(String, Vec<u8>)>,
}

impl Reader<'_> {
    /// A u32 byte count and that many bytes.
    fn blob(&mut self) -> Result<Vec<u8>, String> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }

    fn blobs(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let n = self.u32()?;
        (0..n).map(|_| self.blob()).collect()
    }

    fn names(&mut self) -> Result<Vec<String>, String> {
        let n = self.u8()?;
        (0..n).map(|_| self.string()).collect()
    }
}

pub fn read_creative_tabs(bytes: &[u8]) -> Result<CreativeTabsData, String> {
    let mut r = Reader::open(bytes, "creative_tabs", 1)?;
    let items = r.u32()?;
    let item_features = (0..items).map(|_| r.names()).collect::<Result<_, _>>()?;
    let n = r.u32()?;
    let mut tabs = Vec::with_capacity(n.min(64) as usize);
    for _ in 0..n {
        let key = r.string()?;
        let (row, column, kind, flags) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        let title = r.blob()?;
        let background = r.string()?;
        let icon = r.u32()?;
        let operator = r.u8()? != 0;
        let count = r.u32()?;
        let mut entries = Vec::with_capacity(count.min(4096) as usize);
        for _ in 0..count {
            entries.push(match r.u8()? {
                0 => CreativeEntryData::Stack {
                    item: r.u32()?,
                    patch: r.blob()?,
                    visibility: r.u8()?,
                    features: r.names()?,
                    lines: r.blobs()?,
                },
                1 => CreativeEntryData::EnchantedBooks,
                2 => CreativeEntryData::Paintings {
                    placeable: r.u8()? != 0,
                },
                3 => CreativeEntryData::Instruments {
                    item: r.u32()?,
                    tag: r.string()?,
                },
                4 => {
                    let item = r.u32()?;
                    let patch = r.blob()?;
                    let at = r.u8()?;
                    let visibility = r.u8()?;
                    let lines = r.blobs()?;
                    let n = r.u8()?;
                    let patterns = (0..n)
                        .map(|_| Ok((r.string()?, r.u8()?)))
                        .collect::<Result<_, String>>()?;
                    CreativeEntryData::OminousBanner {
                        item,
                        patch,
                        at,
                        visibility,
                        lines,
                        patterns,
                    }
                }
                k => return Err(format!("unknown creative entry kind {k}")),
            });
        }
        tabs.push(CreativeTabData {
            key,
            row,
            column,
            kind,
            flags,
            title,
            background,
            icon,
            operator,
            entries,
        });
    }
    let n = r.u32()?;
    let enchantments = (0..n)
        .map(|_| Ok((r.string()?, r.u32()?, r.blob()?)))
        .collect::<Result<_, String>>()?;
    let n = r.u32()?;
    let mut paintings = Vec::with_capacity(n.min(1024) as usize);
    for _ in 0..n {
        let key = r.string()?;
        let (width, height) = (r.u8()?, r.u8()?);
        let title = if r.u8()? != 0 { Some(r.blob()?) } else { None };
        let author = if r.u8()? != 0 { Some(r.blob()?) } else { None };
        paintings.push(PaintingData {
            key,
            width,
            height,
            title,
            author,
        });
    }
    let n = r.u32()?;
    let instruments = (0..n)
        .map(|_| Ok((r.string()?, r.blob()?)))
        .collect::<Result<_, String>>()?;
    r.finish()?;
    Ok(CreativeTabsData {
        item_features,
        tabs,
        enchantments,
        paintings,
        instruments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The export the bundle writes (`RUTHENIUM_CREATIVE_TABS`: its path) reads whole.
    #[test]
    fn a_real_export_reads() {
        let Some(path) = std::env::var_os("RUTHENIUM_CREATIVE_TABS") else {
            return;
        };
        let bytes = std::fs::read(path).expect("the export");
        let data = read_creative_tabs(&bytes).expect("it reads");
        assert_eq!(data.tabs.len(), 14);
        assert!(data.enchantments.len() > 30);
    }
}
