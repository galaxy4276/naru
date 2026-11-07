//! Hide AI 제안 for Naru: where agents keep losing time in a repository, gathered from the end of
//! every turn, investigated once a day, and offered as a document fix or an issue.
//!
//! Everything here runs off the main thread or under [`STORE`]; the panel only reads snapshots.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use std::process::Command;
use std::time::Instant;

use chrono::{Local, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agent::Agent;
use crate::titles::ask_claude;
use crate::transcript::{self, message_text as plain_message_text, read_tail};

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn naru_dir() -> Option<PathBuf> {
    home().map(|home| home.join(".naru"))
}

pub const KINDS: [&str; 4] = ["no_doc", "searched_again", "wrong_guess", "tool_friction"];
const MAX_IMPROVEMENTS: usize = 3;
const MAX_IMPROVEMENT_CHARS: usize = 200;
const KEEP_RECORDS: usize = 500;
const KEEP_DAYS: i64 = 60;
pub const WATCH_SESSIONS: usize = 5;
const MAX_FAILED_TAGS: usize = 32;
const MAX_TAG_BYTES: usize = 120;
const MAX_MISSED_CHARS: usize = 120;
pub(crate) const DAY: i64 = 24 * 60 * 60;
/// Records weigh fully for a week, then half as much every thirty days.
const FULL_WEIGHT_DAYS: f32 = 7.;
const HALF_LIFE_DAYS: f32 = 30.;
const IGNORED_SESSIONS: usize = 2;
const QUIET_DAYS: i64 = 14;

const INVESTIGATION_MODEL: &str = "claude-sonnet-5-5";
const RUN_TURNS: usize = 30;
const RUN_SPAN: Duration = Duration::from_secs(20 * 60);
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
const WRAP_UP_TURNS: usize = 4;
const MAX_EDITS: usize = 8;
const MAX_READS: usize = 4;
const MAX_NOTES_CHARS: usize = 1000;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_NOTEBOOK_BYTES: usize = 20 * 1024;
const MAX_RECORDS_BYTES: usize = 12 * 1024;
const MAX_READS_BYTES: usize = 24 * 1024;
const MAX_READ_BYTES: usize = 8 * 1024;
const MAX_LINE_CHARS: usize = 400;
const MAX_TITLE_CHARS: usize = 80;
const MAX_DRAFT_CHARS: usize = 600;
const SESSION_EVENTS: usize = 16;
const SESSION_EVENT_CHARS: usize = 2000;
const FILE_LINES: usize = 120;
const SEARCH_LINES: usize = 40;
const LOG_COMMITS: &str = "10";
const TAIL_BYTES: u64 = 8 * 1024 * 1024;

/// Serializes every read-modify-write of a project's file, across windows and the investigation.
static STORE: Mutex<()> = Mutex::new(());

/// What a record says happened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    /// The turn's end named where it lost time; `kind` says how.
    #[default]
    Improvement,
    /// A tool call failed and no later call of its kind in the turn succeeded.
    FailedTool,
    /// The agent stopped to ask (`question`) or waited on a permission (`blocked`).
    AgentWait,
    /// The turn's end named a problem the session left.
    LabelMissed,
    /// Memory learned or confirmed a card from the user's correction; the tag is the card.
    Correction,
    /// The user said the same thing again; the tag is what it means.
    RepeatedRequest,
    /// The user said again what a card the session was given says; the tag is the card.
    MemoryIgnored,
}

impl Signal {
    /// The title of a line grouped by this signal and `tag` before anyone named its cause.
    pub fn basis_title(self, tag: &str) -> String {
        match self {
            Signal::Improvement => format!("같은 곳에서 계속 헤매요: {tag}"),
            Signal::FailedTool => format!("같은 명령이 계속 실패해요: {tag}"),
            Signal::AgentWait => format!("에이전트가 같은 곳에서 계속 멈춰요: {tag}"),
            Signal::LabelMissed => format!("같은 문제가 계속 남아요: {tag}"),
            Signal::Correction => format!("같은 정정을 계속 하고 있어요: {tag}"),
            Signal::RepeatedRequest => format!("같은 요청을 계속 하고 있어요: {tag}"),
            Signal::MemoryIgnored => format!("에이전트가 받은 기억을 계속 놓쳐요: {tag}"),
        }
    }
}

/// What the user said, for a repeated request.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Said {
    /// A personal preference, fixed by one line of the Naru global instruction.
    pub personal: bool,
    /// The one line that would fix it.
    pub line: String,
    /// The user's words, masked and cut, shown under the evidence.
    pub excerpt: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: u32,
    pub session: String,
    /// The timestamp of the turn's request, which with the session names the turn.
    pub turn: String,
    #[serde(default)]
    pub signal: Signal,
    /// An improvement's kind; empty for other signals.
    pub kind: String,
    /// An improvement's words, or another signal's tag.
    pub text: String,
    pub at: i64,
    /// Not yet read by an investigation.
    pub fresh: bool,
    #[serde(default)]
    pub said: Option<Said>,
    /// How the session had been given the card a `MemoryIgnored` record names: `index` or `body`.
    #[serde(default)]
    pub given: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fix {
    Doc {
        title: String,
        /// Relative to the repository root.
        path: String,
        draft: String,
    },
    Issue {
        title: String,
        body: String,
    },
    /// One line of the Naru global instruction, for a personal preference.
    Global {
        line: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    New,
    Fixing,
    Ready,
    NoChange,
    Issue,
    Applied,
    Effective,
    Skipped,
    Struck,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub diff: String,
    pub added: usize,
    pub removed: usize,
    /// The fix created the file.
    pub created: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub id: u32,
    pub stage: Stage,
    pub discomfort: String,
    pub cause: String,
    pub fix: Option<Fix>,
    pub records: Vec<u32>,
    /// How many records the line had when it was skipped.
    pub skipped_with: Option<usize>,
    pub skips: u32,
    /// Came back after a skip.
    pub revived: bool,
    /// Came back after its fix was kept.
    pub came_back: bool,
    pub applied_at: Option<i64>,
    pub fixing_since: Option<i64>,
    pub change: Option<Change>,
    pub issue: Option<String>,
    pub struck: Option<String>,
    /// Why the last press did nothing, shown on the row.
    pub refusal: Option<String>,
    /// In the notebook but not offered yet: its evidence has not reached the amount's bar.
    #[serde(default)]
    pub held: bool,
    #[serde(default)]
    pub raised_at: Option<i64>,
    /// The signal and tag a line grouped without a model gathers.
    #[serde(default)]
    pub basis: Option<(Signal, String)>,
    /// Started by `자동으로 준비해 두기`.
    #[serde(default)]
    pub auto: bool,
    /// Opened because the user asked (Memory's `문서에 반영`), so it takes no room from what 제안
    /// offers on its own.
    #[serde(default)]
    pub asked: bool,
}

impl Line {
    /// Waiting on the person, with a fix to offer; a line without one waits for its investigation.
    pub fn is_choosable(&self) -> bool {
        matches!(self.stage, Stage::New | Stage::Ready | Stage::NoChange)
            && self.fix.is_some()
            && !self.held
    }

    /// The row's title: its discomfort, or its basis while nobody named one.
    pub fn title(&self) -> String {
        match (&self.discomfort, &self.basis) {
            (discomfort, _) if !discomfort.is_empty() => discomfort.clone(),
            (_, Some((signal, tag))) => signal.basis_title(tag),
            _ => String::new(),
        }
    }
}

/// How much 제안 offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Amount {
    Few,
    #[default]
    Normal,
    Many,
}

