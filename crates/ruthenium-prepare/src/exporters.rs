//! Running Ruthenium's own Java exporters against the imported client jar,
//! and reading what they write.
//!
//! The exporter bundle (`ruthenium-exporters.jar`, built by `cargo xtask
//! exporters`) holds only original `dev/ruthenium/` classes and ships next to
//! the Ruthenium executable. Its output goes to `<cache>/versions/<id>/exports`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::datagen::{StepError, run_logged};
use crate::hash::sha256_file;
use crate::import::Imported;
use crate::java::Java;
use crate::step;

pub const BUNDLE_NAME: &str = "ruthenium-exporters.jar";
const MAIN_CLASS: &str = "dev.ruthenium.exporters.Main";
pub const ENTITY_DIMENSIONS: &str = "entity_dimensions.bin";
pub const BLOCK_PHYSICS: &str = "block_physics.bin";
pub const ITEM_CLASSES: &str = "item_classes.bin";

/// The exporter bundle: `explicit`, then `RUTHENIUM_EXPORTERS`, then next to
/// the executable, then `../java/` from it (`target/java/` in a dev build).
pub fn find_bundle(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_owned());
    }
    if let Some(p) = std::env::var_os("RUTHENIUM_EXPORTERS").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join(BUNDLE_NAME),
        dir.parent()?.join("java").join(BUNDLE_NAME),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

pub fn exports_dir(imported: &Imported) -> PathBuf {
    imported.dir.join("exports")
}

fn key(imported: &Imported, bundle: &Path) -> Result<String, StepError> {
    let bundle_hash = sha256_file(bundle)?;
    Ok(step::key(&["exporters-v1", &imported.key(), &bundle_hash]))
}

/// Whether the exports for this import and bundle already exist.
pub fn is_current(imported: &Imported, bundle: &Path) -> bool {
    key(imported, bundle).is_ok_and(|k| step::is_current(&exports_dir(imported), &k))
}

/// Runs the exporters unless their output for these inputs already exists.
pub fn run(java: &Java, imported: &Imported, bundle: &Path) -> Result<(PathBuf, bool), StepError> {
    let out = exports_dir(imported);
    let bundle = std::path::absolute(bundle)?;
    let key = key(imported, &bundle)?;
    let ran = step::run(&out, &key, |tmp| {
        let mut class_path = imported.class_path();
        class_path.push(bundle.clone());
        let class_path = std::env::join_paths(class_path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        run_logged(
            Command::new(&java.path)
                .arg("-cp")
                .arg(class_path)
                .arg(MAIN_CLASS)
                .arg(tmp),
            imported,
            "exporters",
        )?;
        for file in [ENTITY_DIMENSIONS, BLOCK_PHYSICS] {
            let expected = tmp.join(file);
            if !expected.is_file() {
                return Err(StepError::MissingOutput(expected));
            }
        }
        Ok(())
    })?;
    Ok((out, ran))
}

/// What the exporter knows of an entity type (`entity_dimensions` v2).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityTypeData {
    /// Network (registry) id.
    pub id: u32,
    /// e.g. `minecraft:player`.
    pub key: String,
    /// The type's default size (`EntityType.getDimensions`).
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
    /// The size ignores the entity's scale.
    pub fixed: bool,
    /// Ticks between the server's position updates (`EntityType.updateInterval`),
    /// `i32::MAX` for none.
    pub update_interval: i32,
    /// The simple names of the type's class and its superclasses up to `Entity`,
    /// nearest first; empty if the exporter couldn't tell the class.
    pub classes: Vec<String>,
    /// The synched-data fields those classes declare: `Class.FIELD`, the id and the
    /// serializer's id.
    pub fields: Vec<(String, u8, u8)>,
    /// The nearest static `BABY_DIMENSIONS` of those classes: width, height, fixed.
    pub baby: Option<(f32, f32, bool)>,
}

impl EntityTypeData {
    /// Whether the type's class is or extends `class` (a simple name).
    pub fn is(&self, class: &str) -> bool {
        self.classes.iter().any(|c| c == class)
    }

    /// The id of the synched-data field `name` (`Class.FIELD`).
    pub fn field(&self, name: &str) -> Option<u8> {
        self.fields.iter().find(|f| f.0 == name).map(|f| f.1)
    }

