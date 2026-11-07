//! Brain: one Obsidian note the user keeps beside their work, edited in a floating window.
//!
//! The note is read and written whole. A save only lands when the note on disk is still the one
//! the screen last saw, so an edit made in Obsidian meanwhile is never overwritten unasked.

use std::io::Read;
use std::path::Path;

use serde::Serialize;

const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Note {
    /// The note's folder is missing: the vault's disk is not mounted.
    Offline,
    /// The note has not changed since `mtime`.
    Unchanged {
        mtime: u64,
    },
    /// The note on disk, its front matter kept apart so the editor shows only the body.
    Found {
        front: String,
        body: String,
        mtime: u64,
    },
    Error {
        message: String,
    },
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Written {
    Saved {
        mtime: u64,
    },
    Offline,
    /// The note changed on disk since the screen saw it; nothing was written. No `mtime` when
    /// the note was deleted.
    Changed {
        front: String,
        body: String,
        mtime: Option<u64>,
    },
    Error {
        message: String,
    },
}

fn modified_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// The note as it is now: `None` when there is none yet.
fn read_disk(path: &Path) -> anyhow::Result<Option<Option<(String, u64)>>> {
    if !path.parent().is_some_and(Path::is_dir) {
        return Ok(None);
    }
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Some(None)),
        Err(error) => return Err(error.into()),
    };
    let mtime = modified_ms(&file.metadata()?);
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= MAX_BYTES, "노트가 2MiB보다 큽니다");
    Ok(Some(Some((String::from_utf8(bytes)?, mtime))))
}

/// Splits a leading YAML front matter block from the body.
fn split_front_matter(text: &str) -> (&str, &str) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return ("", text);
    };
    let end = if let Some(end) = rest.find("\n---\n") {
        end + 5
    } else if rest.ends_with("\n---") {
        rest.len()
    } else {
        return ("", text);
    };
    text.split_at(4 + end)
}

fn first_note(today: &str) -> String {
    format!(
        "---\ntitle: Brain\nproject: inbox\nreading: 읽기전\ncreated: {today}\n---\n\n## 지금 집중\n\n"
    )
}

/// Reads the note for the screen, creating it when its folder is there but it is not.
pub fn read(path: &Path, known: Option<u64>) -> Note {
    let result = (|| {
        let disk = match read_disk(path)? {
            None => return Ok(Note::Offline),
            Some(Some(disk)) => disk,
            Some(None) => {
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                replace(path, &first_note(&today))?;
                read_disk(path)?
                    .flatten()
                    .ok_or_else(|| anyhow::anyhow!("노트를 만들지 못했어요"))?
            }
        };
        let (text, mtime) = disk;
        if known == Some(mtime) {
            return Ok(Note::Unchanged { mtime });
        }
        let (front, body) = split_front_matter(&text);
        Ok(Note::Found {
            front: front.to_owned(),
            body: body.to_owned(),
            mtime,
        })
    })();
    result.unwrap_or_else(|error: anyhow::Error| Note::Error {
        message: format!("{error:#}"),
    })
}

/// Writes `body` under the note's current front matter, unless the note is no longer the one
/// the screen saw at `base`.
pub fn write(path: &Path, body: &str, base: Option<u64>) -> Written {
    let result = (|| {
        let Some(disk) = read_disk(path)? else {
            return Ok(Written::Offline);
        };
        let (text, disk_mtime) =
            disk.map_or((String::new(), None), |(text, mtime)| (text, Some(mtime)));
        let (front, on_disk) = split_front_matter(&text);
        if disk_mtime != base {
            return Ok(Written::Changed {
                front: front.to_owned(),
                body: on_disk.to_owned(),
                mtime: disk_mtime,
            });
        }
        // ponytail: the check and the rename are not atomic; a save landing in that gap wins.
        replace(path, &format!("{front}{body}"))?;
        Ok(Written::Saved {
            mtime: modified_ms(&std::fs::metadata(path)?),
        })
    })();
    result.unwrap_or_else(|error: anyhow::Error| Written::Error {
        message: format!("{error:#}"),
    })
}

/// Swaps the note for `text` in one rename, so Obsidian never reads half a note.
fn replace(path: &Path, text: &str) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("노트 경로에 폴더가 없어요"))?;
    let temp = dir.join(format!(".Brain.md.naru-{}", uuid::Uuid::new_v4()));
    let result = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    Ok(result?)
}

#[cfg(test)]
#[path = "brain_tests.rs"]
mod tests;