impl Amount {
    fn threshold(self) -> f32 {
        match self {
            Amount::Few | Amount::Normal => 3.,
            Amount::Many => 2.,
        }
    }

    fn repeat_threshold(self) -> f32 {
        match self {
            Amount::Few => 4.,
            Amount::Normal => 3.,
            Amount::Many => 2.,
        }
    }

    /// New suggestions open at once, and raised in any seven days.
    fn cap(self) -> usize {
        match self {
            Amount::Few => 1,
            Amount::Normal => 3,
            Amount::Many => 5,
        }
    }
}

fn yes() -> bool {
    true
}

/// A project's Naru AI settings, from ⚙ and the head's switch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Naru AI in this project: off, nothing is read and nothing is asked.
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "yes")]
    pub suggestions: bool,
    #[serde(default = "yes")]
    pub memory: bool,
    #[serde(default)]
    pub amount: Amount,
    /// `자동으로 준비해 두기`.
    #[serde(default)]
    pub auto_prepare: bool,
    /// `읽을 곳 · 세션 기록`.
    #[serde(default = "yes")]
    pub read_sessions: bool,
    /// The first-open notice was shown.
    #[serde(default)]
    pub intro_seen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            suggestions: true,
            memory: true,
            amount: Amount::Normal,
            auto_prepare: false,
            read_sessions: true,
            intro_seen: false,
        }
    }
}

impl Settings {
    /// Whether 제안 reads sessions and investigates.
    pub fn suggesting(&self) -> bool {
        self.enabled && self.suggestions && self.read_sessions
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub root: PathBuf,
    pub next_record: u32,
    pub next_line: u32,
    /// The local date of the last investigation, `YYYY-MM-DD`.
    pub last_run: Option<String>,
    pub last_run_at: Option<i64>,
    pub last_end: Option<String>,
    pub records: Vec<Record>,
    pub lines: Vec<Line>,
    /// Each summarized session's last turn time, with or without improvements, so the watch on a
    /// kept fix counts the sessions that went fine too.
    #[serde(default)]
    pub sessions: HashMap<String, i64>,
    #[serde(default)]
    pub settings: Settings,
}

impl Project {
    pub fn line(&self, id: u32) -> Option<&Line> {
        self.lines.iter().find(|line| line.id == id)
    }

    pub fn line_mut(&mut self, id: u32) -> Option<&mut Line> {
        self.lines.iter_mut().find(|line| line.id == id)
    }

    pub fn record(&self, id: u32) -> Option<&Record> {
        self.records.iter().find(|record| record.id == id)
    }

    /// Sessions that had a turn after `at`, for the watch on a kept fix.
    pub fn sessions_since(&self, at: i64) -> usize {
        self.sessions.values().filter(|last| **last > at).count()
    }

    /// Marks kept fixes effective once enough sessions passed without their problem.
    pub fn settle_watches(&mut self) {
        let mut done = Vec::new();
        for line in &self.lines {
            if line.stage == Stage::Applied
                && let Some(at) = line.applied_at
                && self.sessions_since(at) >= WATCH_SESSIONS
            {
                done.push(line.id);
            }
        }
        for id in done {
            if let Some(line) = self.line_mut(id) {
                line.stage = Stage::Effective;
            }
        }
    }

    /// New suggestions and rows that wait on the user; none while Naru AI or 제안 is off.
    pub fn badge(&self) -> usize {
        if !self.settings.enabled || !self.settings.suggestions {
            return 0;
        }
        self.lines.iter().filter(|line| line.is_choosable()).count()
    }
}

pub fn now() -> i64 {
    Utc::now().timestamp()
}

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// The repository `dir` is in: the nearest ancestor with a `.git`.
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

fn store_dir() -> Option<PathBuf> {
    naru_dir().map(|dir| dir.join("suggestions"))
}

pub fn project_key(root: &Path) -> String {
    Sha256::digest(root.to_string_lossy().as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn store_path(root: &Path) -> Option<PathBuf> {
    store_dir().map(|dir| dir.join(format!("{}.json", project_key(root))))
}

/// `root`'s project for showing; a file that cannot be read shows as empty.
pub fn load(root: &Path) -> Project {
    try_load(root).unwrap_or_else(|error| {
        eprintln!("Suggestions: cannot read {}: {error}", root.display());
        empty_project(root)
    })
}

fn empty_project(root: &Path) -> Project {
    Project {
        root: root.to_path_buf(),
        next_record: 1,
        next_line: 1,
        ..Default::default()
    }
}

/// `root`'s project; a missing file is an empty project, any other read failure an error, so an
/// update never saves an empty project over one it could not read.
fn try_load(root: &Path) -> std::io::Result<Project> {
    let fresh = || empty_project(root);
    let Some(path) = store_path(root) else {
        return Err(std::io::Error::other("HOME is not set"));
    };
    Ok(match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            // Kept aside rather than overwritten by the next save.
            let aside = path.with_extension("json.bad");
            eprintln!(
                "Suggestions: cannot parse {} ({error}), moved to {}",
                path.display(),
                aside.display()
            );
            let _ = std::fs::rename(&path, &aside);
            fresh()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fresh(),
        Err(error) => return Err(error),
    })
}

fn save(project: &Project) {
    let Some(path) = store_path(&project.root) else {
        eprintln!("Suggestions: HOME is not set, nothing saved");
        return;
    };
    let result = serde_json::to_vec_pretty(project)
        .map_err(std::io::Error::other)
        .and_then(|bytes| write_private(&path, &bytes));
    if let Err(error) = result {
        eprintln!("Suggestions: cannot save {}: {error}", path.display());
    }
}

/// Replaces `path` with `bytes` in one rename, readable by the user alone.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".new");
    let temporary = PathBuf::from(temporary);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&temporary)?, bytes)?;
    std::fs::rename(&temporary, path)
}

/// Loads `root`'s project, lets `change` edit it, and saves it, all under the store lock. `None`,
/// with nothing changed, when the file could not be read.
pub fn update<T>(root: &Path, change: impl FnOnce(&mut Project) -> T) -> Option<T> {
    let _guard = STORE.lock().expect("suggestions store lock poisoned");
    let mut project = match try_load(root) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("Suggestions: not updating {}: {error}", root.display());
            return None;
        }
    };
    let result = change(&mut project);
    save(&project);
    Some(result)
}

/// Every repository that has a suggestions file.
pub fn known_roots() -> Vec<PathBuf> {
    let Some(dir) = store_dir() else {
        return Vec::new();
    };
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| {
            let bytes = std::fs::read(path).ok()?;
            serde_json::from_slice::<Project>(&bytes).ok()
        })
        .map(|project| project.root)
        .collect()
}

