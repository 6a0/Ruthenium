//! Profiles whose skin is shown: a player's as the player list has it, a head's owner, a
//! mannequin's (`ResolvableProfile`).

use crate::nbt::{Compound, Tag};
use crate::packets::Property;
use crate::wire::{Reader, WireError};

/// The longest property value.
const MAX_STRING: usize = 32767;

/// What of a skin a profile replaces: pictures by their id in the resource packs (below
/// `textures`, without the suffix), and whether the arms are the thin ones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct SkinPatch {
    pub body: Option<String>,
    pub cape: Option<String>,
    pub elytra: Option<String>,
    pub slim: Option<bool>,
}

/// A profile, maybe in parts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ResolvableProfile {
    pub id: Option<u128>,
    pub name: Option<String>,
    pub properties: Vec<Property>,
    pub patch: SkinPatch,
}

/// What a profile that names nothing else is looked up by.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Lookup {
    Name(String),
    Id(u128),
}

impl ResolvableProfile {
    /// What has to be looked up before its skin is known: its name or its id, where it
    /// has only one of them and no properties.
    pub fn lookup(&self) -> Option<Lookup> {
        if !self.properties.is_empty() {
            return None;
        }
        match (&self.id, &self.name) {
            (None, Some(name)) => Some(Lookup::Name(name.clone())),
            (Some(id), None) => Some(Lookup::Id(*id)),
            _ => None,
        }
    }

    /// The first of its properties that holds its skin.
    pub fn textures(&self) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == "textures")
    }

    /// As entity data and item components carry it: whole (with an id and a name) or in
    /// parts, its properties, and the patch.
    pub fn read(r: &mut Reader<'_>) -> Result<ResolvableProfile, WireError> {
        let mut p = ResolvableProfile::default();
        if r.bool()? {
            p.id = Some(r.uuid()?);
            p.name = Some(r.string(16)?.to_owned());
        } else {
            if r.bool()? {
                p.name = Some(r.string(16)?.to_owned());
            }
            if r.bool()? {
                p.id = Some(r.uuid()?);
            }
        }
        p.properties = Property::read_list(r)?;
        let mut texture = || -> Result<Option<String>, WireError> {
            Ok(if r.bool()? {
                Some(r.identifier()?)
            } else {
                None
            })
        };
        p.patch.body = texture()?;
        p.patch.cape = texture()?;
        p.patch.elytra = texture()?;
        if r.bool()? {
            p.patch.slim = Some(r.bool()?);
        }
        Ok(p)
    }

    /// As a block entity's data holds it: a name, or a compound. `None` for what is
    /// neither.
    pub fn from_nbt(tag: &Tag) -> Option<ResolvableProfile> {
        let c = match tag {
            Tag::String(name) => {
                return valid_name(name).then(|| ResolvableProfile {
                    name: Some(name.clone()),
                    ..ResolvableProfile::default()
                });
            }
            Tag::Compound(c) => c,
            _ => return None,
        };
        let name = match c.get("name") {
            Some(Tag::String(n)) if valid_name(n) => Some(n.clone()),
            None => None,
            Some(_) => return None,
        };
        let id = match c.get("id") {
            Some(Tag::IntArray(a)) if a.len() == 4 => {
                Some(a.iter().fold(0u128, |n, i| n << 32 | u128::from(*i as u32)))
            }
            None => None,
            Some(_) => return None,
        };
        let texture = |key: &str| c.string(key).map(str::to_owned);
        Some(ResolvableProfile {
            id,
            name,
            properties: properties(c),
            patch: SkinPatch {
                body: texture("texture"),
                cape: texture("cape"),
                elytra: texture("elytra"),
                slim: match c.string("model") {
                    Some("slim") => Some(true),
                    Some("wide") => Some(false),
                    _ => None,
                },
            },
        })
    }
}

/// Whether a name is one a player can have (`ExtraCodecs.PLAYER_NAME`): at most 16
/// characters, none of them a space or a control.
fn valid_name(name: &str) -> bool {
    name.encode_utf16().count() <= 16 && name.bytes().all(|b| b > 32 && b < 127)
}

