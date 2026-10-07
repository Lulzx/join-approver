//! Profile pictures and the local cache.
//!
//! People in the pending list get Telegram's inline preview, which arrives
//! with the list itself: a photo for everyone, no extra calls, nothing taken
//! from the rate limit the approvals need. Channels and the signed-in account
//! get a real small photo, downloaded once and kept on disk; the file is named
//! after Telegram's photo id, so a changed photo is fetched again by itself.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use grammers_client::media::Downloadable;
use grammers_client::{tl, InvocationError};

use crate::tg::Tg;

/// The JPEG header Telegram strips from inline previews; height and width
/// go in at bytes 164 and 166. Taken from Telethon/tdesktop.
const STRIPPED_HEADER: &[u8; 623] = include_bytes!("stripped_header.bin");

pub fn stripped_to_data_url(stripped: &[u8]) -> Option<String> {
    if stripped.len() < 3 || stripped[0] != 1 {
        return None;
    }
    let mut jpg = Vec::with_capacity(STRIPPED_HEADER.len() + stripped.len());
    jpg.extend_from_slice(STRIPPED_HEADER);
    jpg[164] = stripped[1];
    jpg[166] = stripped[2];
    jpg.extend_from_slice(&stripped[3..]);
    jpg.extend_from_slice(&[0xff, 0xd9]);
    Some(jpeg_data_url(&jpg))
}

fn jpeg_data_url(bytes: &[u8]) -> String {
    format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

struct PeerPhoto(tl::enums::InputFileLocation);

impl Downloadable for PeerPhoto {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(self.0.clone())
    }
}

/// A peer's small (160 px) profile photo, from disk if already fetched.
/// `key` names the peer in the cache ("c123", "me").
pub async fn photo(
    tg: &Tg,
    dir: &Path,
    key: &str,
    peer: tl::enums::InputPeer,
    photo_id: i64,
) -> Result<String, InvocationError> {
    let path = dir.join(format!("{key}_{photo_id}.jpg"));
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(jpeg_data_url(&bytes));
    }
    let location = PeerPhoto(
        tl::types::InputPeerPhotoFileLocation { big: false, peer, photo_id }.into(),
    );
    tg.pace().await;
    let mut chunks = tg.client.iter_download(&location);
    let mut bytes = Vec::new();
    while let Some(chunk) = chunks.next().await? {
        bytes.extend(chunk);
    }
    store(dir, key, &path, &bytes);
    Ok(jpeg_data_url(&bytes))
}

/// Writes the new photo and drops the peer's old ones.
fn store(dir: &Path, key: &str, path: &Path, bytes: &[u8]) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let prefix = format!("{key}_");
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let _ = std::fs::write(path, bytes);
}

/// Small JSON documents the interface keeps between launches (channel list,
/// counts, account), so the window opens on the last known state at once.
pub struct Cache(pub PathBuf);

impl Cache {
    fn file(&self, key: &str) -> Option<PathBuf> {
        let ok = !key.is_empty()
            && key.len() <= 64
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        ok.then(|| self.0.join(format!("{key}.json")))
    }

    pub fn read(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.file(key)?).ok()
    }

    pub fn write(&self, key: &str, json: &str) -> Result<(), String> {
        let path = self.file(key).ok_or("bad cache key")?;
        std::fs::create_dir_all(&self.0).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    /// Everything cached, photos included: on sign-out, nothing of the
    /// account stays behind.
    pub fn clear(&self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