/// Hides secrets, email addresses and the home account before text is stored or sent.
pub fn mask(text: &str) -> String {
    static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        [
            (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}", "<email>"),
            (r"/Users/[^/\s]+", "~"),
            (r"/home/[^/\s]+", "~"),
            (
                r"\b(sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16}|xox[abpr]-[A-Za-z0-9-]{10,})",
                "<secret>",
            ),
            (
                r#"(?i)\b(password|passwd|secret|token|api[_-]?key)\s*[=:]\s*["']?[^\s"']+"#,
                "$1=<secret>",
            ),
        ]
        .into_iter()
        .map(|(pattern, with)| (Regex::new(pattern).expect("mask pattern"), with))
        .collect()
    });
    let mut text = text.to_string();
    for (pattern, with) in PATTERNS.iter() {
        text = pattern.replace_all(&text, *with).into_owned();
    }
    text
}

pub fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut kept: String = text.chars().take(max.saturating_sub(1)).collect();
    kept.push('…');
    kept
}

/// The `개선: <kind> | <text>` lines of a turn summary, masked, cut, de-duplicated and capped.
pub fn parse_improvements(output: &str) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    let mut dropped = 0;
    for line in output.lines() {
        let Some(rest) = line.trim().strip_prefix("개선:") else {
            continue;
        };
        let Some((kind, text)) = rest.split_once('|') else {
            dropped += 1;
            continue;
        };
        let kind = kind.trim().trim_matches('`').to_string();
        let text = text.trim();
        if !KINDS.contains(&kind.as_str()) || text.is_empty() {
            dropped += 1;
            continue;
        }
        let text = cut(&mask(text), MAX_IMPROVEMENT_CHARS);
        if kept.len() == MAX_IMPROVEMENTS || !seen.insert(text.clone()) {
            dropped += 1;
            continue;
        }
        kept.push((kind, text));
    }
    if dropped > 0 {
        eprintln!(
            "Suggestions: improvements_kept={} dropped={dropped}",
            kept.len()
        );
    }
    kept
}

pub fn is_request(entry: &serde_json::Value) -> bool {
    entry.get("type").and_then(|kind| kind.as_str()) == Some("user")
        && entry.get("isMeta").and_then(|v| v.as_bool()) != Some(true)
        && entry
            .get("message")
            .and_then(plain_message_text)
            .is_some_and(|text| !text.starts_with('<') && !text.starts_with('['))
}

fn tool_parts(entry: &serde_json::Value, kind: &str) -> Vec<serde_json::Value> {
    entry
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part.get("type").and_then(|kind| kind.as_str()) == Some(kind))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// The last turn of a Claude transcript: its request's timestamp and Naru's own count of its tool
/// use. Commands and patterns are counted, never kept.
pub fn last_turn(jsonl: &str) -> Option<LastTurn> {
    let entries: Vec<serde_json::Value> = jsonl
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|entry: &serde_json::Value| {
            entry.get("isSidechain").and_then(|v| v.as_bool()) != Some(true)
        })
        .collect();
    let start = entries.iter().rposition(is_request)?;
    let turn = entries[start]
        .get("timestamp")
        .and_then(|at| at.as_str())?
        .to_string();
    let mut calls = 0;
    let mut files = HashSet::new();
    let mut runs: HashMap<String, usize> = HashMap::new();
    let mut failed = 0;
    let mut kinds: HashMap<String, String> = HashMap::new();
    // Kinds in the order they last failed; a later success of the same kind takes one out.
    let mut failing: Vec<String> = Vec::new();
    for entry in &entries[start + 1..] {
        for part in tool_parts(entry, "tool_use") {
            calls += 1;
            let name = part
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or("");
            let input = part.get("input");
            let field = |key: &str| {
                input
                    .and_then(|input| input.get(key))
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            };
            if let Some(path) = field("file_path") {
                files.insert(path);
            }
            if let Some(key) = field("command").or_else(|| field("pattern")) {
                *runs.entry(format!("{name}:{key}")).or_default() += 1;
            }
            if let Some(id) = part.get("id").and_then(|id| id.as_str()) {
                kinds.insert(id.to_string(), tool_kind(name, field("command").as_deref()));
            }
        }
        for part in tool_parts(entry, "tool_result") {
            let kind = part
                .get("tool_use_id")
                .and_then(|id| id.as_str())
                .and_then(|id| kinds.get(id))
                .cloned();
            let errored = part.get("is_error").and_then(|v| v.as_bool()) == Some(true);
            if errored {
                failed += 1;
            }
            let Some(kind) = kind else {
                continue;
            };
            failing.retain(|other| *other != kind);
            if errored {
                failing.push(kind);
                if failing.len() > MAX_FAILED_TAGS {
                    failing.remove(0);
                }
            }
        }
    }
    let repeated: Vec<usize> = runs.values().copied().filter(|count| *count > 1).collect();
    let summary = format!(
        "tool calls {calls} · distinct files read {} · searches or commands run more than once {} \
         (most {} times) · failed calls {failed}",
        files.len(),
        repeated.len(),
        repeated.iter().max().copied().unwrap_or(0),
    );
    Some(LastTurn {
        turn,
        summary,
        failed: failing,
    })
}

/// The last turn of a transcript, as Naru counted it.
#[derive(Clone, Debug, PartialEq)]
pub struct LastTurn {
    /// The request's timestamp.
    pub turn: String,
    /// The tool-use count, for the summarizing model.
    pub summary: String,
    /// The kinds of tool call that failed and stayed failed.
    pub failed: Vec<String>,
}

/// The `남은 문제:` line of a turn's summary, if the model named one.
pub fn parse_missed(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("남은 문제:"))
        .map(|text| cut(&mask(text.trim()), MAX_MISSED_CHARS))
        .filter(|text| !text.is_empty())
}

/// What one turn's end left for 제안.
#[derive(Debug, Default)]
pub struct TurnFacts<'a> {
    pub turn: &'a str,
    pub improvements: &'a [(String, String)],
    /// The kinds of tool call that failed and stayed failed.
    pub failed: &'a [String],
    pub missed: Option<&'a str>,
}

/// Notes that `session` had a turn and keeps its facts, replacing what the same turn left.
pub fn record_turn(root: &Path, session: &str, facts: &TurnFacts) {
    let turn = facts.turn;
    update(root, |project| {
        let at = now();
        let oldest = at - KEEP_DAYS * DAY;
        project.sessions.insert(session.to_string(), at);
        project.sessions.retain(|_, last| *last >= oldest);
        if !project.settings.suggesting() {
            project.settle_watches();
            return;
        }
        let earlier: Vec<u32> = project
            .records
            .iter()
            .filter(|record| record.session == session && record.turn == turn)
            .map(|record| record.id)
            .collect();
        // A turn an investigation already read stays as it was read.
        if project
            .records
            .iter()
            .any(|record| earlier.contains(&record.id) && !record.fresh)
        {
            project.settle_watches();
            return;
        }
        project
            .records
            .retain(|record| !earlier.contains(&record.id));
        for (kind, text) in facts.improvements {
            push_record(project, session, turn, Signal::Improvement, kind, text);
        }
        for tag in facts.failed {
            add_session_signal(project, session, turn, Signal::FailedTool, tag);
        }
        if let Some(missed) = facts.missed {
            add_session_signal(project, session, turn, Signal::LabelMissed, missed);
        }
        trim_records(project, oldest);
        project.settle_watches();
    });
}

