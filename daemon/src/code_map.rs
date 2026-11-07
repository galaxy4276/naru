//! What the code map shows: the files a pane's checkout changed since its last commit, their
//! text with the changed and removed lines, and which of them import which.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use anyhow::{Context, bail};
use regex::Regex;
use serde::Serialize;

// ponytail: fixed caps keep one message small; raise them if real checkouts hit them.
const MAX_FILES: usize = 80;
const MAX_BYTES: u64 = 512 * 1024;
/// git's empty tree, the base of a repository with no commit yet.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Serialize)]
pub struct CodeMap {
    pub root: String,
    pub files: Vec<MapFile>,
    /// `[importer, imported]` indexes into `files`.
    pub edges: Vec<(usize, usize)>,
    /// Changed files left out past the cap.
    pub more: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct MapFile {
    pub path: String,
    pub status: Status,
    pub added: u32,
    pub removed: u32,
    /// The file as it is now; empty when deleted, binary or too big.
    pub lines: Vec<String>,
    /// 1-based line numbers in `lines` that are new.
    pub marks: Vec<u32>,
    pub gone: Vec<Gone>,
    /// Why `lines` is empty while the file exists.
    pub skipped: Option<String>,
}

#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Changed,
    New,
    Deleted,
}

/// Removed lines, shown right after line `after` of the file as it is now (0 = at the top).
#[derive(Debug, PartialEq, Serialize)]
pub struct Gone {
    pub after: u32,
    pub lines: Vec<String>,
}

fn git(root: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn read(cwd: &Path) -> anyhow::Result<CodeMap> {
    let root = git(cwd, &["rev-parse", "--show-toplevel"])
        .map_err(|_| anyhow::anyhow!("git 저장소가 아니에요"))?
        .trim()
        .to_owned();
    let root_path = PathBuf::from(&root);
    let base = if git(&root_path, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
        "HEAD"
    } else {
        EMPTY_TREE
    };
    let diff = git(
        &root_path,
        &[
            "diff",
            base,
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
            "--no-renames",
        ],
    )?;
    let mut files = parse_diff(&diff);
    let untracked = git(
        &root_path,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?;
    for path in untracked.split('\0').filter(|path| !path.is_empty()) {
        files.insert(
            path.to_owned(),
            MapFile {
                status: Status::New,
                ..MapFile::default()
            },
        );
    }
    let total = files.len();
    let mut files: Vec<MapFile> = files
        .into_iter()
        .take(MAX_FILES)
        .map(|(path, mut file)| {
            file.path = path;
            fill(&root_path, &mut file);
            file
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let edges = edges(&files);
    Ok(CodeMap {
        root,
        more: total - files.len(),
        files,
        edges,
    })
}

/// Reads the file as it is now; a new file is all new lines.
fn fill(root: &Path, file: &mut MapFile) {
    if file.status == Status::Deleted {
        return;
    }
    let path = root.join(&file.path);
    let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    if size > MAX_BYTES {
        file.skipped = Some(format!("파일이 커서 열지 않았어요 ({} KiB)", size / 1024));
        return;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        file.skipped = Some("글자가 아닌 파일이에요".into());
        return;
    };
    file.lines = text.lines().map(str::to_owned).collect();
    if file.status == Status::New {
        file.added = file.lines.len() as u32;
        file.marks = (1..=file.added).collect();
    }
}

static HUNK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@").expect("hunk regex"));

/// `git diff --unified=0` by path: new lines, removed lines and whether the file came or went.
pub fn parse_diff(diff: &str) -> BTreeMap<String, MapFile> {
    let mut files = BTreeMap::new();
    let mut path: Option<String> = None;
    let (mut new_at, mut gone_after) = (0u32, 0u32);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            let name = rest.split_once(" b/").map_or(rest, |(_, b)| b).to_owned();
            files.insert(name.clone(), MapFile::default());
            path = Some(name);
            continue;
        }
        let Some(file) = path.as_ref().and_then(|path| files.get_mut(path)) else {
            continue;
        };
        if line.starts_with("new file mode") {
            file.status = Status::New;
        } else if line.starts_with("deleted file mode") {
            file.status = Status::Deleted;
        } else if let Some(hunk) = HUNK.captures(line) {
            let start: u32 = hunk[1].parse().unwrap_or(0);
            let count: u32 = hunk
                .get(2)
                .map_or(1, |count| count.as_str().parse().unwrap_or(1));
            // With no new lines git names the line before the gap.
            new_at = start;
            gone_after = if count == 0 {
                start
            } else {
                start.saturating_sub(1)
            };
        } else if line.starts_with('+') && !line.starts_with("+++") {
            file.marks.push(new_at);
            file.added += 1;
            new_at += 1;
        } else if let Some(text) = line.strip_prefix('-').filter(|_| !line.starts_with("---")) {
            file.removed += 1;
            match file.gone.last_mut() {
                Some(gone) if gone.after == gone_after => gone.lines.push(text.to_owned()),
                _ => file.gone.push(Gone {
                    after: gone_after,
                    lines: vec![text.to_owned()],
                }),
            }
        }
    }
    files
}

static TS_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:from|import|require\()\s*['"]([^'"]+)['"]"#).expect("ts import regex")
});
static RS_CRATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"crate::([a-z_][a-z0-9_]*)").expect("crate regex"));
static RS_MOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:pub\s+)?mod\s+([a-z_][a-z0-9_]*)\s*;").expect("mod regex")
});

