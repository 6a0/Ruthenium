//! Static registries, from `reports/registries.json`.

use std::fmt::Write as _;

use serde_json::Value;

use super::Generated;
use super::rust;

/// Registries that get a typed enum (named after the registry: `Block`,
/// `EntityType`, ...). Every other static registry is still in
/// `REGISTRIES` by name. Add one here when code needs its ids as a type.
const TYPED: &[&str] = &[
    "minecraft:attribute",
    "minecraft:block",
    "minecraft:block_entity_type",
    "minecraft:command_argument_type",
    "minecraft:consume_effect_type",
    "minecraft:custom_stat",
    "minecraft:data_component_type",
    "minecraft:entity_type",
    "minecraft:fluid",
    "minecraft:game_event",
    "minecraft:item",
    "minecraft:map_decoration_type",
    "minecraft:menu",
    "minecraft:mob_effect",
    "minecraft:number_format_type",
    "minecraft:particle_type",
    "minecraft:position_source_type",
    "minecraft:potion",
    "minecraft:recipe_book_category",
    "minecraft:recipe_display",
    "minecraft:slot_display",
    "minecraft:sound_event",
    "minecraft:stat_type",
    "minecraft:villager_profession",
    "minecraft:villager_type",
];

/// A registry's entries as `(name, id)`, sorted by id and checked to be
/// exactly `0..n`.
pub fn entries(report: &Value, registry: &str) -> Result<Vec<(String, u32)>, String> {
    let what = format!("registries.json: {registry}");
    let map = report[registry]["entries"]
        .as_object()
        .ok_or_else(|| format!("{what}: no entries"))?;
    let mut out = map
        .iter()
        .map(|(name, e)| Ok((name.clone(), rust::u32_of(&e["protocol_id"], &what)?)))
        .collect::<Result<Vec<_>, String>>()?;
    out.sort_by_key(|e| e.1);
    if out.iter().enumerate().any(|(i, e)| e.1 as usize != i) {
        return Err(format!("{what}: ids are not 0..{}", out.len()));
    }
    Ok(out)
}

pub fn generate(report: &Value) -> Result<Generated, String> {
    let all = report.as_object().ok_or("registries.json: not an object")?;
    let mut registries = all
        .iter()
        .map(|(name, r)| Ok((name.as_str(), rust::u32_of(&r["protocol_id"], name)?, r)))
        .collect::<Result<Vec<_>, String>>()?;
    registries.sort_by_key(|r| r.1);
    if registries
        .iter()
        .enumerate()
        .any(|(i, r)| r.1 as usize != i)
    {
        return Err("registries.json: registry ids are not 0..n".into());
    }
    if let Some(missing) = TYPED.iter().find(|t| !all.contains_key(**t)) {
        return Err(format!("registries.json: no {missing} registry"));
    }

    let mut o = String::from("use crate::registry::Registry;\n\n");
    let mut table = String::new();
    let _ = writeln!(
        table,
        "/// Every static registry, by registry id.\npub static REGISTRIES: [Registry; {}] = [",
        registries.len()
    );
    for (name, id, r) in &registries {
        let list = entries(report, name)?;
        let default = match r.get("default") {
            None => None,
            Some(d) => Some(rust::str_of(d, &format!("{name} default"))?),
        };
        if let Some(d) = default
            && !list.iter().any(|(n, _)| n == d)
        {
            return Err(format!(
                "registries.json: {name}: default {d:?} is not an entry"
            ));
        }
        let entries_expr = if TYPED.contains(name) {
            let ty = rust::pascal_case(name).ok_or_else(|| format!("{name} has no Rust name"))?;
            let variants = rust::variants(list, &format!("registries.json: {name}"))?;
            rust::id_enum(
                &mut o,
                "",
                &format!("The `{name}` registry."),
                &ty,
                &variants,
            );
            if let Some(d) = default {
                let v = variants
                    .iter()
                    .find(|v| v.name == d)
                    .expect("checked above");
                let _ = writeln!(
                    o,
                    "\nimpl {ty} {{\n    /// The registry's default entry.\n    pub const DEFAULT: Self = Self::{};\n}}",
                    v.ident
                );
            }
            o.push('\n');
            format!("{ty}::NAMES")
        } else {
            let names: Vec<String> = list.iter().map(|(n, _)| format!("{n:?}")).collect();
            format!("&[{}]", names.join(", "))
        };
        let _ = writeln!(
            table,
            "    Registry {{ name: {name:?}, id: {id}, default: {default:?}, entries: {entries_expr} }},"
        );
    }
    table.push_str("];\n");
    o.push_str(&table);
    o.push_str(
        "\n/// Looks a static registry up by name, e.g. `\"minecraft:item\"`.\npub fn registry(name: &str) -> Option<&'static Registry> {\n    REGISTRIES.iter().find(|r| r.name == name)\n}\n",
    );
    Ok(Generated {
        doc: "Static (built-in) registries: entry names by id.".into(),
        body: o,
    })
}