fn push_record(
    project: &mut Project,
    session: &str,
    turn: &str,
    signal: Signal,
    kind: &str,
    text: &str,
) -> u32 {
    let id = project.next_record;
    project.next_record += 1;
    project.records.push(Record {
        id,
        session: session.to_string(),
        turn: turn.to_string(),
        signal,
        kind: kind.to_string(),
        text: text.to_string(),
        at: now(),
        fresh: true,
        said: None,
        given: None,
    });
    id
}

fn trim_records(project: &mut Project, oldest: i64) {
    project.records.retain(|record| record.at >= oldest);
    let excess = project.records.len().saturating_sub(KEEP_RECORDS);
    project.records.drain(..excess);
}

/// A session-wide signal, kept once per session and tag.
fn add_session_signal(project: &mut Project, session: &str, turn: &str, signal: Signal, tag: &str) {
    let tag = clean_tag(tag);
    if tag.is_empty()
        || project.records.iter().any(|record| {
            record.session == session && record.signal == signal && record.text == tag
        })
    {
        return;
    }
    push_record(project, session, turn, signal, "", &tag);
}

/// Whitespace folded, control characters dropped, cut to the tag size.
pub fn clean_tag(tag: &str) -> String {
    let folded = tag
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>();
    cut(&mask(&folded), MAX_TAG_BYTES)
}

/// Notes once per session that its agent stopped to wait (`question` or `blocked`).
pub fn record_wait(root: &Path, session: &str, tag: &str) {
    update(root, |project| {
        if project.settings.suggesting() {
            add_session_signal(project, session, "", Signal::AgentWait, tag);
        }
    });
}

/// What Memory read in a session for 제안: corrections, repeated requests and ignored cards.
#[derive(Clone, Debug, PartialEq)]
pub struct MemorySignal {
    pub signal: Signal,
    pub tag: String,
    pub said: Option<Said>,
    pub given: Option<String>,
}

/// Keeps what Memory read in `session`, when 제안 reads sessions and Memory is on.
pub fn record_memory(root: &Path, session: &str, signals: Vec<MemorySignal>) {
    update(root, |project| {
        if !(project.settings.suggesting() && project.settings.memory) {
            return;
        }
        for found in signals {
            let tag = clean_tag(&found.tag);
            if tag.is_empty() {
                continue;
            }
            let id = push_record(project, session, "", found.signal, "", &tag);
            if let Some(record) = project.records.iter_mut().find(|record| record.id == id) {
                record.said = found.said;
                record.given = found.given;
            }
        }
        trim_records(project, now() - KEEP_DAYS * DAY);
    });
}

/// Offers a ready-made line now, as Memory does when the user asks to take a card to a document.
pub fn add_line(root: &Path, discomfort: String, fix: Fix) -> Option<u32> {
    let fix = checked_fix(fix)
        .map_err(|why| eprintln!("Suggestions: line refused: {why}"))
        .ok()?;
    update(root, |project| {
        let id = project.next_line;
        project.next_line += 1;
        project.lines.push(Line {
            id,
            discomfort: cut(&mask(&discomfort), MAX_TITLE_CHARS),
            fix: Some(fix),
            raised_at: Some(now()),
            asked: true,
            ..Default::default()
        });
        id
    })
}

/// The kind of a failed tool call: a shell command's program and subcommand, else the tool.
pub fn tool_kind(name: &str, command: Option<&str>) -> String {
    let Some(command) = command.filter(|_| name == "Bash") else {
        return name.chars().take(MAX_TAG_BYTES).collect();
    };
    const SKIP: [&str; 7] = ["cd", "export", "set", "source", "unset", "true", "."];
    let words = command
        .split(['\n', ';', '|', '&'])
        .flat_map(|segment| {
            let mut words = segment
                .split_whitespace()
                .skip_while(|word| word.contains('=') && !word.starts_with('-'));
            let program = words.next()?;
            let program = program.rsplit('/').next().unwrap_or(program);
            if SKIP.contains(&program) {
                return None;
            }
            let sub = words.next().filter(|word| {
                word.len() <= 32
                    && word.starts_with(|c: char| c.is_ascii_alphabetic())
                    && word
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
            });
            Some(match sub {
                Some(sub) => format!("{program} {sub}"),
                None => program.to_string(),
            })
        })
        .next();
    words.unwrap_or_else(|| name.to_string())
}

/// One change the investigation asks for.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Edit {
    Open {
        discomfort: String,
        #[serde(default)]
        cause: String,
        fix: Option<Fix>,
        #[serde(default)]
        records: Vec<u32>,
    },
    Attach {
        line: u32,
        records: Vec<u32>,
    },
    Update {
        line: u32,
        discomfort: Option<String>,
        cause: Option<String>,
        fix: Option<Fix>,
    },
    Strike {
        line: u32,
        #[serde(default)]
        why: String,
    },
}

/// A fix the user can act on without opening the records again, or why it is not one.
fn checked_fix(fix: Fix) -> Result<Fix, &'static str> {
    match fix {
        Fix::Doc { title, path, draft } => {
            let (title, draft) = (title.trim(), draft.trim());
            if title.is_empty() || draft.is_empty() {
                return Err("doc fix needs a title and a draft");
            }
            if !is_repo_path(&path) {
                return Err("doc fix path must stay inside the repository");
            }
            Ok(Fix::Doc {
                title: cut(&mask(title), MAX_TITLE_CHARS),
                path,
                draft: cut(&mask(draft), MAX_DRAFT_CHARS),
            })
        }
        Fix::Issue { title, body } => {
            let (title, body) = (title.trim(), body.trim());
            if title.is_empty() || body.is_empty() {
                return Err("issue needs a title and a body");
            }
            Ok(Fix::Issue {
                title: cut(&mask(title), MAX_TITLE_CHARS),
                body: mask(body),
            })
        }
        // A global instruction line comes only from what the user said, never from a model.
        Fix::Global { .. } => Err("a global instruction line is not the investigation's to write"),
    }
}

/// A relative path that names no parent, no `.git` and no credential file.
pub fn is_repo_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && !is_secret_path(path)
}

fn is_secret_path(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_lowercase();
        name == ".git"
            || name.starts_with(".env")
            || name.contains("secret")
            || name.contains("credential")
            || [".pem", ".key", ".p12", ".pfx", ".crt", ".cer"]
                .iter()
                .any(|ext| name.ends_with(ext))
            || name.starts_with("id_rsa")
            || name.starts_with("id_ed25519")
    })
}

