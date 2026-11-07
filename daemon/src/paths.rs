//! Folders outside Naru that it reads and writes: the Obsidian notes behind Brain, 서재 and 학습함.
//! They are set in 설정 → 경로 and kept in `~/.naru/paths.json`. A feature whose folder is not set
//! stops and says where to set it; Naru never guesses a vault.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paths {
    /// The one note Brain edits.
    pub brain_note: Option<String>,
    /// The folder 서재 keeps a note per book in (under `learning/서재`).
    pub library_notes: Option<String>,
    /// The folder 학습함 writes study notes to, one folder per day.
    pub study_root: Option<String>,
}

fn file(naru_dir: &Path) -> PathBuf {
    naru_dir.join("paths.json")
}

fn naru_dir() -> Result<PathBuf, String> {
    crate::suggestions::naru_dir().ok_or_else(|| "HOME을 알 수 없어요".to_owned())
}

pub fn load_from(naru_dir: &Path) -> Result<Paths, String> {
    let path = file(naru_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| format!("{}을 읽지 못했어요: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Paths::default()),
        Err(error) => Err(format!("{}을 읽지 못했어요: {error}", path.display())),
    }
}

pub fn load() -> Result<Paths, String> {
    load_from(&naru_dir()?)
}

/// Saves what the user typed: blanks unset a path, `~/` becomes the home folder, and anything
/// else must be absolute.
pub fn save_to(naru_dir: &Path, home: &Path, paths: Paths) -> Result<Paths, String> {
    let clean = |value: Option<String>, name: &str| -> Result<Option<String>, String> {
        let Some(value) = value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let value = match value.strip_prefix("~/") {
            Some(rest) => home.join(rest).to_string_lossy().into_owned(),
            None => value,
        };
        if !Path::new(&value).is_absolute() {
            return Err(format!("{name}는 /나 ~/로 시작하는 전체 경로여야 해요"));
        }
        Ok(Some(value))
    };
    let paths = Paths {
        brain_note: clean(paths.brain_note, "Brain 노트")?,
        library_notes: clean(paths.library_notes, "서재 노트 폴더")?,
        study_root: clean(paths.study_root, "학습 노트 폴더")?,
    };
    let bytes = serde_json::to_vec_pretty(&paths).map_err(|error| error.to_string())?;
    crate::suggestions::write_private(&file(naru_dir), &bytes).map_err(|error| format!("경로를 저장하지 못했어요: {error}"))?;
    Ok(paths)
}

pub fn save(paths: Paths) -> Result<Paths, String> {
    let home = crate::suggestions::home().ok_or_else(|| "HOME을 알 수 없어요".to_owned())?;
    save_to(&naru_dir()?, &home, paths)
}

fn need(value: Option<String>, name: &str) -> Result<PathBuf, String> {
    value.map(PathBuf::from).ok_or_else(|| format!("설정 → 경로에서 {name}를 정해 주세요"))
}

/// The note Brain edits. `NARU_BRAIN_NOTE` points QA runs at a note of their own.
pub fn brain_note() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("NARU_BRAIN_NOTE") {
        return Ok(PathBuf::from(path));
    }
    need(load()?.brain_note, "Brain 노트")
}

pub fn library_notes() -> Result<PathBuf, String> {
    need(load()?.library_notes, "서재 노트 폴더")
}

pub fn study_root() -> Result<PathBuf, String> {
    need(load()?.study_root, "학습 노트 폴더")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_paths_expand_home_drop_blanks_and_refuse_relative_ones() {
        let dir = std::env::temp_dir().join(format!("naru-paths-{}", uuid::Uuid::new_v4()));
        let home = Path::new("/Users/me");
        assert_eq!(load_from(&dir).unwrap(), Paths::default());
        let typed = Paths { brain_note: Some(" ~/vault/Brain.md ".into()), library_notes: Some("".into()), study_root: Some("/v/study".into()) };
        let saved = save_to(&dir, home, typed).unwrap();
        assert_eq!(saved.brain_note.as_deref(), Some("/Users/me/vault/Brain.md"));
        assert_eq!(saved.library_notes, None);
        assert_eq!(load_from(&dir).unwrap(), saved);
        assert!(save_to(&dir, home, Paths { study_root: Some("vault/study".into()), ..Default::default() }).is_err());
        assert_eq!(need(None, "학습 노트 폴더").unwrap_err(), "설정 → 경로에서 학습 노트 폴더를 정해 주세요");
    }
}
