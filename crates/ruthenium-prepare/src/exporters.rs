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

/// Default size of an entity type.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityDimensions {
    /// Network (registry) id.
    pub id: u32,
    /// e.g. `minecraft:player`.
    pub key: String,
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
    /// The size ignores the entity's scale.
    pub fixed: bool,
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
pub fn read_entity_dimensions(bytes: &[u8]) -> Result<Vec<EntityDimensions>, String> {
    let mut r = Reader::open(bytes, "entity_dimensions", 1)?;
    let count = r.u32()?;
    let mut out = Vec::with_capacity(count.min(4096) as usize);
    for _ in 0..count {
        out.push(EntityDimensions {
            id: r.u32()?,
            key: r.string()?,
            width: r.f32()?,
            height: r.f32()?,
            eye_height: r.f32()?,
            fixed: r.u8()? != 0,
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
    pub offset: Option<OffsetData>,
}

impl StatePhysics {
    pub const LARGE_SHAPE: u8 = 1;
    pub const DYNAMIC_SHAPE: u8 = 2;
    /// The shape depends on who collides (liquids, powder snow, scaffolding, moving
    /// pistons).
    pub const CONTEXT_SHAPE: u8 = 4;
    pub const OFFSET: u8 = 8;
    /// `isSuffocating` at the origin.
    pub const SUFFOCATING: u8 = 16;
}

/// `block_physics.bin`: every block state's collision shape and movement factors.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockPhysics {
    pub shapes: Vec<ShapeData>,
    pub states: Vec<StatePhysics>,
}

/// Parses `block_physics.bin`.
pub fn read_block_physics(bytes: &[u8]) -> Result<BlockPhysics, String> {
    let mut r = Reader::open(bytes, "block_physics", 2)?;
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
        let offset = if flags & StatePhysics::OFFSET != 0 {
            Some(OffsetData {
                vertical: r.u8()? == 2,
                max_horizontal: r.f32()?,
                max_vertical: r.f32()?,
            })
        } else {
            None
        };
        states.push(StatePhysics {
            shape,
            flags,
            friction,
            speed_factor,
            jump_factor,
            bounce,
            offset,
        });
    }
    r.finish()?;
    Ok(BlockPhysics { shapes, states })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as u16).to_le_bytes());
        out.extend(s.as_bytes());
    }

    #[test]
    fn reads_entity_dimensions() {
        let mut b = b"RTHX".to_vec();
        b.extend(1u32.to_le_bytes());
        string(&mut b, "entity_dimensions");
        b.extend(1u32.to_le_bytes());
        b.extend(7u32.to_le_bytes());
        string(&mut b, "example:thing");
        for f in [0.5f32, 1.25, 1.0] {
            b.extend(f.to_bits().to_le_bytes());
        }
        b.push(1);
        let dims = read_entity_dimensions(&b).unwrap();
        assert_eq!(
            dims,
            [EntityDimensions {
                id: 7,
                key: "example:thing".into(),
                width: 0.5,
                height: 1.25,
                eye_height: 1.0,
                fixed: true,
            }]
        );
        assert!(read_entity_dimensions(&b[..b.len() - 1]).is_err());
        let mut extra = b.clone();
        extra.push(0);
        assert!(read_entity_dimensions(&extra).is_err());
    }
}
