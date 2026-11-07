//! Screen settings the web keeps under `naru.*` keys (rail order, thumbnails, panel sizes, places
//! on the map), so the installed app and a development screen see the same ones.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, ensure};

const PREFIX: &str = "naru.";
const MAX_KEY: usize = 100;
const MAX_VALUE: usize = 1 << 20;
const MAX_KEYS: usize = 200;

static STORE: Mutex<()> = Mutex::new(());

pub fn path(state_file: &Path) -> PathBuf {
    state_file.with_file_name("ui.json")
}

/// The saved settings, empty before the first one is saved.
pub fn read(path: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not a settings file", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// Saves `value` under `key`, or forgets the key for `None`. A file that cannot be read is left
/// as it is.
pub fn set(path: &Path, key: &str, value: Option<&str>) -> anyhow::Result<()> {
    ensure!(
        key.starts_with(PREFIX) && key.len() <= MAX_KEY,
        "not a screen setting: {key}"
    );
    if let Some(value) = value {
        ensure!(
            value.len() <= MAX_VALUE,
            "{key} is too large to keep ({} bytes)",
            value.len()
        );
    }
    let _guard = STORE.lock().expect("ui store lock");
    let mut values = read(path)?;
    match value {
        Some(value) => values.insert(key.to_owned(), value.to_owned()),
        None => values.remove(key),
    };
    ensure!(
        values.len() <= MAX_KEYS,
        "too many screen settings to keep ({})",
        values.len()
    );
    crate::suggestions::write_private(path, &serde_json::to_vec(&values)?)
        .with_context(|| format!("cannot save {}", path.display()))
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
