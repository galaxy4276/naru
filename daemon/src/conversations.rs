//! ⌘K conversation search: Claude transcripts by the words the user typed in them, to resume.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::agent::Agent;
use crate::core::Daemon;

/// The start of a transcript holds its folder and first request; the end its recent ones.
const HEAD_BYTES: u64 = 1024 * 1024;
const TAIL_BYTES: u64 = 1024 * 1024;
const MAX_REQUEST_CHARS: usize = 200;
const MAX_REQUESTS: usize = 40;
const MAX_RESULTS: usize = 8;
const REFRESH_AFTER: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Conversation {
    pub session_id: String,
    pub cwd: PathBuf,
    pub first: String,
    pub last: String,
    /// Unix milliseconds of the transcript's last write.
    pub modified: u64,
    /// Lowercased folder, session id and requests, for matching.
    #[serde(skip)]
    haystack: String,
}

/// Transcripts by path, with the modification time they were read at.
#[derive(Default)]
pub struct Index {
    entries: Mutex<HashMap<PathBuf, (SystemTime, Option<Conversation>)>>,
    refreshing: AtomicBool,
    refreshed_at: Mutex<Option<Instant>>,
}

impl Index {
    /// Rereads the transcripts that changed, unless that happened lately or is happening now.
    pub fn refresh_if_stale(&self, projects: &Path) {
        let stale = self
            .refreshed_at
            .lock()
            .expect("index lock")
            .is_none_or(|at| at.elapsed() > REFRESH_AFTER);
        if !stale || self.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        self.refresh(projects);
        *self.refreshed_at.lock().expect("index lock") = Some(Instant::now());
        self.refreshing.store(false, Ordering::SeqCst);
    }

    fn refresh(&self, projects: &Path) {
        let Ok(dirs) = std::fs::read_dir(projects) else {
            return;
        };
        let mut seen = HashSet::new();
        for path in dirs
            .flatten()
            .filter_map(|dir| std::fs::read_dir(dir.path()).ok())
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        {
            let Ok(modified) = path.metadata().and_then(|meta| meta.modified()) else {
                continue;
            };
            seen.insert(path.clone());
            let known = self
                .entries
                .lock()
                .expect("index lock")
                .get(&path)
                .is_some_and(|(at, _)| *at == modified);
            if !known {
                let conversation = read_conversation(&path, modified);
                self.entries
                    .lock()
                    .expect("index lock")
                    .insert(path, (modified, conversation));
            }
        }
        self.entries
            .lock()
            .expect("index lock")
            .retain(|path, _| seen.contains(path));
    }

    /// The newest conversations containing every word of `query`, newest first.
    pub fn search(&self, query: &str) -> Vec<Conversation> {
        let entries = self.entries.lock().expect("index lock");
        search(query, entries.values().filter_map(|(_, c)| c.clone()))
    }
}

