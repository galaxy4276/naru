//! Session card background images. Naru keeps the ones it saves in its own folder; the Warp
//! fork's are read where they are and never deleted.

use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};

const MAX_BYTES: usize = 8 << 20;

pub fn dir(state_file: &Path) -> PathBuf {
    state_file.with_file_name("session-backgrounds")
}

/// The MIME type and file extension of a PNG or JPEG.
fn kind(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("image/jpeg", "jpg"))
    } else {
        None
    }
}

/// Writes a new image into `dir` and returns its path.
pub fn save(dir: &Path, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    ensure!(
        bytes.len() <= MAX_BYTES,
        "the background image is too large ({} bytes)",
        bytes.len()
    );
    let (_, extension) = kind(bytes).context("the background image is not a PNG or JPEG")?;
    let path = dir.join(format!("{}.{extension}", uuid::Uuid::new_v4()));
    crate::suggestions::write_private(&path, bytes)
        .with_context(|| format!("cannot save {}", path.display()))?;
    Ok(path)
}

/// The image's MIME type and bytes.
pub fn read(path: &Path) -> anyhow::Result<(&'static str, Vec<u8>)> {
    let size = std::fs::metadata(path)
        .with_context(|| format!("cannot read {}", path.display()))?
        .len();
    ensure!(
        size <= MAX_BYTES as u64,
        "{} is too large to show ({size} bytes)",
        path.display()
    );
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let (mime, _) =
        kind(&bytes).with_context(|| format!("{} is not a PNG or JPEG", path.display()))?;
    Ok((mime, bytes))
}

/// Deletes `path` when Naru wrote it into `dir`; any other file is the user's and stays.
pub fn remove_own(dir: &Path, path: &Path) {
    if path.parent() != Some(dir) {
        return;
    }
    if let Err(error) = std::fs::remove_file(path) {
        eprintln!("cannot remove {}: {error}", path.display());
    }
}

#[cfg(test)]
#[path = "background_tests.rs"]
mod tests;
