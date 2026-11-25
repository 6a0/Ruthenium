//! What a profile's textures property says: where its skin and cape are, and whether
//! Mojang's services signed it. And the profile of a name or an id, for a head or a
//! mannequin that gives no more.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rsa::pkcs8::DecodePublicKey;
use serde_json::Value;
use sha1::{Digest, Sha1};

use crate::http::{Http, encode};
use crate::skin::texture_id;

const PUBLIC_KEYS: &str = "https://api.minecraftservices.com/publickeys";
const PROFILE_BY_ID: &str = "https://sessionserver.mojang.com/session/minecraft/profile/";
const PROFILE_BY_NAME: &str = "https://api.mojang.com/users/profiles/minecraft/";

/// The pictures of a profile, by their addresses (all on the texture server).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Textures {
    pub skin: Option<String>,
    /// Whether the skin is for the thin arms.
    pub slim: bool,
    pub cape: Option<String>,
}

impl Textures {
    /// From a textures property's value. Empty if it cannot be read, or if any address in
    /// it is not on the texture server.
    pub fn unpack(value: &str) -> Textures {
        unpack(value).unwrap_or_default()
    }
}

fn unpack(value: &str) -> Option<Textures> {
    let bytes = STANDARD.decode(value).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let all = v.get("textures")?.as_object()?;
    for t in all.values() {
        texture_id(t.get("url")?.as_str()?)?;
    }
    let url = |kind: &str| Some(all.get(kind)?.get("url")?.as_str()?.to_owned());
    Some(Textures {
        skin: url("SKIN"),
        slim: v
            .pointer("/textures/SKIN/metadata/model")
            .and_then(Value::as_str)
            == Some("slim"),
        cape: url("CAPE"),
    })
}

/// The services' keys that profiles' properties are signed with.
#[derive(Debug, Clone, Default)]
pub struct PropertyKeys(Vec<rsa::PublicKey>);

impl PropertyKeys {
    /// From the services' answer.
    pub fn read(v: &Value) -> PropertyKeys {
        let listed = v.get("profilePropertyKeys").and_then(Value::as_array);
        PropertyKeys(
            listed
                .into_iter()
                .flatten()
                .filter_map(|k| {
                    let der = STANDARD.decode(k.get("publicKey")?.as_str()?).ok()?;
                    rsa::PublicKey::from_public_key_der(&der).ok()
                })
                .collect(),
        )
    }

    /// Fetches them; `None` if they cannot be had.
    pub fn fetch(http: &dyn Http) -> Option<PropertyKeys> {
        let keys = PropertyKeys::read(&http.get(PUBLIC_KEYS, None).ok().filter(|r| r.ok())?.json());
        (!keys.0.is_empty()).then_some(keys)
    }

    /// Whether one of them signed `value`.
    pub fn signed(&self, value: &str, signature: &str) -> bool {
        let Ok(signature) = STANDARD.decode(signature) else {
            return false;
        };
        let digest = Sha1::digest(value.as_bytes());
        self.0.iter().any(|k| {
            k.verify(rsa::Pkcs1v15Sign::new::<Sha1>(), &digest, &signature)
                .is_ok()
        })
    }
}

/// A profile as the services know it: its id, and its textures property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: u128,
    pub textures: Option<String>,
}

/// The profile with this id.
pub fn profile_by_id(http: &dyn Http, id: u128) -> Option<Profile> {
    let r = http.get(&format!("{PROFILE_BY_ID}{id:032x}?unsigned=false"), None);
    let v = r.ok().filter(|r| r.status == 200)?.json();
    let id = read_id(&v)?;
    let textures = v.get("properties").and_then(Value::as_array);
    Some(Profile {
        id,
        textures: textures
            .into_iter()
            .flatten()
            .find(|p| p.get("name").and_then(Value::as_str) == Some("textures"))
            .and_then(|p| Some(p.get("value")?.as_str()?.to_owned())),
    })
}

/// The profile with this name. One that no player can have is not asked for.
pub fn profile_by_name(http: &dyn Http, name: &str) -> Option<Profile> {
    let valid = !name.is_empty() && name.len() <= 16 && name.bytes().all(|b| b > 32 && b < 127);
    if !valid {
        return None;
    }
    let r = http.get(&format!("{PROFILE_BY_NAME}{}", encode(name)), None);
    let v = r.ok().filter(|r| r.status == 200)?.json();
    profile_by_id(http, read_id(&v)?)
}

