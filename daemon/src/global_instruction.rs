//! The Naru global instruction: `~/.naru/global-instruction.md`, lines every agent in a Naru pane
//! gets at SessionStart (`naru guide --hook`), written only from the user's own repeated words.

use std::hash::{Hash, Hasher};

use std::path::PathBuf;

use crate::suggestions::{self, naru_dir};

/// The hook drops a larger file whole rather than cut it, so a line never lands past this.
const MAX_BYTES: usize = 4 * 1024;
const MAX_LINE_CHARS: usize = 400;

fn path() -> Option<PathBuf> {
    naru_dir().map(|dir| dir.join("global-instruction.md"))
}

/// The file as it is now; empty when there is none.
pub fn read() -> std::io::Result<String> {
    let Some(path) = path() else {
        return Err(std::io::Error::other("HOME is not set"));
    };
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

/// What the preview was drawn from, so a press applies only to the file the user saw.
pub fn stamp(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
}

/// Why `line` cannot go into the file, if it cannot.
pub fn check_line(line: &str) -> Result<(), String> {
    let line = line.trim();
    if line.is_empty() {
        return Err("빈 줄이에요".into());
    }
    if line.chars().count() > MAX_LINE_CHARS {
        return Err(format!("{MAX_LINE_CHARS}자를 넘어요"));
    }
    if line.contains(['<', '>']) {
        return Err("<, > 같은 표시는 넣지 않아요".into());
    }
    if line.chars().any(|c| c.is_control() || is_invisible(c)) {
        return Err("보이지 않는 문자가 있어요".into());
    }
    if suggestions::mask(line) != line {
        return Err("비밀 값이나 개인 경로가 있어요".into());
    }
    Ok(())
}

fn has_line(text: &str, line: &str) -> bool {
    text.lines().any(|existing| existing.trim() == line.trim())
}

/// `text` with `line` added at the end.
fn added(text: &str, line: &str) -> Result<String, String> {
    check_line(line)?;
    if has_line(text, line) {
        return Err(format!("이미 \"{}\" 줄이 있어요", line.trim()));
    }
    let mut next = text.to_string();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(line.trim());
    next.push('\n');
    if next.len() > MAX_BYTES {
        return Err(format!(
            "자리 없음: 전역 지시는 {}KiB까지예요 (지금 {}B)",
            MAX_BYTES / 1024,
            text.len()
        ));
    }
    Ok(next)
}

/// `text` without `line`, only when it is still there exactly as it went in.
fn removed(text: &str, line: &str) -> Result<String, String> {
    let mut found = false;
    let kept: Vec<&str> = text
        .lines()
        .filter(|existing| {
            let same = !found && *existing == line.trim();
            found |= same;
            !same
        })
        .collect();
    if !found {
        return Err("손으로 고친 줄이라 그대로 뒀어요".into());
    }
    let mut next = kept.join("\n");
    if !next.is_empty() {
        next.push('\n');
    }
    Ok(next)
}

fn write(text: &str) -> Result<(), String> {
    let path = path().ok_or("HOME is not set")?;
    suggestions::write_private(&path, text.as_bytes())
        .map_err(|error| format!("저장하지 못했어요: {error}"))
}

/// Adds `line` when the file is still the one the preview showed (`seen`).
pub fn apply(line: &str, seen: u64) -> Result<(), String> {
    let text = read().map_err(|error| format!("파일을 읽지 못했어요: {error}"))?;
    if stamp(&text) != seen {
        return Err("파일이 바뀌어서 새 미리보기예요".into());
    }
    write(&added(&text, line)?)
}

pub fn revert(line: &str) -> Result<(), String> {
    let text = read().map_err(|error| format!("파일을 읽지 못했어요: {error}"))?;
    write(&removed(&text, line)?)
}

#[cfg(test)]
#[path = "global_instruction_tests.rs"]
mod tests;
