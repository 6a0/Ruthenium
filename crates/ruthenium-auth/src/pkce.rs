//! Proof Key for Code Exchange (RFC 7636) and random state values.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// `n` random bytes from the operating system, base64url without padding.
pub fn random_token(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).expect("the OS random source");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A verifier (43 characters from 32 random bytes) and its S256 challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Pkce {
        Pkce::from_verifier(random_token(32))
    }

    pub fn from_verifier(verifier: String) -> Pkce {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Pkce {
            verifier,
            challenge,
        }
    }
}

impl Default for Pkce {
    fn default() -> Pkce {
        Pkce::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_rfc_example() {
        // RFC 7636, appendix B.
        let p = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let fresh = Pkce::new();
        assert_eq!(fresh.verifier.len(), 43);
        assert_ne!(fresh.verifier, Pkce::new().verifier);
    }
}
