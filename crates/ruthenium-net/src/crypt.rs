//! The login's cryptography (`util/Crypt.java`): the shared secret, the RSA-encrypted
//! key packet, the server hash for the session join, and the AES/CFB8 stream both
//! directions use afterwards. Also offline-mode UUIDs (`UUIDUtil`).

use aes::Aes128;
use aes::cipher::{BlockCipherEncrypt, KeyInit};
use md5::Md5;
use rsa::pkcs8::DecodePublicKey;
use sha1::{Digest, Sha1};

/// AES-128 in CFB8 mode with the shared secret as key and IV (`Crypt.getCipher`): one
/// stream per direction for the whole connection, one AES block per byte.
#[derive(Clone)]
pub struct Cfb8 {
    aes: Aes128,
    register: [u8; 16],
}

impl std::fmt::Debug for Cfb8 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cfb8").finish_non_exhaustive()
    }
}

impl Cfb8 {
    pub fn new(secret: &[u8; 16]) -> Cfb8 {
        Cfb8 {
            aes: Aes128::new(&(*secret).into()),
            register: *secret,
        }
    }

    fn keystream(&self) -> u8 {
        let mut block = aes::Block::from(self.register);
        self.aes.encrypt_block(&mut block);
        block[0]
    }

    fn shift(&mut self, cipher_byte: u8) {
        self.register.copy_within(1.., 0);
        self.register[15] = cipher_byte;
    }

    pub fn encrypt(&mut self, data: &mut [u8]) {
        for b in data {
            *b ^= self.keystream();
            self.shift(*b);
        }
    }

    pub fn decrypt(&mut self, data: &mut [u8]) {
        for b in data {
            let c = *b;
            *b ^= self.keystream();
            self.shift(c);
        }
    }
}

/// A new shared secret (`Crypt.generateSecretKey`: 128 random bits).
pub fn secret() -> Result<[u8; 16], String> {
    let mut s = [0u8; 16];
    getrandom::fill(&mut s).map_err(|e| format!("no random numbers: {e}"))?;
    Ok(s)
}

/// The server hash the session join sends (`Crypt.digestData`, then
/// `new BigInteger(digest).toString(16)`): SHA-1 over the server id in ISO-8859-1, the
/// secret and the server's public key, as a signed hexadecimal number.
pub fn server_hash(server_id: &str, secret: &[u8], public_key: &[u8]) -> String {
    let mut h = Sha1::new();
    // Java replaces what ISO-8859-1 can't hold with '?'.
    let latin1: Vec<u8> = server_id
        .chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect();
    h.update(&latin1);
    h.update(secret);
    h.update(public_key);
    let mut digest: [u8; 20] = h.finalize().into();
    let negative = digest[0] & 0x80 != 0;
    if negative {
        // Two's complement: invert and add one.
        let mut carry = true;
        for b in digest.iter_mut().rev() {
            *b = !*b;
            if carry {
                let (v, c) = b.overflowing_add(1);
                *b = v;
                carry = c;
            }
        }
    }
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let trimmed = hex.trim_start_matches('0');
    let digits = if trimmed.is_empty() { "0" } else { trimmed };
    if negative {
        format!("-{digits}")
    } else {
        digits.to_owned()
    }
}

/// The operating system's random numbers, for RSA padding.
struct OsRng;

impl rsa::rand_core::TryRng for OsRng {
    type Error = getrandom::Error;

    fn try_next_u32(&mut self) -> Result<u32, getrandom::Error> {
        getrandom::u32()
    }

    fn try_next_u64(&mut self) -> Result<u64, getrandom::Error> {
        getrandom::u64()
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), getrandom::Error> {
        getrandom::fill(dst)
    }
}

impl rsa::rand_core::TryCryptoRng for OsRng {}

/// Encrypts `data` with the server's public key (X.509 DER) as Java's
/// `Cipher.getInstance("RSA")` does: PKCS#1 v1.5 padding.
pub fn rsa_encrypt(public_key_der: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let key = rsa::PublicKey::from_public_key_der(public_key_der)
        .map_err(|e| format!("bad server key: {e}"))?;
    key.encrypt(&mut OsRng, rsa::Pkcs1v15Encrypt, data)
        .map_err(|e| format!("couldn't encrypt for the server: {e}"))
}

/// The UUID an offline-mode server gives `name` (`UUIDUtil.createOfflinePlayerUUID`):
/// a version 3 UUID of `OfflinePlayer:<name>`.
pub fn offline_uuid(name: &str) -> u128 {
    let mut d: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    d[6] = (d[6] & 0x0F) | 0x30;
    d[8] = (d[8] & 0x3F) | 0x80;
    u128::from_be_bytes(d)
}

/// A UUID as text with dashes.
pub fn uuid_string(id: u128) -> String {
    let h = format!("{id:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn cfb8_matches_nist() {
        // NIST SP 800-38A, F.3.7 (CFB8-AES128.Encrypt), with the key as IV replaced by
        // the vector's IV through the register.
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let mut c = Cfb8::new(&key);
        c.register = iv;
        let plain = hex("6bc1bee22e409f96e93d7e117393172aae2d");
        let mut data = plain.clone();
        c.encrypt(&mut data);
        assert_eq!(data, hex("3b79424c9c0dd436bace9e0ed4586a4f32b9"));
        let mut d = Cfb8::new(&key);
        d.register = iv;
        // Decrypting in two pieces gives the same stream.
        let (a, b) = data.split_at_mut(7);
        d.decrypt(a);
        d.decrypt(b);
        assert_eq!(data, plain);
    }

    #[test]
    fn server_hashes_match_java() {
        // SHA-1 of the names alone, as BigInteger.toString(16) prints them.
        assert_eq!(
            server_hash("Notch", &[], &[]),
            "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"
        );
        assert_eq!(
            server_hash("jeb_", &[], &[]),
            "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"
        );
        assert_eq!(
            server_hash("simon", &[], &[]),
            "88e16a1019277b15d58faf0541e11910eb756f6"
        );
    }

    #[test]
    fn offline_uuids_match_java() {
        // UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8)).
        assert_eq!(
            uuid_string(offline_uuid("Notch")),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
    }

    #[test]
    fn rsa_takes_java_keys() {
        // A 1024-bit X.509 key as Java's KeyPairGenerator makes for vanilla servers.
        let der = hex(concat!(
            "30819f300d06092a864886f70d010101050003818d0030818902818100be9f72d1fab58fad73f4a9",
            "427be645a946099bc3d74159007693150c2e835501e79c9142f52a3c109d8c7334282441d03a82ad",
            "c4d8291acd8f2c9fabdbb95fcd5972fce9448180cc5131812d07403dca3cdb94797d440b4d113141",
            "62ef7055619db96367cbcb4b6531e1e310dc562ea2febfcefe1bb53a025c48e5165f5b63bd0203010001"
        ));
        let out = rsa_encrypt(&der, &[7; 16]).unwrap();
        assert_eq!(out.len(), 128);
        assert!(rsa_encrypt(&[1, 2, 3], &[7; 16]).is_err());
    }
}
