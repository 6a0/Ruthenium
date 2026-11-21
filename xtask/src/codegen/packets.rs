//! Packet ids per protocol state and direction, from `reports/packets.json`.

use std::fmt::Write as _;

use serde_json::Value;

use super::Generated;
use super::rust;

/// Protocol states in connection order.
const STATES: &[&str] = &["handshake", "status", "login", "configuration", "play"];
const DIRECTIONS: &[(&str, &str)] = &[
    ("clientbound", "Clientbound"),
    ("serverbound", "Serverbound"),
];

pub fn generate(report: &Value) -> Result<Generated, String> {
    let states = report.as_object().ok_or("packets.json: not an object")?;
    if let Some(unknown) = states.keys().find(|k| !STATES.contains(&k.as_str())) {
        return Err(format!("packets.json: unknown protocol state {unknown:?}"));
    }
    let mut o = String::new();
    for state in STATES {
        let dirs = states
            .get(*state)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("packets.json: no {state:?} state"))?;
        if let Some(unknown) = dirs
            .keys()
            .find(|k| !DIRECTIONS.iter().any(|(d, _)| d == k))
        {
            return Err(format!(
                "packets.json: {state}: unknown direction {unknown:?}"
            ));
        }
        let _ = writeln!(
            o,
            "/// Packet ids of the `{state}` protocol state.\npub mod {state} {{"
        );
        for (dir, ty) in DIRECTIONS {
            let Some(packets) = dirs.get(*dir) else {
                continue;
            };
            let what = format!("packets.json: {state} {dir}");
            let packets = packets
                .as_object()
                .ok_or_else(|| format!("{what}: not an object"))?;
            let entries = packets
                .iter()
                .map(|(name, p)| Ok((name.clone(), rust::u32_of(&p["protocol_id"], &what)?)))
                .collect::<Result<Vec<_>, String>>()?;
            let variants = rust::variants(entries, &what)?;
            let doc = format!("{ty} packets of the `{state}` state, by wire id.");
            rust::id_enum(&mut o, "    ", &doc, ty, &variants);
        }
        let _ = writeln!(o, "}}\n");
    }
    Ok(Generated {
        doc: "Packet ids, per protocol state and direction.".into(),
        body: o,
    })
}