/// Applies one edit and says what happened, for the model's next turn.
pub fn apply_edit(project: &mut Project, edit: Edit) -> String {
    let known: HashSet<u32> = project.records.iter().map(|record| record.id).collect();
    match edit {
        Edit::Open {
            discomfort,
            cause,
            fix,
            records,
        } => {
            let fix = match fix.map(checked_fix).transpose() {
                Ok(fix) => fix,
                Err(why) => return format!("open refused: {why}"),
            };
            let id = project.next_line;
            project.next_line += 1;
            project.lines.push(Line {
                id,
                discomfort: cut(&mask(discomfort.trim()), MAX_TITLE_CHARS * 2),
                cause: cut(&mask(cause.trim()), MAX_TITLE_CHARS * 2),
                fix,
                records: records
                    .into_iter()
                    .filter(|id| known.contains(id))
                    .collect(),
                held: true,
                ..Default::default()
            });
            format!("opened s{id}")
        }
        Edit::Attach { line, records } => {
            let times: HashMap<u32, i64> = project
                .records
                .iter()
                .map(|record| (record.id, record.at))
                .collect();
            let Some(target) = project.line_mut(line) else {
                return format!("attach refused: no s{line}");
            };
            let mut added = 0;
            for id in records {
                if times.contains_key(&id) && !target.records.contains(&id) {
                    target.records.push(id);
                    added += 1;
                    if matches!(target.stage, Stage::Applied | Stage::Effective)
                        && target.applied_at.is_some_and(|at| times[&id] > at)
                    {
                        target.stage = Stage::New;
                        target.came_back = true;
                    }
                }
            }
            if target.stage == Stage::Skipped
                && target.skips == 1
                && target
                    .skipped_with
                    .is_some_and(|with| target.records.len() >= with * 2)
            {
                target.stage = Stage::New;
                target.revived = true;
            }
            format!("attached {added} to s{line}")
        }
        Edit::Update {
            line,
            discomfort,
            cause,
            fix,
        } => {
            let fix = match fix.map(checked_fix).transpose() {
                Ok(fix) => fix,
                Err(why) => return format!("update refused: {why}"),
            };
            let Some(target) = project.line_mut(line) else {
                return format!("update refused: no s{line}");
            };
            if !matches!(target.stage, Stage::New | Stage::Skipped) {
                return format!("update refused: s{line} is being applied or applied");
            }
            if let Some(discomfort) = discomfort {
                target.discomfort = cut(&mask(discomfort.trim()), MAX_TITLE_CHARS * 2);
            }
            if let Some(cause) = cause {
                target.cause = cut(&mask(cause.trim()), MAX_TITLE_CHARS * 2);
            }
            if fix.is_some() {
                target.fix = fix;
            }
            format!("updated s{line}")
        }
        Edit::Strike { line, why } => {
            let Some(target) = project.line_mut(line) else {
                return format!("strike refused: no s{line}");
            };
            if target.stage != Stage::New {
                return format!("strike refused: s{line} is not new");
            }
            target.stage = Stage::Struck;
            target.struck = Some(cut(&mask(&why), MAX_TITLE_CHARS * 2));
            format!("struck s{line}")
        }
    }
}

/// Not now: the line goes away until as much evidence piles up again, once.
pub fn skip(line: &mut Line) {
    line.skips += 1;
    line.skipped_with = Some(line.records.len().max(1));
    line.revived = false;
    line.stage = Stage::Skipped;
}

/// Whether a skipped line can still come back once on its own.
pub fn can_return(line: &Line) -> bool {
    line.skips == 1 && line.skipped_with.is_some()
}

/// `되살리기`: the line is judged again on what piled up.
pub fn restore(project: &mut Project, id: u32) {
    if let Some(line) = project.line_mut(id)
        && line.stage == Stage::Skipped
    {
        line.stage = Stage::New;
        line.skips = 0;
        line.skipped_with = None;
        line.held = true;
        line.refusal = None;
    }
    raise(project);
}

fn weight(at: i64, now: i64) -> f32 {
    let days = (now - at).max(0) as f32 / DAY as f32;
    if days <= FULL_WEIGHT_DAYS {
        1.
    } else {
        0.5f32.powf((days - FULL_WEIGHT_DAYS) / HALF_LIFE_DAYS)
    }
}

/// How much a line's evidence weighs: per session, 3 with a correction, 0.5 with only a left
/// problem, else 1; a repeated request scores 0 for its first saying, 1 again in the same session
/// and 2 in another, each weighed by age.
pub fn score(project: &Project, line: &Line) -> f32 {
    let now = now();
    let mut records: Vec<&Record> = line
        .records
        .iter()
        .filter_map(|id| project.record(*id))
        .collect();
    records.sort_by_key(|record| record.at);
    if matches!(line.basis, Some((Signal::RepeatedRequest, _))) {
        let mut seen: HashSet<&str> = HashSet::new();
        return records
            .iter()
            .enumerate()
            .map(|(index, record)| {
                let points = match (index, seen.insert(record.session.as_str())) {
                    (0, _) => 0.,
                    (_, false) => 1.,
                    (_, true) => 2.,
                };
                points * weight(record.at, now)
            })
            .sum();
    }
    let mut sessions: HashMap<&str, (Vec<Signal>, i64)> = HashMap::new();
    for record in records {
        let entry = sessions.entry(&record.session).or_default();
        entry.0.push(record.signal);
        entry.1 = entry.1.max(record.at);
    }
    sessions
        .values()
        .map(|(signals, at)| {
            let points = if signals.contains(&Signal::Correction) {
                3.
            } else if signals.iter().all(|signal| *signal == Signal::LabelMissed) {
                0.5
            } else {
                1.
            };
            points * weight(*at, now)
        })
        .sum()
}

fn ignored_sessions(project: &Project, line: &Line) -> usize {
    line.records
        .iter()
        .filter_map(|id| project.record(*id))
        .filter(|record| record.signal == Signal::MemoryIgnored)
        .map(|record| record.session.as_str())
        .collect::<HashSet<_>>()
        .len()
}

/// Offers held lines whose evidence reached the amount's bar, within its caps: so many new ones
/// open at once and raised in seven days. A line agents keep ignoring a memory for rises after
/// two sessions whatever the amount; at 보통, a project quiet for two weeks gets its strongest.
pub fn raise(project: &mut Project) {
    let amount = project.settings.amount;
    let now = now();
    let open = project
        .lines
        .iter()
        .filter(|line| line.stage == Stage::New && !line.held && !line.asked)
        .count();
    let recent = project
        .lines
        .iter()
        .filter(|line| !line.asked && line.raised_at.is_some_and(|at| now - at < 7 * DAY))
        .count();
    let mut room = amount.cap().saturating_sub(open.max(recent));
    let mut candidates: Vec<(u32, f32, bool)> = project
        .lines
        .iter()
        .filter(|line| line.held && line.stage == Stage::New && line.fix.is_some())
        .map(|line| {
            let bar = if matches!(line.basis, Some((Signal::RepeatedRequest, _))) {
                amount.repeat_threshold()
            } else {
                amount.threshold()
            };
            let score = score(project, line);
            let passes = score >= bar || ignored_sessions(project, line) >= IGNORED_SESSIONS;
            (line.id, score, passes)
        })
        .collect();
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let quiet = amount == Amount::Normal
        && open == 0
        && !project.lines.iter().any(|line| {
            !line.asked && line.raised_at.is_some_and(|at| now - at < QUIET_DAYS * DAY)
        });
    let mut chosen: Vec<u32> = candidates
        .iter()
        .filter(|(_, _, passes)| *passes)
        .map(|(id, _, _)| *id)
        .collect();
    if chosen.is_empty() && quiet {
        chosen.extend(candidates.first().map(|(id, _, _)| *id));
    }
    for id in chosen {
        if room == 0 {
            break;
        }
        if let Some(line) = project.line_mut(id) {
            line.held = false;
            line.raised_at = Some(now);
            room -= 1;
        }
    }
}

