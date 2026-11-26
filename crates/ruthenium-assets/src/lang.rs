//! Language files (`lang/<code>.json`): a flat object of strings, as
//! `locale/Language.loadFromJson` reads one.

use std::collections::HashMap;

use serde_json::Value;

use crate::resources::Resource;

/// One language file's entries, no more than its pack's limits allow
/// ([`crate::Limits::lang_entries`], [`crate::Limits::json_bytes`]). Vanilla fails the
/// whole file (it is skipped with a warning) when it is not an object or a value is not a
/// string; so does this, and when it has more entries than the limit.
pub fn read(file: &Resource<'_>) -> Result<HashMap<String, String>, String> {
    let max = file.limits().lang_entries;
    let Value::Object(map) = file.json()? else {
        return Err("not a JSON object".into());
    };
    if map.len() > max {
        return Err(format!(
            "{} entries is more than a language file may have ({max})",
            map.len()
        ));
    }
    map.into_iter()
        .map(|(k, v)| match v {
            Value::String(s) => Ok((k, s)),
            other => Err(format!("{k}: {other} is not a string")),
        })
        .collect()
}
