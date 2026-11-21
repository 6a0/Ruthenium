//! Helpers for emitting Rust source.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

/// One enum variant: Rust identifier, numeric id and the game's name for it.
#[derive(Debug)]
pub struct Variant {
    pub ident: String,
    pub id: u32,
    pub name: String,
}

/// Turns `(name, id)` pairs into variants sorted by id. Fails unless the ids
/// are exactly `0..n` and every name maps to its own Rust identifier.
pub fn variants(mut entries: Vec<(String, u32)>, what: &str) -> Result<Vec<Variant>, String> {
    if entries.is_empty() {
        return Err(format!("{what}: no entries"));
    }
    entries.sort_by_key(|e| e.1);
    let n = entries.len();
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut out = Vec::with_capacity(n);
    for (i, (name, id)) in entries.into_iter().enumerate() {
        if id as usize != i {
            return Err(format!(
                "{what}: ids are not 0..{n} (expected {i}, found {id} for {name:?})"
            ));
        }
        let ident =
            pascal_case(&name).ok_or_else(|| format!("{what}: {name:?} has no Rust name"))?;
        if let Some(other) = seen.insert(ident.clone(), name.clone()) {
            return Err(format!(
                "{what}: {other:?} and {name:?} both become {ident}"
            ));
        }
        out.push(Variant { ident, id, name });
    }
    Ok(out)
}

/// `minecraft:set_entity_motion` → `SetEntityMotion`. Other namespaces are
/// kept as a prefix: `brigadier:bool` → `BrigadierBool`.
pub fn pascal_case(name: &str) -> Option<String> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let mut out = String::new();
    for word in name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.extend(chars);
        }
    }
    let valid = out.starts_with(|c: char| c.is_ascii_alphabetic()) && out != "Self";
    valid.then_some(out)
}

/// `waterlogged` → `WATERLOGGED`.
pub fn screaming_snake(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Emits an `id_enum!` invocation (defined in ruthenium-data). Its body is
/// not touched by rustfmt, so it is laid out here.
pub fn id_enum(o: &mut String, indent: &str, doc: &str, ty: &str, variants: &[Variant]) {
    let repr = match variants.len() {
        n if n <= 1 << 8 => "u8",
        n if n <= 1 << 16 => "u16",
        _ => "u32",
    };
    let _ = writeln!(o, "{indent}crate::id_enum! {{");
    let _ = writeln!(o, "{indent}    /// {doc}");
    let _ = writeln!(o, "{indent}    pub enum {ty}: {repr} {{");
    for v in variants {
        let _ = writeln!(o, "{indent}        {} = {} => {:?},", v.ident, v.id, v.name);
    }
    let _ = writeln!(o, "{indent}    }}");
    let _ = writeln!(o, "{indent}}}");
}

pub fn u32_of(v: &Value, what: &str) -> Result<u32, String> {
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("{what}: expected a number, found {v}"))
}

pub fn str_of<'a>(v: &'a Value, what: &str) -> Result<&'a str, String> {
    v.as_str()
        .ok_or_else(|| format!("{what}: expected a string, found {v}"))
}

/// Formats generated source with the workspace's rustfmt settings.
pub fn format(root: &Path, source: &str) -> Result<String, String> {
    let mut child = Command::new("rustfmt")
        .arg("--config-path")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("running rustfmt: {e}"))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = source.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let out = child
        .wait_with_output()
        .map_err(|e| format!("running rustfmt: {e}"))?;
    writer
        .join()
        .expect("rustfmt writer thread")
        .map_err(|e| format!("writing to rustfmt: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "rustfmt rejected the generated code:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("rustfmt output: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            pascal_case("minecraft:set_entity_motion").unwrap(),
            "SetEntityMotion"
        );
        assert_eq!(
            pascal_case("minecraft:block.anvil.break").unwrap(),
            "BlockAnvilBreak"
        );
        assert_eq!(pascal_case("brigadier:bool").unwrap(), "BrigadierBool");
        assert_eq!(pascal_case("minecraft:3d"), None);
        assert_eq!(pascal_case("minecraft:self"), None);
        assert_eq!(screaming_snake("has_bottle_0"), "HAS_BOTTLE_0");
    }

    #[test]
    fn variants_must_be_dense_and_distinct() {
        let v = variants(
            vec![("minecraft:b".into(), 1), ("minecraft:a".into(), 0)],
            "t",
        )
        .unwrap();
        assert_eq!(v[0].ident, "A");
        assert_eq!(v[1].id, 1);
        assert!(variants(vec![("minecraft:a".into(), 1)], "t").is_err());
        assert!(
            variants(
                vec![("minecraft:a_b".into(), 0), ("minecraft:a.b".into(), 1)],
                "t"
            )
            .is_err()
        );
    }
}