/// Which changed files import which, read from TypeScript/JavaScript imports and Rust
/// `crate::`/`mod` paths. Other languages get no lines.
pub fn edges(files: &[MapFile]) -> Vec<(usize, usize)> {
    let index: BTreeMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, file)| (file.path.as_str(), i))
        .collect();
    let mut out = BTreeSet::new();
    for (from, file) in files.iter().enumerate() {
        let path = Path::new(&file.path);
        let dir = path.parent().unwrap_or(Path::new(""));
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        let mut targets: Vec<PathBuf> = Vec::new();
        for line in &file.lines {
            match ext {
                "ts" | "tsx" | "js" | "jsx" | "mjs" => {
                    for spec in TS_IMPORT.captures_iter(line) {
                        targets.extend(ts_targets(dir, &spec[1]));
                    }
                }
                "rs" => {
                    if let Some(src) = crate_src(dir) {
                        for module in RS_CRATE.captures_iter(line) {
                            targets.push(src.join(format!("{}.rs", &module[1])));
                            targets.push(src.join(&module[1]).join("mod.rs"));
                        }
                    }
                    if let Some(module) = RS_MOD.captures(line) {
                        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        let base = if matches!(stem, "main" | "lib" | "mod") {
                            dir.to_path_buf()
                        } else {
                            dir.join(stem)
                        };
                        targets.push(base.join(format!("{}.rs", &module[1])));
                        targets.push(base.join(&module[1]).join("mod.rs"));
                    }
                }
                _ => {}
            }
        }
        for target in targets {
            if let Some(&to) = target.to_str().and_then(|target| index.get(target))
                && to != from
            {
                out.insert((from, to));
            }
        }
    }
    out.into_iter().collect()
}

/// Candidate files for an import spec: relative paths, and `@/` as the nearest `src` folder.
fn ts_targets(dir: &Path, spec: &str) -> Vec<PathBuf> {
    let bases: Vec<PathBuf> = if spec.starts_with('.') {
        vec![normalize(&dir.join(spec))]
    } else if let Some(rest) = spec.strip_prefix("@/") {
        dir.ancestors()
            .filter(|at| at.file_name().is_some_and(|name| name == "src"))
            .map(|src| normalize(&src.join(rest)))
            .collect()
    } else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for base in bases {
        let text = base.to_string_lossy();
        for suffix in ["", ".ts", ".tsx", ".js", ".jsx", "/index.ts", "/index.tsx"] {
            out.push(PathBuf::from(format!("{text}{suffix}")));
        }
    }
    out
}

/// `a/b/../c` → `a/c`, without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The `src` folder of the crate a Rust file's folder sits in (repo-relative).
fn crate_src(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|at| at.file_name().is_some_and(|name| name == "src"))
        .map(Path::to_path_buf)
}

pub fn for_pane(cwd: Option<&str>) -> anyhow::Result<CodeMap> {
    let cwd = cwd.context("이 pane의 폴더를 아직 몰라요")?;
    read(Path::new(cwd))
}

#[cfg(test)]
#[path = "code_map_tests.rs"]
mod tests;