    /// The id of the nearest class's field called `field` (just the field's name).
    pub fn nearest_field(&self, field: &str) -> Option<u8> {
        self.classes
            .iter()
            .find_map(|c| self.field(&format!("{c}.{field}")))
    }
}

/// Reads export files: `RTHX`, u32 version, name, then the payload. All
/// little-endian; strings are a u16 length and UTF-8.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn open(bytes: &'a [u8], name: &str, version: u32) -> Result<Self, String> {
        let mut r = Reader { bytes, pos: 0 };
        if r.take(4)? != b"RTHX" {
            return Err("not a Ruthenium export file".into());
        }
        let v = r.u32()?;
        let n = r.string()?;
        if n != name || v != version {
            return Err(format!("expected {name} v{version}, found {n} v{v}"));
        }
        Ok(r)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.bytes.len());
        let end = end.ok_or_else(|| format!("truncated at byte {}", self.pos))?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn f64(&mut self) -> Result<f64, String> {
        let lo = u64::from(self.u32()?);
        let hi = u64::from(self.u32()?);
        Ok(f64::from_bits(lo | hi << 32))
    }

    fn string(&mut self) -> Result<String, String> {
        let len = u16::from_le_bytes(self.take(2)?.try_into().expect("2 bytes"));
        String::from_utf8(self.take(len.into())?.to_vec()).map_err(|e| e.to_string())
    }

    fn finish(&self) -> Result<(), String> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(format!("{} trailing bytes", self.bytes.len() - self.pos))
        }
    }
}

/// Parses `entity_dimensions.bin`.
pub fn read_entity_types(bytes: &[u8]) -> Result<Vec<EntityTypeData>, String> {
    let mut r = Reader::open(bytes, "entity_dimensions", 2)?;
    let count = r.u32()?;
    let mut out = Vec::with_capacity(count.min(4096) as usize);
    for _ in 0..count {
        let id = r.u32()?;
        let key = r.string()?;
        let (width, height, eye_height) = (r.f32()?, r.f32()?, r.f32()?);
        let fixed = r.u8()? != 0;
        let update_interval = r.u32()? as i32;
        let n = r.u8()?;
        let mut classes = Vec::with_capacity(n.into());
        for _ in 0..n {
            classes.push(r.string()?);
        }
        let n = r.u32()?;
        let mut fields = Vec::with_capacity(n.min(256) as usize);
        for _ in 0..n {
            fields.push((r.string()?, r.u8()?, r.u8()?));
        }
        let baby = match r.u8()? {
            0 => None,
            _ => Some((r.f32()?, r.f32()?, r.u8()? != 0)),
        };
        out.push(EntityTypeData {
            id,
            key,
            width,
            height,
            eye_height,
            fixed,
            update_interval,
            classes,
            fields,
            baby,
        });
    }
    r.finish()?;
    Ok(out)
}

/// A collision shape as the game stores it: the coordinates of its grid on each axis
/// (one more than the cells) and which cells are full.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeData {
    /// x, y and z coordinates.
    pub coords: [Vec<f64>; 3],
    /// Full cells, x outermost and z innermost.
    pub full: Vec<bool>,
}

/// How a block's position offsets its shape (`BlockBehaviour.OffsetType`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffsetData {
    pub vertical: bool,
    pub max_horizontal: f32,
    pub max_vertical: f32,
}

/// A block state's physics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatePhysics {
    /// Index into [`BlockPhysics::shapes`]; 0 is the empty shape.
    pub shape: u32,
    pub flags: u8,
    pub friction: f32,
    pub speed_factor: f32,
    pub jump_factor: f32,
    /// `BlockBehaviour.bounceRestitution` (slime blocks and beds bounce).
    pub bounce: f32,
    /// The state's fluid (`getFluidState`): bits 0-2 the type (`FLUID_*`), bit 3
    /// falling, bits 4-7 the amount.
    pub fluid: u8,
    /// `isFaceSturdy` at the origin, one bit per `Direction` ordinal (down, up, north,
    /// south, west, east).
    pub sturdy: u8,
    pub offset: Option<OffsetData>,
    /// Index into [`BlockPhysics::shapes`] of the outline shape (`getShape`, what the
    /// crosshair picks).
    pub outline: u32,
    /// Index of the interaction shape (`getInteractionShape`), which can change the
    /// face a pick reports.
    pub interaction_shape: u32,
    /// Whether the block's class overrides `useWithoutItem` ([`Self::USE_WITHOUT_ITEM`])
    /// and `useItemOn` ([`Self::USE_ITEM_ON`]).
    pub interaction: u8,
}