fn read_id(v: &Value) -> Option<u128> {
    u128::from_str_radix(&v.get("id")?.as_str()?.replace('-', ""), 16).ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::AuthError;
    use crate::http::Reply;
    use serde_json::json;

    fn packed(v: &Value) -> String {
        STANDARD.encode(v.to_string())
    }

    const SKIN: &str = "http://textures.minecraft.net/texture/ab12";

    #[test]
    fn a_property_says_where_the_skin_is() {
        let t = Textures::unpack(&packed(&json!({"textures": {
            "SKIN": {"url": SKIN, "metadata": {"model": "slim"}},
            "CAPE": {"url": "https://textures.minecraft.net/texture/cd"}}})));
        assert_eq!((t.skin.as_deref(), t.slim), (Some(SKIN), true));
        assert_eq!(
            t.cape.as_deref(),
            Some("https://textures.minecraft.net/texture/cd")
        );
        let wide = Textures::unpack(&packed(&json!({"textures": {"SKIN": {"url": SKIN}}})));
        assert!(wide.skin.is_some() && !wide.slim && wide.cape.is_none());
    }

    #[test]
    fn an_address_elsewhere_empties_all_of_it() {
        for other in [
            json!({"url": "https://example.com/texture/ab"}),
            json!({"url": "https://textures.minecraft.net.example/texture/ab"}),
            json!({}),
        ] {
            let t = json!({"textures": {"SKIN": {"url": SKIN}, "CAPE": other}});
            assert_eq!(Textures::unpack(&packed(&t)), Textures::default());
        }
        assert_eq!(Textures::unpack("not base64!"), Textures::default());
        assert_eq!(Textures::unpack(&STANDARD.encode("[")), Textures::default());
    }

    /// A key of 2048 bits made for these tests, and the property it signed.
    const KEY: &str = include_str!("../tests/property_key.txt");

    #[test]
    fn only_a_services_key_signs() {
        let lines: Vec<&str> = KEY.lines().collect();
        let (key, value, signature) = (lines[0], lines[1], lines[2]);
        let keys = PropertyKeys::read(&json!({"profilePropertyKeys": [
            {"publicKey": "AAAA"}, {"publicKey": key}], "playerCertificateKeys": []}));
        assert!(keys.signed(value, signature));
        assert!(!keys.signed(&format!("{value}A"), signature));
        assert!(!keys.signed(value, "AAAA") && !keys.signed(value, "?"));
        assert!(!PropertyKeys::default().signed(value, signature));
        let other = PropertyKeys::read(&json!({"playerCertificateKeys": [{"publicKey": key}]}));
        assert!(!other.signed(value, signature));
    }

    struct Answers(Vec<(&'static str, u16, Value)>, Mutex<Vec<String>>);

    impl Http for Answers {
        fn post_form(&self, _: &str, _: &[(&str, &str)]) -> Result<Reply, AuthError> {
            unreachable!()
        }
        fn post_json(&self, _: &str, _: &Value, _: Option<&str>) -> Result<Reply, AuthError> {
            unreachable!()
        }
        fn get(&self, url: &str, bearer: Option<&str>) -> Result<Reply, AuthError> {
            assert_eq!(bearer, None);
            self.1.lock().unwrap().push(url.to_owned());
            let (_, status, v) = self.0.iter().find(|(u, ..)| url.starts_with(u)).unwrap();
            Ok(Reply {
                status: *status,
                body: v.to_string(),
            })
        }
    }

    #[test]
    fn profiles_are_looked_up() {
        let http = Answers(
            vec![
                (PROFILE_BY_NAME, 200, json!({"id": "0a", "name": "Kai"})),
                (
                    PROFILE_BY_ID,
                    200,
                    json!({"id": "0a", "properties": [{"name": "textures", "value": "e30="}]}),
                ),
            ],
            Mutex::default(),
        );
        let p = profile_by_name(&http, "Kai").unwrap();
        assert_eq!((p.id, p.textures.as_deref()), (10, Some("e30=")));
        assert_eq!(
            http.1.lock().unwrap()[1],
            format!("{PROFILE_BY_ID}{:032x}?unsigned=false", 10)
        );
        // Never asked for.
        assert_eq!(profile_by_name(&http, "a/b c"), None);
        assert_eq!(profile_by_name(&http, ""), None);
        assert_eq!(http.1.lock().unwrap().len(), 2);
        let none = Answers(vec![(PROFILE_BY_ID, 204, json!(null))], Mutex::default());
        assert_eq!(profile_by_id(&none, 1), None);
    }
}