/// Gathers fresh records of the tagged signals onto the line with their basis, opening one when
/// none has it, so they are grouped even without a model. Improvements and corrections are not:
/// a model groups the first, and Memory takes the second to the documents itself.
pub fn group_by_tags(project: &mut Project) {
    let grouped: HashSet<u32> = project
        .lines
        .iter()
        .flat_map(|line| line.records.iter().copied())
        .collect();
    let fresh: Vec<(u32, Signal, String, i64, Option<String>)> = project
        .records
        .iter()
        .filter(|record| {
            record.fresh
                && !grouped.contains(&record.id)
                && !matches!(record.signal, Signal::Improvement | Signal::Correction)
        })
        .map(|record| {
            let personal = record
                .said
                .as_ref()
                .filter(|said| said.personal && !said.line.trim().is_empty())
                .map(|said| said.line.trim().to_string());
            (
                record.id,
                record.signal,
                record.text.clone(),
                record.at,
                personal,
            )
        })
        .collect();
    for (id, signal, tag, at, personal) in fresh {
        // A personal preference is fixed by one global instruction line, the user's own words.
        let global = personal.map(|line| Fix::Global { line });
        let basis = Some((signal, tag.clone()));
        let existing = project
            .lines
            .iter()
            .position(|line| line.basis == basis && line.stage != Stage::Struck);
        match existing {
            Some(index) => {
                let line = &mut project.lines[index];
                line.records.push(id);
                if line.fix.is_none() {
                    line.fix = global;
                }
                if line.stage == Stage::Skipped
                    && can_return(line)
                    && line.records.len() >= line.skipped_with.unwrap_or(1) * 2
                {
                    line.stage = Stage::New;
                    line.revived = true;
                }
                if matches!(line.stage, Stage::Applied | Stage::Effective)
                    && line.applied_at.is_some_and(|applied| at > applied)
                {
                    line.stage = Stage::New;
                    line.came_back = true;
                }
            }
            None => {
                let line = project.next_line;
                project.next_line += 1;
                project.lines.push(Line {
                    id: line,
                    records: vec![id],
                    basis,
                    fix: global,
                    held: true,
                    ..Default::default()
                });
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum ReadRequest {
    Session { record: u32 },
    File { path: String },
    Search { text: String },
    GitLog { path: Option<String> },
}

#[derive(Debug, Deserialize)]
struct Answer {
    #[serde(default)]
    edits: Vec<Edit>,
    #[serde(default)]
    reads: Vec<ReadRequest>,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    done: bool,
}

fn cut_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[cut]", &text[..end])
}

fn bound_lines(text: &str) -> String {
    text.lines()
        .map(|line| cut(&mask(line), MAX_LINE_CHARS))
        .collect::<Vec<_>>()
        .join("\n")
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("git failed to start: {error}"))?;
    if !output.status.success() && output.stdout.is_empty() {
        return Err(format!("git exited with {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn read_session(project: &Project, record: u32) -> String {
    let Some(record) = project.record(record) else {
        return "refused: no such record".into();
    };
    let Some(home) = home() else {
        return "refused: HOME is not set".into();
    };
    let Some(path) = transcript::find(Agent::Claude, &record.session, &home) else {
        return "refused: session not found".into();
    };
    let Ok((jsonl, _)) = read_tail(&path, TAIL_BYTES) else {
        return "refused: session_read_bound".into();
    };
    let mut events = Vec::new();
    let mut started = false;
    for entry in jsonl
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
    {
        if !started {
            started =
                entry.get("timestamp").and_then(|at| at.as_str()) == Some(record.turn.as_str());
            if !started {
                continue;
            }
        }
        let kind = entry
            .get("type")
            .and_then(|kind| kind.as_str())
            .unwrap_or("");
        let mut parts = Vec::new();
        if let Some(text) = entry.get("message").and_then(plain_message_text) {
            parts.push(format!("{kind}: {text}"));
        }
        for tool in tool_parts(&entry, "tool_use") {
            parts.push(format!(
                "tool {}: {}",
                tool.get("name")
                    .and_then(|name| name.as_str())
                    .unwrap_or(""),
                tool.get("input")
                    .map(|input| input.to_string())
                    .unwrap_or_default()
            ));
        }
        for result in tool_parts(&entry, "tool_result") {
            let error = result.get("is_error").and_then(|v| v.as_bool()) == Some(true);
            let content = match result.get("content") {
                Some(serde_json::Value::String(text)) => text.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            parts.push(format!(
                "result{}: {content}",
                if error { " (error)" } else { "" }
            ));
        }
        for part in parts {
            events.push(cut(&mask(&part), SESSION_EVENT_CHARS));
            if events.len() == SESSION_EVENTS {
                break;
            }
        }
        if events.len() == SESSION_EVENTS {
            break;
        }
    }
    if events.is_empty() {
        return "refused: the turn is no longer in the session".into();
    }
    events.join("\n---\n")
}

fn read_file(root: &Path, path: &str) -> String {
    if !is_repo_path(path) {
        return "refused: path outside the repository or a credential file".into();
    }
    let full = root.join(path);
    match std::fs::read(&full) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes[..bytes.len().min(256 * 1024)]).into_owned();
            text.lines()
                .take(FILE_LINES)
                .enumerate()
                .map(|(index, line)| {
                    format!("{:>4} {}", index + 1, cut(&mask(line), MAX_LINE_CHARS))
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        Err(_) => "the file does not exist".into(),
    }
}

fn search(root: &Path, text: &str) -> String {
    if text.trim().is_empty() {
        return "refused: empty search".into();
    }
    match git(root, &["grep", "-n", "-I", "-i", "-F", "-e", text]) {
        Ok(output) => {
            let shown: Vec<String> = output
                .lines()
                .filter(|line| {
                    line.split(':')
                        .next()
                        .is_some_and(|path| !is_secret_path(Path::new(path)))
                })
                .take(SEARCH_LINES)
                .map(|line| cut(&mask(line), MAX_LINE_CHARS))
                .collect();
            if shown.is_empty() {
                "no matches".into()
            } else {
                shown.join("\n")
            }
        }
        Err(error) => error,
    }
}

fn git_log(root: &Path, path: Option<&str>) -> String {
    let mut args = vec!["log", "--oneline", "-n", LOG_COMMITS];
    if let Some(path) = path {
        if !is_repo_path(path) {
            return "refused: path outside the repository".into();
        }
        args.extend(["--", path]);
    }
    git(root, &args).map_or_else(|error| error, |output| bound_lines(&output))
}

pub fn run_read(project: &Project, read: &ReadRequest) -> String {
    let result = match read {
        ReadRequest::Session { record } => read_session(project, *record),
        ReadRequest::File { path } => read_file(&project.root, path),
        ReadRequest::Search { text } => search(&project.root, text),
        ReadRequest::GitLog { path } => git_log(&project.root, path.as_deref()),
    };
    cut_bytes(&result, MAX_READ_BYTES)
}

fn notebook(project: &Project) -> String {
    let mut lines: Vec<&Line> = project
        .lines
        .iter()
        .filter(|line| line.stage != Stage::Struck)
        .collect();
    lines.sort_by_key(|line| !matches!(line.stage, Stage::New | Stage::Skipped));
    let text = lines
        .iter()
        .map(|line| {
            let fix = match &line.fix {
                Some(Fix::Doc { title, path, draft }) => {
                    format!("doc fix \"{title}\" in {path}: {draft}")
                }
                Some(Fix::Issue { title, .. }) => format!("issue \"{title}\""),
                Some(Fix::Global { line }) => format!("global instruction line \"{line}\""),
                None => "no fix yet".into(),
            };
            let newest: Vec<String> = line
                .records
                .iter()
                .rev()
                .take(5)
                .map(u32::to_string)
                .collect();
            format!(
                "s{} [{:?}{}{}] {} · cause: {} · {} · records {} (newest {})",
                line.id,
                line.stage,
                if line.skips > 0 {
                    ", skipped by the user (not a finding)"
                } else {
                    ""
                },
                if line.held { ", not offered yet" } else { "" },
                line.title(),
                line.cause,
                fix,
                line.records.len(),
                newest.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    cut_bytes(&text, MAX_NOTEBOOK_BYTES)
}

fn fresh_records(project: &Project) -> String {
    let sessions: Vec<&str> = {
        let mut seen = Vec::new();
        for record in &project.records {
            if !seen.contains(&record.session.as_str()) {
                seen.push(record.session.as_str());
            }
        }
        seen
    };
    let now = now();
    let text = project
        .records
        .iter()
        .filter(|record| record.fresh)
        .map(|record| {
            format!(
                "r{} [{}] session #{} · {} days ago · {}",
                record.id,
                match record.signal {
                    Signal::Improvement => record.kind.clone(),
                    other => format!("{other:?} tag"),
                },
                sessions
                    .iter()
                    .position(|session| *session == record.session)
                    .map_or(0, |index| index + 1),
                (now - record.at) / 86_400,
                record.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    cut_bytes(&text, MAX_RECORDS_BYTES)
}

fn prompt(
    project: &Project,
    turn: usize,
    last: bool,
    notes: &str,
    outcomes: &[String],
    reads: &[(String, String)],
) -> String {
    let phase = if last {
        "LAST TURN: no reads; give your final edits and done=true."
    } else if turn + WRAP_UP_TURNS > RUN_TURNS {
        "Wrap up: finish the edits you are sure of."
    } else {
        "Investigate."
    };
    let reads_text = cut_bytes(
        &reads
            .iter()
            .map(|(what, result)| format!("## {what}\n{result}"))
            .collect::<Vec<_>>()
            .join("\n\n"),
        MAX_READS_BYTES,
    );
    let text = format!(
        "You maintain a notebook of recurring problems agents hit in the repository {root}. \
Records are points where a turn lost time (improvement kinds: no_doc, searched_again, \
wrong_guess, tool_friction) or a signal with its tag (FailedTool: a command kind that kept \
failing; AgentWait: the agent stopped to ask or wait; LabelMissed: a problem the session left; \
RepeatedRequest; MemoryIgnored). A tag is the key the records were grouped by, not the problem: \
split a tag line when its records have different causes and strike it when they share none. \
Lines with a basis were grouped by tag alone; give them a cause and a fix with update. Group records that share one cause into one line; \
attach new records to an existing line instead of opening a duplicate. A line is worth offering \
only when a document, rule or setting change would prevent it. Strike a new line that is only \
noise. The user skipping a line is not a finding. You run nothing yourself; ask for reads.\n\
A doc fix must be one the user can act on without reading the records: a title (at most 80 \
characters, saying what changes), the repository-relative path of the one file it changes \
(usually AGENTS.md or CLAUDE.md, or a doc it already has), and the draft words to write there (at \
most 600 characters). Check the file with a read before offering it. An issue fix carries a title \
and a body saying what was seen and what to look at first; use it only when no document can fix \
it. Write discomfort, cause, titles and drafts in Korean, the way a person would say them.\n\
Answer with one JSON object and nothing else:\n\
{{\"edits\":[{{\"op\":\"open\",\"discomfort\":\"…\",\"cause\":\"…\",\"fix\":{{\"kind\":\"doc\",\"title\":\"…\",\"path\":\"AGENTS.md\",\"draft\":\"…\"}},\"records\":[1,2]}},\
{{\"op\":\"attach\",\"line\":3,\"records\":[4]}},{{\"op\":\"update\",\"line\":3,\"fix\":{{\"kind\":\"issue\",\"title\":\"…\",\"body\":\"…\"}}}},\
{{\"op\":\"strike\",\"line\":5,\"why\":\"…\"}}],\
\"reads\":[{{\"tool\":\"session\",\"record\":4}},{{\"tool\":\"file\",\"path\":\"AGENTS.md\"}},{{\"tool\":\"search\",\"text\":\"cargo test\"}},{{\"tool\":\"git_log\",\"path\":null}}],\
\"notes\":\"what to remember next turn\",\"done\":false}}\n\
At most {MAX_EDITS} edits and {MAX_READS} reads. Lines are numbered s<n> (use the number), records r<n>.\n\n\
Turn {turn} of {RUN_TURNS}. {phase}\n\n# Notebook\n{notebook}\n\n# Fresh records\n{records}\n\n\
# Your notes from the last turn\n{notes}\n\n# Outcome of your last edits\n{outcomes}\n\n# Reads you asked for\n{reads_text}",
        root = project.root.display(),
        notebook = notebook(project),
        records = fresh_records(project),
        outcomes = outcomes.join("\n"),
    );
    cut_bytes(&text, MAX_INPUT_BYTES)
}

/// The JSON object in a model's answer, which may come wrapped in prose or a code fence.
fn parse_answer(output: &str) -> Option<Answer> {
    let start = output.find('{')?;
    let end = output.rfind('}')?;
    serde_json::from_str(output.get(start..=end)?).ok()
}

/// How an investigation ended, as the panel and the log read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum End {
    Done,
    TurnLimit,
    TimeLimit,
    Failed,
}

impl End {
    pub fn as_str(self) -> &'static str {
        match self {
            End::Done => "done",
            End::TurnLimit => "turn_limit",
            End::TimeLimit => "time_limit",
            End::Failed => "failed",
        }
    }
}

/// Runs a day's investigation of `root`: turns of the model editing the notebook, each edit saved
/// before the next is taken.
pub fn investigate(root: &Path, claude: &Path) -> End {
    let started = Instant::now();
    let Some(fresh) = update(root, |project| {
        group_by_tags(project);
        project
            .records
            .iter()
            .filter(|record| record.fresh)
            .map(|record| record.id)
            .collect::<HashSet<u32>>()
    }) else {
        return End::Failed;
    };
    let mut notes = String::new();
    let mut outcomes = Vec::new();
    let mut reads: Vec<(String, String)> = Vec::new();
    let mut answered = 0;
    let mut force_last = false;
    let mut end = End::TurnLimit;
    for turn in 1..=RUN_TURNS {
        let last = force_last || turn == RUN_TURNS || started.elapsed() + TURN_TIMEOUT > RUN_SPAN;
        let project = {
            let _guard = STORE.lock().expect("suggestions store lock poisoned");
            load(root)
        };
        let request = prompt(&project, turn, last, &notes, &outcomes, &reads);
        let answer = ask_claude(claude, INVESTIGATION_MODEL, &request, TURN_TIMEOUT)
            .map_err(|error| eprintln!("Suggestions: investigation turn {turn}: {error}"))
            .ok()
            .and_then(|output| parse_answer(&output));
        let Some(answer) = answer else {
            if last {
                end = if answered == 0 {
                    End::Failed
                } else {
                    End::TurnLimit
                };
                break;
            }
            force_last = true;
            continue;
        };
        answered += 1;
        outcomes = answer
            .edits
            .into_iter()
            .take(MAX_EDITS)
            .map(|edit| {
                update(root, |project| apply_edit(project, edit))
                    .unwrap_or_else(|| "refused: the notebook could not be read".into())
            })
            .collect();
        notes = cut(&answer.notes, MAX_NOTES_CHARS);
        if answer.done || last {
            end = if answer.done {
                End::Done
            } else if started.elapsed() + TURN_TIMEOUT > RUN_SPAN {
                End::TimeLimit
            } else {
                End::TurnLimit
            };
            break;
        }
        let project = {
            let _guard = STORE.lock().expect("suggestions store lock poisoned");
            load(root)
        };
        reads = answer
            .reads
            .iter()
            .take(MAX_READS)
            .map(|read| (format!("{read:?}"), run_read(&project, read)))
            .collect();
    }
    update(root, |project| {
        if end != End::Failed {
            for record in &mut project.records {
                if fresh.contains(&record.id) {
                    record.fresh = false;
                }
            }
        }
        project.last_run = Some(today());
        project.last_run_at = Some(now());
        project.last_end = Some(end.as_str().to_string());
        raise(project);
        project.settle_watches();
    });
    eprintln!(
        "Suggestions: investigation.finished end={} turns_answered={answered} fresh_records={} \
         duration_s={}",
        end.as_str(),
        fresh.len(),
        started.elapsed().as_secs()
    );
    end
}

/// The change a fix left in its file, against the last commit; `None` when there is none.
pub fn file_change(root: &Path, path: &str) -> Option<Change> {
    let tracked =
        git(root, &["ls-files", "--error-unmatch", "--", path]).is_ok_and(|out| !out.is_empty());
    let diff = if tracked {
        git(root, &["diff", "--", path]).ok()?
    } else {
        let full = root.join(path);
        let text = std::fs::read_to_string(&full).ok()?;
        let body: String = text.lines().map(|line| format!("+{line}\n")).collect();
        let count = text.lines().count();
        format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{count} @@\n{body}")
    };
    if diff.trim().is_empty() {
        return None;
    }
    // Only the headers before the first hunk are skipped: a body line can start with `---`.
    let count = |sign: char| {
        diff.lines()
            .skip_while(|line| !line.starts_with("@@"))
            .filter(|line| line.starts_with(sign))
            .count()
    };
    Some(Change {
        added: count('+'),
        removed: count('-'),
        created: !tracked,
        diff,
    })
}

/// Whether the file has changes or is untracked, so a fix would mix with the user's own edits.
pub fn file_is_dirty(root: &Path, path: &str) -> bool {
    git(root, &["status", "--porcelain", "--", path]).is_ok_and(|out| !out.trim().is_empty())
}

/// Takes back a fix whose change is still the one the user was shown.
pub fn revert_change(root: &Path, path: &str, shown: &Change) -> Result<(), &'static str> {
    match file_change(root, path) {
        Some(current) if current.diff == shown.diff => {}
        Some(_) => return Err("본 뒤에 파일이 바뀌었어요"),
        None => return Ok(()),
    }
    if shown.created {
        std::fs::remove_file(root.join(path)).map_err(|_| "파일을 지우지 못했어요")
    } else {
        git(root, &["checkout", "--", path])
            .map(|_| ())
            .map_err(|_| "되돌리지 못했어요")
    }
}

/// Opens an issue with `gh` in the repository; the issue's URL, or why it could not.
pub fn open_issue(root: &Path, title: &str, body: &str) -> Result<String, String> {
    let output = Command::new("gh")
        .current_dir(root)
        .args(["issue", "create", "--title", title, "--body", body])
        .output()
        .map_err(|_| "GitHub 연결 필요".to_string())?;
    if !output.status.success() {
        eprintln!("Suggestions: gh issue create exited with {}", output.status);
        return Err("이슈를 만들지 못했어요".into());
    }
    let url = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find(|line| line.starts_with("https://"))
        .map(str::to_owned);
    url.ok_or_else(|| "이슈를 만들지 못했어요".into())
}

/// Whether `gh` is installed and signed in for this repository.
pub fn github_ready(root: &Path) -> bool {
    Command::new("gh")
        .current_dir(root)
        .args(["auth", "status"])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// The instructions a fixing agent reads, written where the shell never sees their words.
pub fn write_task(root: &Path, line: &Line) -> Option<PathBuf> {
    let Some(Fix::Doc { title, path, draft }) = &line.fix else {
        return None;
    };
    let dir = store_dir()?.join("tasks");
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join(format!("{}-s{}.md", project_key(root), line.id));
    let text = format!(
        "# {title}\n\nNaru 제안 s{id}. 이 저장소({root})에서 `{path}` 파일 하나만 고친다.\n\n\
         왜: {discomfort}{cause}\n\n넣을 내용(알맞은 자리에, 문서의 말투와 형식에 맞춰):\n\n{draft}\n\n\
         규칙:\n- `{path}` 말고 다른 파일은 건드리지 않는다.\n- 커밋하지 않는다. 브랜치를 만들거나 \
         바꾸지 않는다.\n- 이미 같은 뜻이 있으면 고치지 않고 그렇다고 말한 뒤 멈춘다.\n- 끝나면 \
         무엇을 바꿨는지 한 줄로 말하고 멈춘다.\n",
        id = line.id,
        root = root.display(),
        discomfort = line.discomfort,
        cause = if line.cause.is_empty() {
            String::new()
        } else {
            format!(" (놓친 것: {})", line.cause)
        },
    );
    std::fs::write(&file, text).ok()?;
    Some(file)
}

/// The day's investigation is due: not run today and records it has not read.
pub fn is_due(project: &Project) -> bool {
    project.last_run.as_deref() != Some(today().as_str())
        && project.records.iter().any(|record| record.fresh)
}

#[cfg(test)]
#[path = "suggestions_tests.rs"]
mod tests;