impl StatePhysics {
    pub const USE_WITHOUT_ITEM: u8 = 1;
    pub const USE_ITEM_ON: u8 = 2;

    pub const LARGE_SHAPE: u8 = 1;
    pub const DYNAMIC_SHAPE: u8 = 2;
    /// The shape depends on who collides (liquids, powder snow, scaffolding, moving
    /// pistons).
    pub const CONTEXT_SHAPE: u8 = 4;
    pub const OFFSET: u8 = 8;
    /// `isSuffocating` at the origin.
    pub const SUFFOCATING: u8 = 16;
    /// The block is an `IceBlock` (ice, frosted ice).
    pub const ICE: u8 = 32;

    pub const FLUID_NONE: u8 = 0;
    pub const FLUID_WATER: u8 = 1;
    pub const FLUID_FLOWING_WATER: u8 = 2;
    pub const FLUID_LAVA: u8 = 3;
    pub const FLUID_FLOWING_LAVA: u8 = 4;
}

/// `block_physics.bin`: every block state's collision shape and movement factors.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockPhysics {
    pub shapes: Vec<ShapeData>,
    pub states: Vec<StatePhysics>,
}

/// Parses `block_physics.bin`.
pub fn read_block_physics(bytes: &[u8]) -> Result<BlockPhysics, String> {
    let mut r = Reader::open(bytes, "block_physics", 4)?;
    let n = r.u32()? as usize;
    let mut shapes = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let mut coords: [Vec<f64>; 3] = Default::default();
        for axis in &mut coords {
            let len = r.u32()? as usize;
            if len == 0 || len > 1024 {
                return Err(format!("bad coordinate count {len}"));
            }
            *axis = (0..len).map(|_| r.f64()).collect::<Result<_, _>>()?;
        }
        let cells = (coords[0].len() - 1) * (coords[1].len() - 1) * (coords[2].len() - 1);
        let packed = r.take(cells.div_ceil(8))?;
        let full = (0..cells)
            .map(|i| packed[i / 8] >> (i % 8) & 1 != 0)
            .collect();
        shapes.push(ShapeData { coords, full });
    }
    let n = r.u32()? as usize;
    let mut states = Vec::with_capacity(n.min(1 << 17));
    for _ in 0..n {
        let shape = r.u32()?;
        if shape as usize >= shapes.len() {
            return Err(format!("shape {shape} out of range"));
        }
        let flags = r.u8()?;
        let (friction, speed_factor, jump_factor) = (r.f32()?, r.f32()?, r.f32()?);
        let bounce = r.f32()?;
        let (fluid, sturdy) = (r.u8()?, r.u8()?);
        if fluid & 7 > StatePhysics::FLUID_FLOWING_LAVA {
            return Err(format!("bad fluid {fluid}"));
        }
        let offset = if flags & StatePhysics::OFFSET != 0 {
            Some(OffsetData {
                vertical: r.u8()? == 2,
                max_horizontal: r.f32()?,
                max_vertical: r.f32()?,
            })
        } else {
            None
        };
        let outline = r.u32()?;
        if outline as usize >= shapes.len() {
            return Err(format!("outline shape {outline} out of range"));
        }
        let interaction_shape = r.u32()?;
        if interaction_shape as usize >= shapes.len() {
            return Err(format!(
                "interaction shape {interaction_shape} out of range"
            ));
        }
        let interaction = r.u8()?;
        states.push(StatePhysics {
            shape,
            flags,
            friction,
            speed_factor,
            jump_factor,
            bounce,
            fluid,
            sturdy,
            offset,
            outline,
            interaction_shape,
            interaction,
        });
    }
    r.finish()?;
    Ok(BlockPhysics { shapes, states })
}

