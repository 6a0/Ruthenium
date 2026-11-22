//! Xbox Live: the user token from the Microsoft access token, then the XSTS token for the
//! Minecraft services.

use serde_json::{Value, json};

use crate::AuthError;
use crate::http::Http;

const USER_AUTH: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT_PARTY: &str = "rp://api.minecraftservices.com/";

/// Why Xbox Live refused (XSTS `XErr` codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XboxRefusal {
    /// The Microsoft account has no Xbox profile yet.
    NoXboxAccount,
    /// Xbox Live is not available in the account's country or region.
    Region,
    /// The account needs adult verification (South Korea).
    AdultVerification,
    /// A child account that must be added to a Microsoft family.
    ChildAccount,
    /// The account is banned from Xbox Live.
    Banned,
    Other(u64),
}

impl XboxRefusal {
    pub fn from_xerr(code: u64) -> XboxRefusal {
        match code {
            2_148_916_233 => XboxRefusal::NoXboxAccount,
            2_148_916_235 => XboxRefusal::Region,
            2_148_916_236 | 2_148_916_237 => XboxRefusal::AdultVerification,
            2_148_916_238 => XboxRefusal::ChildAccount,
            2_148_916_227 => XboxRefusal::Banned,
            other => XboxRefusal::Other(other),
        }
    }
}

impl std::fmt::Display for XboxRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XboxRefusal::NoXboxAccount => f.write_str(
                "this Microsoft account has no Xbox profile yet: sign in once at xbox.com to create one",
            ),
            XboxRefusal::Region => f.write_str("Xbox Live isn't available in this account's country or region"),
            XboxRefusal::AdultVerification => f.write_str("this account needs adult verification on xbox.com"),
            XboxRefusal::ChildAccount => {
                f.write_str("this is a child account: an adult has to add it to a Microsoft family first")
            }
            XboxRefusal::Banned => f.write_str("this account is banned from Xbox Live"),
            XboxRefusal::Other(c) => write!(f, "Xbox Live refused the sign-in (XErr {c})"),
        }
    }
}

/// An Xbox token and the user hash it belongs to.
#[derive(Clone, PartialEq, Eq)]
pub struct XboxToken {
    pub token: String,
    pub user_hash: String,
}

impl std::fmt::Debug for XboxToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XboxToken").finish_non_exhaustive()
    }
}

fn token(v: &Value) -> Result<XboxToken, AuthError> {
    let token = v.get("Token").and_then(Value::as_str);
    let uhs = v
        .pointer("/DisplayClaims/xui/0/uhs")
        .and_then(Value::as_str);
    match (token, uhs) {
        (Some(t), Some(u)) => Ok(XboxToken {
            token: t.to_owned(),
            user_hash: u.to_owned(),
        }),
        _ => Err(AuthError::BadResponse(
            "Xbox token without a user hash".into(),
        )),
    }
}

/// The Xbox Live user token for a Microsoft access token.
pub fn user_token(http: &dyn Http, msa_access_token: &str) -> Result<XboxToken, AuthError> {
    let body = json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={msa_access_token}"),
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT",
    });
    let r = http.post_json(USER_AUTH, &body, None)?;
    if !r.ok() {
        return Err(AuthError::Xbox(XboxRefusal::Other(u64::from(r.status))));
    }
    token(&r.json())
}

/// The XSTS token for the Minecraft services.
pub fn xsts_token(http: &dyn Http, user: &XboxToken) -> Result<XboxToken, AuthError> {
    let body = json!({
        "Properties": { "SandboxId": "RETAIL", "UserTokens": [user.token] },
        "RelyingParty": MINECRAFT_PARTY,
        "TokenType": "JWT",
    });
    let r = http.post_json(XSTS, &body, None)?;
    let v = r.json();
    if r.status == 401 {
        let code = v.get("XErr").and_then(Value::as_u64).unwrap_or(0);
        return Err(AuthError::Xbox(XboxRefusal::from_xerr(code)));
    }
    if !r.ok() {
        return Err(AuthError::Xbox(XboxRefusal::Other(u64::from(r.status))));
    }
    token(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xerr_codes_map() {
        assert_eq!(
            XboxRefusal::from_xerr(2148916233),
            XboxRefusal::NoXboxAccount
        );
        assert_eq!(
            XboxRefusal::from_xerr(2148916238),
            XboxRefusal::ChildAccount
        );
        assert_eq!(XboxRefusal::from_xerr(1), XboxRefusal::Other(1));
        let t = token(&json!({"Token": "t", "DisplayClaims": {"xui": [{"uhs": "123"}]}})).unwrap();
        assert_eq!((t.token.as_str(), t.user_hash.as_str()), ("t", "123"));
        assert!(token(&json!({"Token": "t"})).is_err());
    }
}
