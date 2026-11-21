//! Golden traces recorded from the vanilla client by `cargo xtask trace`
//! (format `ruthenium-trace/1`, described in `docs/golden-traces.md`).
//!
//! A trace is JSON Lines: a header, then per client tick the inputs (`tick`),
//! the packets sent and processed, and the player's state after the tick
//! (`state`). This crate reads them; the simulation's replay tests compare
//! against them.
//!
//! Floats are written in Java's shortest round-trip form, so they must be
//! parsed from the text straight to `f32` / `f64`. [`Record::f32`] and
//! [`Record::f64`] do that; going through `f64` first could round an `f32`
//! twice.

#![forbid(unsafe_code)]

use serde_json::Value;

pub const FORMAT: &str = "ruthenium-trace/1";

/// One line of a trace.
#[derive(Debug, Clone)]
pub struct Record {
    /// The record kind: `header`, `frame`, `tick`, `state`, `out`,
    /// `recv`, `handle` or `end`.
    pub kind: String,
    pub value: Value,
}

impl Record {
    /// The client tick the record belongs to (-1 before the first tick).
    pub fn tick(&self) -> Option<i64> {
        self.value["n"].as_i64()
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.value[key].as_str()
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.value[key].as_bool()
    }

    /// A float field, parsed exactly from its text.
    pub fn f32(&self, key: &str) -> Option<f32> {
        number_text(&self.value[key])?.parse().ok()
    }

    /// A double field, parsed exactly from its text.
    pub fn f64(&self, key: &str) -> Option<f64> {
        number_text(&self.value[key])?.parse().ok()
    }

    /// A vector of doubles (`pos`, `vel`).
    pub fn vec3(&self, key: &str) -> Option<[f64; 3]> {
        let a = self.value[key].as_array()?;
        let mut out = [0.0; 3];
        for (o, v) in out.iter_mut().zip(a) {
            *o = number_text(v)?.parse().ok()?;
        }
        (a.len() == 3).then_some(out)
    }

    /// A packet's bytes: the packet id (VarInt) and its payload, as on the
    /// wire before framing, compression and encryption.
    pub fn bytes(&self) -> Option<Vec<u8>> {
        let hex = self.str("hex")?;
        if hex.len() % 2 != 0 {
            return None;
        }
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
            .collect()
    }

    /// The packet id at the start of [`Record::bytes`].
    pub fn packet_id(&self) -> Option<u32> {
        let bytes = self.bytes()?;
        let mut value = 0u32;
        for (i, b) in bytes.iter().take(5).enumerate() {
            value |= u32::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
}

/// The number exactly as written (serde_json's `arbitrary_precision` keeps
/// the text instead of converting to f64).
fn number_text(v: &Value) -> Option<String> {
    v.as_number().map(|n| n.to_string())
}

/// A parsed trace.
#[derive(Debug, Clone)]
pub struct Trace {
    pub header: Value,
    pub records: Vec<Record>,
}

impl Trace {
    pub fn parse(text: &str) -> Result<Trace, String> {
        let mut lines = text.lines().enumerate();
        let (_, first) = lines.next().ok_or("empty trace")?;
        let header: Value = serde_json::from_str(first).map_err(|e| format!("line 1: {e}"))?;
        if header["k"] != "header" || header["format"] != FORMAT {
            return Err(format!("line 1 is not a {FORMAT} header"));
        }
        let mut records = Vec::new();
        for (i, line) in lines {
            let value: Value =
                serde_json::from_str(line).map_err(|e| format!("line {}: {e}", i + 1))?;
            let kind = value["k"]
                .as_str()
                .ok_or_else(|| format!("line {}: no record kind", i + 1))?
                .to_owned();
            records.push(Record { kind, value });
        }
        Ok(Trace { header, records })
    }

    pub fn scenario(&self) -> &str {
        self.header["scenario"].as_str().unwrap_or("")
    }

    /// Records of one kind, in order.
    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Record> {
        self.records.iter().filter(move |r| r.kind == kind)
    }
}