fn read_conversation(path: &Path, modified: SystemTime) -> Option<Conversation> {
    let session_id = path.file_stem()?.to_str()?.to_owned();
    if !crate::agent::is_plain_word(&session_id) {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut head = Vec::new();
    (&mut file).take(HEAD_BYTES).read_to_end(&mut head).ok()?;
    let mut tail = Vec::new();
    if len > HEAD_BYTES {
        file.seek(SeekFrom::Start(
            len.saturating_sub(TAIL_BYTES).max(HEAD_BYTES),
        ))
        .ok()?;
        file.read_to_end(&mut tail).ok()?;
    }
    let modified = modified
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    parse_conversation(
        session_id,
        &String::from_utf8_lossy(&head),
        &String::from_utf8_lossy(&tail),
        modified,
    )
}

/// A conversation from the start and end of its transcript; `None` when no request was typed.
fn parse_conversation(
    session_id: String,
    head: &str,
    tail: &str,
    modified: u64,
) -> Option<Conversation> {
    let mut cwd = None;
    let mut requests = Vec::new();
    // A seek lands mid-line; such a line does not parse and is skipped.
    for line in head.lines().chain(tail.lines()) {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if cwd.is_none() {
            cwd = entry
                .get("cwd")
                .and_then(|cwd| cwd.as_str())
                .map(PathBuf::from);
        }
        if let Some(request) = request_text(&entry) {
            requests.push(request);
        }
    }
    let cwd = cwd?;
    let first = requests.first()?.clone();
    let last = requests.last()?.clone();
    let recent = requests.len().saturating_sub(MAX_REQUESTS);
    let haystack = [cwd.display().to_string(), session_id.clone(), first.clone()]
        .into_iter()
        .chain(requests[recent..].iter().cloned())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    Some(Conversation {
        session_id,
        cwd,
        first,
        last,
        modified,
        haystack,
    })
}

/// What the user typed in a transcript entry, leaving out injected context and command wrappers.
fn request_text(entry: &serde_json::Value) -> Option<String> {
    if entry.get("type")?.as_str()? != "user"
        || entry.get("isMeta").and_then(|v| v.as_bool()) == Some(true)
        || entry.get("isSidechain").and_then(|v| v.as_bool()) == Some(true)
    {
        return None;
    }
    let text = match entry.get("message")?.get("content")? {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter(|part| part.get("type").and_then(|kind| kind.as_str()) == Some("text"))
            .filter_map(|part| part.get("text").and_then(|text| text.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let injected = text.starts_with('<')
        || text.starts_with('[')
        || text.starts_with("This session is being continued");
    (!text.is_empty() && !injected).then(|| text.chars().take(MAX_REQUEST_CHARS).collect())
}

fn search(query: &str, conversations: impl Iterator<Item = Conversation>) -> Vec<Conversation> {
    let words: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<Conversation> = conversations
        .filter(|c| words.iter().all(|word| c.haystack.contains(word.as_str())))
        .collect();
    found.sort_by(|a, b| b.modified.cmp(&a.modified));
    found.truncate(MAX_RESULTS);
    found
}

impl Daemon {
    /// Searches the transcripts, rereading changed ones first when the index is old.
    pub fn search_conversations(&self, query: &str) -> Vec<Conversation> {
        self.conversations.refresh_if_stale(&self.claude_projects);
        self.conversations.search(query)
    }

    /// Goes to the pane already showing `session_id`, else resumes it: in the focused pane when
    /// `here` and that pane is a plain shell, otherwise in a new card in the conversation's folder.
    pub fn resume_conversation(
        self: &Arc<Self>,
        session_id: &str,
        cwd: &Path,
        here: bool,
    ) -> anyhow::Result<()> {
        let (showing, launcher, focused) = {
            let core = self.core();
            let workspace = &core.workspace;
            let showing = workspace.panes.iter().find_map(|(pane, record)| {
                (record.session.as_ref()?.session_id.as_deref() == Some(session_id))
                    .then_some(*pane)
            });
            let focused = workspace
                .tabs
                .iter()
                .find(|tab| Some(tab.id) == workspace.active_tab)
                .map(|tab| tab.focused)
                .filter(|pane| {
                    workspace.panes.get(pane).is_some_and(|record| {
                        record.session.is_none()
                            && record.running.is_none()
                            && record.node.is_none()
                    })
                });
            (
                showing,
                usual_launcher(workspace.panes.values().filter_map(|r| r.session.as_ref())),
                focused,
            )
        };
        if let Some(pane) = showing {
            return self.focus(pane);
        }
        let command = Agent::Claude
            .resume_command(launcher.as_deref(), session_id)
            .ok_or_else(|| anyhow::anyhow!("not resuming {session_id}: not a plain session id"))?;
        anyhow::ensure!(cwd.is_dir(), "{}이 없어요", cwd.display());
        if here && let Some(pane) = focused {
            let line = format!(
                "cd {} && {command}\r",
                shell_words::quote(&cwd.to_string_lossy())
            );
            return self.input(pane, line);
        }
        self.new_tab(
            None,
            Some(cwd.to_string_lossy().into_owned()),
            Some(command),
        )?;
        Ok(())
    }
}

/// The launcher most open Claude panes were started with (such as a shell function wrapping
/// `claude`), so a resumed conversation runs the same way. `None` runs plain `claude`.
pub(crate) fn usual_launcher<'a>(
    sessions: impl Iterator<Item = &'a crate::session::Session>,
) -> Option<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for launcher in sessions
        .filter(|session| session.agent == Agent::Claude)
        .filter_map(|session| session.launcher.as_deref())
        .filter(|launcher| crate::agent::is_plain_word(launcher))
    {
        *counts.entry(launcher).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(launcher, _)| launcher.to_owned())
}

#[cfg(test)]
#[path = "conversations_tests.rs"]
mod tests;
