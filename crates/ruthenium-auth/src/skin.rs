//! Player skins from Mojang's texture server. Only addresses on that server are fetched
//! (the discovery document's `profiles.getTexture` allow-list), and always over HTTPS;
//! the texture id, a hash, names the file a skin is kept in.

use std::path::{Path, PathBuf};

use crate::AuthError;
use crate::http::agent;

/// The texture server's addresses (`validUris` in the discovery document, 2026-11-21).
const PREFIXES: [&str; 2] = [
    "https://textures.minecraft.net/texture/",
    "http://textures.minecraft.net/texture/",
];
/// Skins are a few kilobytes; nothing larger is read.
const MAX_BYTES: u64 = 256 * 1024;

/// The texture id in a skin address, if the address is on the texture server.
pub fn texture_id(url: &str) -> Option<&str> {
    let id = PREFIXES.iter().find_map(|p| url.strip_prefix(p))?;
    (!id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then_some(id)
}

/// Where the skin at `url` is kept in `cache_dir`.
pub fn cache_path(cache_dir: &Path, url: &str) -> Option<PathBuf> {
    Some(
        cache_dir
            .join("skins")
            .join(format!("{}.png", texture_id(url)?)),
    )
}

/// Downloads the skin PNG at `url`. Blocks: run it off the main thread.
pub fn download(url: &str) -> Result<Vec<u8>, AuthError> {
    let id = texture_id(url)
        .ok_or_else(|| AuthError::BadResponse(format!("not a skin address: {url}")))?;
    let mut r = agent()
        .get(&format!("{}{id}", PREFIXES[0]))
        .call()
        .map_err(|e| AuthError::Network(e.to_string()))?;
    let status = r.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(AuthError::Mojang(status, "skin download failed".into()));
    }
    r.body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .map_err(|e| AuthError::Network(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_texture_server() {
        assert_eq!(
            texture_id("http://textures.minecraft.net/texture/1a2b3c"),
            Some("1a2b3c")
        );
        assert_eq!(
            texture_id("https://textures.minecraft.net/texture/ff"),
            Some("ff")
        );
        assert_eq!(texture_id("https://evil.example/texture/ff"), None);
        assert_eq!(
            texture_id("https://textures.minecraft.net/texture/../x"),
            None
        );
        assert_eq!(texture_id("https://textures.minecraft.net/texture/"), None);
        assert_eq!(
            cache_path(Path::new("/c"), "https://textures.minecraft.net/texture/ab"),
            Some(PathBuf::from("/c/skins/ab.png"))
        );
    }
}