/// A compound's `properties`: a list of them, or a compound of their values by name.
fn properties(c: &Compound) -> Vec<Property> {
    match c.get("properties") {
        Some(Tag::List(list)) => list
            .iter()
            .filter_map(|p| {
                let p = p.as_compound()?;
                Some(Property {
                    name: p.string("name")?.to_owned(),
                    value: p.string("value")?.to_owned(),
                    signature: p.string("signature").map(str::to_owned),
                })
            })
            .collect(),
        Some(Tag::Compound(by_name)) => by_name
            .iter()
            .filter_map(|(name, values)| match values {
                Tag::List(values) => Some((name, values)),
                _ => None,
            })
            .flat_map(|(name, values)| {
                values.iter().filter_map(move |v| match v {
                    Tag::String(value) => Some(Property {
                        name: name.to_owned(),
                        value: value.clone(),
                        signature: None,
                    }),
                    _ => None,
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl Property {
    /// `ByteBufCodecs.GAME_PROFILE_PROPERTIES`.
    pub(crate) fn read_list(r: &mut Reader<'_>) -> Result<Vec<Property>, WireError> {
        let n = r.count(16)?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(Property {
                name: r.string(64)?.to_owned(),
                value: r.string(MAX_STRING)?.to_owned(),
                signature: if r.bool()? {
                    Some(r.string(1024)?.to_owned())
                } else {
                    None
                },
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire;

    #[test]
    fn a_profile_in_parts_is_read_from_the_wire() {
        let mut b = vec![0, 1];
        wire::write_string(&mut b, "Alex");
        b.push(0);
        wire::write_varint(&mut b, 1);
        wire::write_string(&mut b, "textures");
        wire::write_string(&mut b, "e30=");
        b.push(1);
        wire::write_string(&mut b, "c2ln");
        b.push(1);
        wire::write_string(&mut b, "minecraft:entity/player/wide/kai");
        b.extend([0, 0, 1, 1, 99]);
        let mut r = Reader::new(&b);
        let p = ResolvableProfile::read(&mut r).unwrap();
        assert_eq!((p.id, p.name.as_deref()), (None, Some("Alex")));
        assert_eq!(p.textures().unwrap().signature.as_deref(), Some("c2ln"));
        assert_eq!(
            p.patch.body.as_deref(),
            Some("minecraft:entity/player/wide/kai")
        );
        assert_eq!((p.patch.cape, p.patch.slim), (None, Some(true)));
        // Its properties are there: nothing to look up.
        assert_eq!(r.rest(), [99]);
        let mut whole = vec![1];
        wire::write_uuid(&mut whole, 5);
        wire::write_string(&mut whole, "Kai");
        whole.extend([0, 0, 0, 0, 0]);
        let p = ResolvableProfile::read(&mut Reader::new(&whole)).unwrap();
        assert_eq!((p.id, p.lookup()), (Some(5), None));
    }

    #[test]
    fn a_heads_owner_is_read_from_its_data() {
        let name = ResolvableProfile::from_nbt(&Tag::String("Steve".into())).unwrap();
        assert_eq!(name.lookup(), Some(Lookup::Name("Steve".into())));
        assert_eq!(
            ResolvableProfile::from_nbt(&Tag::String("not a name".into())),
            None
        );
        let mut c = Compound::new();
        c.put("id", Tag::IntArray(vec![1, -1, 0, 2]));
        c.put("model", Tag::String("slim".into()));
        let id = 1u128 << 96 | 0xFFFF_FFFF << 64 | 2;
        let by_id = ResolvableProfile::from_nbt(&Tag::Compound(c.clone())).unwrap();
        assert_eq!(by_id.lookup(), Some(Lookup::Id(id)));
        assert_eq!(by_id.patch.slim, Some(true));
        let mut property = Compound::new();
        property.put("name", Tag::String("textures".into()));
        property.put("value", Tag::String("e30=".into()));
        c.put("properties", Tag::List(vec![Tag::Compound(property)]));
        let with = ResolvableProfile::from_nbt(&Tag::Compound(c.clone())).unwrap();
        assert_eq!(with.lookup(), None);
        assert_eq!(with.textures().unwrap().value, "e30=");
        let mut by_name = Compound::new();
        by_name.put("textures", Tag::List(vec![Tag::String("e30=".into())]));
        c.put("properties", Tag::Compound(by_name));
        let with = ResolvableProfile::from_nbt(&Tag::Compound(c)).unwrap();
        assert_eq!(with.textures().unwrap().value, "e30=");
        assert_eq!(ResolvableProfile::from_nbt(&Tag::Int(1)), None);
    }
}
