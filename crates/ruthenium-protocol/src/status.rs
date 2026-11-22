//! The status response a server sends the server list (`network/protocol/status/
//! ServerStatus.java`): its description, players, version and favicon, as JSON.

use base64::Engine;
use serde_json::Value;

use crate::text::StyledText;

/// The longest status response string, in UTF-16 units.
pub const MAX_RESPONSE: usize = 32767;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Players {
    pub max: i32,
    pub online: i32,
    /// Names from the sample, in order.
    pub sample: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub name: String,
    pub protocol: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    pub description: StyledText,
    pub players: Option<Players>,
    pub version: Option<Version>,
    /// PNG bytes.
    pub favicon: Option<Vec<u8>>,
    pub enforces_secure_chat: bool,
}

const FAVICON_PREFIX: &str = "data:image/png;base64,";

/// The anonymous player's id, shown as "Anonymous Player" in vanilla's tooltip.
pub const ANONYMOUS_ID: &str = "00000000-0000-0000-0000-000000000000";

impl ServerStatus {
    /// Parses the response's JSON. Missing or malformed optional parts are left out; a
    /// response that is not a JSON object is an error.
    pub fn parse(json: &str) -> Result<ServerStatus, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| format!("status JSON: {e}"))?;
        let o = v.as_object().ok_or("status is not a JSON object")?;
        let int = |v: &Value, k: &str| v.get(k).and_then(Value::as_i64).map(|n| n as i32);
        let players = o.get("players").and_then(|p| {
            Some(Players {
                max: int(p, "max")?,
                online: int(p, "online")?,
                sample: p
                    .get("sample")
                    .and_then(Value::as_array)
                    .map(|s| {
                        s.iter()
                            .filter_map(|e| Some(e.get("name")?.as_str()?.to_owned()))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        });
        let version = o.get("version").and_then(|v| {
            Some(Version {
                name: v.get("name")?.as_str()?.to_owned(),
                protocol: int(v, "protocol")?,
            })
        });
        let favicon = o
            .get("favicon")
            .and_then(Value::as_str)
            .and_then(|s| s.strip_prefix(FAVICON_PREFIX))
            .and_then(|b64| {
                let clean: String = b64.chars().filter(|&c| c != '\n').collect();
                base64::engine::general_purpose::STANDARD.decode(clean).ok()
            });
        Ok(ServerStatus {
            description: o
                .get("description")
                .map(StyledText::from_json)
                .unwrap_or_default(),
            players,
            version,
            favicon,
            enforces_secure_chat: o
                .get("enforcesSecureChat")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

/// Width and height from a PNG's header, if `bytes` starts like a PNG.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.get(..8)? != SIGNATURE || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_typical_response() {
        let json = r#"{"version":{"name":"Paper 26.3","protocol":777},
            "players":{"max":100,"online":3,"sample":[{"name":"Alex","id":"0541ed27-7a6e-4bb3-a0a4-1c56a1a3a9b5"}]},
            "description":{"text":"","extra":[{"text":"Hello","color":"gold"}]},
            "favicon":"data:image/png;base64,iVBO\nRw0KGgo=",
            "enforcesSecureChat":true}"#;
        let s = ServerStatus::parse(json).unwrap();
        assert_eq!(s.description.plain(), "Hello");
        assert_eq!(
            s.players,
            Some(Players {
                max: 100,
                online: 3,
                sample: vec!["Alex".into()]
            })
        );
        assert_eq!(s.version.unwrap().protocol, 777);
        assert_eq!(s.favicon.unwrap()[..4], [0x89, b'P', b'N', b'G']);
        assert!(s.enforces_secure_chat);
    }

    #[test]
    fn tolerates_missing_parts() {
        let s = ServerStatus::parse(r#"{"description":"§bA plain MOTD"}"#).unwrap();
        assert_eq!(s.description.plain(), "A plain MOTD");
        assert_eq!(s.players, None);
        assert_eq!(s.version, None);
        assert!(ServerStatus::parse("[]").is_err());
        assert!(ServerStatus::parse("{").is_err());
    }

    #[test]
    fn reads_png_sizes() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&64u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((64, 64)));
        assert_eq!(png_size(&png[..20]), None);
    }
}