/// What the exporter knows of an item's class (`item_classes` v1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemClassData {
    /// Network (registry) id.
    pub id: u32,
    /// e.g. `minecraft:bow`.
    pub key: String,
    /// The simple names of the item's class and its superclasses up to `Item`,
    /// nearest first.
    pub classes: Vec<String>,
    /// [`Self::USE`], [`Self::USE_ON`], [`Self::INTERACT_LIVING_ENTITY`],
    /// [`Self::RELEASE_USING`] (a class below `Item` declares the method) and
    /// [`Self::BLOCK_ITEM`].
    pub flags: u8,
}

impl ItemClassData {
    pub const USE: u8 = 1;
    pub const USE_ON: u8 = 2;
    pub const INTERACT_LIVING_ENTITY: u8 = 4;
    pub const RELEASE_USING: u8 = 8;
    pub const BLOCK_ITEM: u8 = 16;

    /// Whether the item's class is or extends `class` (a simple name).
    pub fn is(&self, class: &str) -> bool {
        self.classes.iter().any(|c| c == class)
    }
}

/// Parses `item_classes.bin`.
pub fn read_item_classes(bytes: &[u8]) -> Result<Vec<ItemClassData>, String> {
    let mut r = Reader::open(bytes, "item_classes", 1)?;
    let count = r.u32()?;
    let mut out = Vec::with_capacity(count.min(8192) as usize);
    for _ in 0..count {
        let id = r.u32()?;
        let key = r.string()?;
        let n = r.u8()?;
        let classes = (0..n).map(|_| r.string()).collect::<Result<_, _>>()?;
        let flags = r.u8()?;
        out.push(ItemClassData {
            id,
            key,
            classes,
            flags,
        });
    }
    r.finish()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as u16).to_le_bytes());
        out.extend(s.as_bytes());
    }

    #[test]
    fn reads_item_classes() {
        let mut b = b"RTHX".to_vec();
        b.extend(1u32.to_le_bytes());
        string(&mut b, "item_classes");
        b.extend(1u32.to_le_bytes());
        b.extend(9u32.to_le_bytes());
        string(&mut b, "example:bow");
        b.push(3);
        for c in ["BowItem", "ProjectileWeaponItem", "Item"] {
            string(&mut b, c);
        }
        b.push(ItemClassData::USE | ItemClassData::RELEASE_USING);
        let items = read_item_classes(&b).unwrap();
        assert_eq!(items[0].key, "example:bow");
        assert!(items[0].is("ProjectileWeaponItem") && !items[0].is("BlockItem"));
        assert_eq!(items[0].flags, 9);
        assert!(read_item_classes(&b[..b.len() - 1]).is_err());
    }

    #[test]
    fn reads_entity_types() {
        let mut b = b"RTHX".to_vec();
        b.extend(2u32.to_le_bytes());
        string(&mut b, "entity_dimensions");
        b.extend(1u32.to_le_bytes());
        b.extend(7u32.to_le_bytes());
        string(&mut b, "example:thing");
        for f in [0.5f32, 1.25, 1.0] {
            b.extend(f.to_bits().to_le_bytes());
        }
        b.push(1);
        b.extend(3u32.to_le_bytes());
        b.push(2);
        string(&mut b, "Thing");
        string(&mut b, "Entity");
        b.extend(1u32.to_le_bytes());
        string(&mut b, "Entity.DATA_POSE");
        b.extend([6, 20]);
        b.push(1);
        for f in [0.25f32, 0.5] {
            b.extend(f.to_bits().to_le_bytes());
        }
        b.push(0);
        let types = read_entity_types(&b).unwrap();
        assert_eq!(
            types,
            [EntityTypeData {
                id: 7,
                key: "example:thing".into(),
                width: 0.5,
                height: 1.25,
                eye_height: 1.0,
                fixed: true,
                update_interval: 3,
                classes: vec!["Thing".into(), "Entity".into()],
                fields: vec![("Entity.DATA_POSE".into(), 6, 20)],
                baby: Some((0.25, 0.5, false)),
            }]
        );
        assert!(types[0].is("Entity") && !types[0].is("LivingEntity"));
        assert_eq!(types[0].nearest_field("DATA_POSE"), Some(6));
        assert!(read_entity_types(&b[..b.len() - 1]).is_err());
        let mut extra = b.clone();
        extra.push(0);
        assert!(read_entity_types(&extra).is_err());
    }
}
