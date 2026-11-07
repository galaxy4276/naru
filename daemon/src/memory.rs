//! Memory: what the user and the agents settled on in a repository, kept as cards that every agent
//! in a Naru pane gets: an index at SessionStart and the matching bodies with each request (both
//! from `naru guide --hook` and `naru memory --hook`, which leave receipts beside the store).
//!
//! A session is read once it has been quiet for half an hour or its pane closed. What the read
//! finds besides cards (corrections, repeated requests, ignored cards) goes to 제안.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::agent::Agent as CLIAgent;
use crate::suggestions::{self, MemorySignal, Said, Signal, cut, naru_dir};
use crate::titles::ask_claude;
use crate::transcript::{self, read_tail};

fn find_transcript(agent: CLIAgent, session: &str) -> Option<PathBuf> {
    transcript::find(agent, session, &suggestions::home()?)
}

/// Each request with its time in seconds and the agent's last reply, oldest first.
fn requests(agent: CLIAgent, jsonl: &str) -> Vec<(Option<i64>, String, Option<String>)> {
    transcript::turns(agent, jsonl)
        .into_iter()
        .map(|turn| {
            let at = turn.at.as_deref().and_then(|at| {
                chrono::DateTime::parse_from_rfc3339(at)
                    .ok()
                    .map(|at| at.timestamp())
            });
            (at, turn.request, turn.reply)
        })
        .collect()
}

pub const QUIET_SECS: i64 = 30 * 60;
const ARCHIVE_DAYS: i64 = 60;
const DAY: i64 = 24 * 60 * 60;
const MAX_TITLE_CHARS: usize = 60;
const MAX_BODY_CHARS: usize = 4000;
const MAX_ALIASES: usize = 8;
const MAX_CARDS: usize = 200;
const KEEP_LOG: usize = 50;
const MAX_REPEATS: usize = 5;
/// Documents a card can be taken to, the first five the repository has.
pub const DOCS: [&str; 5] = [
    "AGENTS.md",
    "CLAUDE.md",
    "GEMINI.md",
    ".github/copilot-instructions.md",
    ".cursorrules",
];
const CAPTURE_MODEL: &str = "claude-sonnet-5-5";
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(180);
const TAIL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_TALK_BYTES: usize = 48 * 1024;
const REQUEST_CHARS: usize = 600;
const REPLY_CHARS: usize = 300;
const DOC_BYTES: usize = 24 * 1024;
const DOCS_BYTES: usize = 40 * 1024;
const RECEIPT_BYTES: u64 = 1024 * 1024;

static STORE: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Rule,
    Decision,
    Lesson,
    Fact,
    Preference,
}

impl Kind {
    fn parse(text: &str) -> Option<Kind> {
        Some(match text {
            "rule" => Kind::Rule,
            "decision" => Kind::Decision,
            "lesson" => Kind::Lesson,
            "fact" => Kind::Fact,
            "preference" => Kind::Preference,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub id: u32,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Sessions the card was learned or confirmed in.
    #[serde(default)]
    pub sources: Vec<String>,
    pub at: i64,
    #[serde(default)]
    pub archived_at: Option<i64>,
    /// Documents a suggestion was opened for.
    #[serde(default)]
    pub promoted: Vec<String>,
}

/// A card a repository document says otherwise about.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub card: u32,
    pub doc: String,
    pub says: String,
}

/// One change to the cards, with what to put back for `되돌리기`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub id: u32,
    pub at: i64,
    pub what: String,
    pub card: u32,
    /// The card before the change; `None` when the change added it.
    pub before: Option<Card>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    pub root: PathBuf,
    pub next_card: u32,
    pub next_change: u32,
    pub cards: Vec<Card>,
    pub conflicts: Vec<Conflict>,
    pub log: Vec<Change>,
    /// Each read session's last turn time when it was read.
    pub captured: HashMap<String, i64>,
    /// Codex sessions seen in a pane of the repository, with their rollout's last write. Claude
    /// sessions come from 제안's turn summaries instead.
    #[serde(default)]
    pub codex: HashMap<String, i64>,
}

impl Memory {
    pub fn card(&self, id: u32) -> Option<&Card> {
        self.cards.iter().find(|card| card.id == id)
    }

    fn card_mut(&mut self, id: u32) -> Option<&mut Card> {
        self.cards.iter_mut().find(|card| card.id == id)
    }

    /// Notes a change about to be made to `card`, keeping it as it is now.
    fn note(&mut self, card: u32, what: String) {
        let before = self.card(card).cloned();
        let id = self.next_change.max(1);
        self.next_change = id + 1;
        self.log.push(Change {
            id,
            at: suggestions::now(),
            what,
            card,
            before,
        });
        let excess = self.log.len().saturating_sub(KEEP_LOG);
        self.log.drain(..excess);
    }

    pub fn archive(&mut self, id: u32, why: &str) {
        if self.card(id).is_some_and(|card| card.archived_at.is_none()) {
            self.note(id, format!("m{id} 보관 · {why}"));
            if let Some(card) = self.card_mut(id) {
                card.archived_at = Some(suggestions::now());
            }
        }
    }

    pub fn revive(&mut self, id: u32) {
        if self.card(id).is_some_and(|card| card.archived_at.is_some()) {
            self.note(id, format!("m{id} 되살림"));
            if let Some(card) = self.card_mut(id) {
                card.archived_at = None;
                card.at = suggestions::now();
            }
        }
    }

    /// Puts back what change `id` replaced.
    pub fn undo(&mut self, id: u32) {
        let Some(index) = self.log.iter().position(|change| change.id == id) else {
            return;
        };
        let change = self.log.remove(index);
        self.cards.retain(|card| card.id != change.card);
        if let Some(before) = change.before {
            self.cards.push(before);
            self.cards.sort_by_key(|card| card.id);
        }
    }
}

fn store_dir() -> Option<PathBuf> {
    naru_dir().map(|dir| dir.join("memory"))
}

fn store_path(root: &Path) -> Option<PathBuf> {
    store_dir().map(|dir| dir.join(format!("{}.json", suggestions::project_key(root))))
}

fn receipts_path(root: &Path) -> Option<PathBuf> {
    store_dir().map(|dir| dir.join(format!("{}.receipts.jsonl", suggestions::project_key(root))))
}

fn empty(root: &Path) -> Memory {
    Memory {
        root: root.to_path_buf(),
        next_card: 1,
        next_change: 1,
        ..Default::default()
    }
}

fn try_load(root: &Path) -> std::io::Result<Memory> {
    let path = store_path(root).ok_or_else(|| std::io::Error::other("HOME is not set"))?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).or_else(|error| {
            let aside = path.with_extension("json.bad");
            eprintln!(
                "Memory: cannot parse {} ({error}), moved to {}",
                path.display(),
                aside.display()
            );
            std::fs::rename(&path, &aside)?;
            Ok(empty(root))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(empty(root)),
        Err(error) => Err(error),
    }
}

/// `root`'s memory for showing; a file that cannot be read shows as empty.
pub fn load(root: &Path) -> Memory {
    try_load(root).unwrap_or_else(|error| {
        eprintln!("Memory: cannot read {}: {error}", root.display());
        empty(root)
    })
}

/// Loads, edits and saves `root`'s memory under the store lock; `None`, with nothing changed,
/// when the file could not be read.
pub fn update<T>(root: &Path, change: impl FnOnce(&mut Memory) -> T) -> Option<T> {
    let _guard = STORE.lock().expect("memory store lock poisoned");
    let mut memory = try_load(root)
        .map_err(|error| eprintln!("Memory: not updating {}: {error}", root.display()))
        .ok()?;
    let result = change(&mut memory);
    let path = store_path(root)?;
    if let Err(error) = serde_json::to_vec_pretty(&memory)
        .map_err(std::io::Error::other)
        .and_then(|bytes| suggestions::write_private(&path, &bytes))
    {
        eprintln!("Memory: cannot save {}: {error}", path.display());
    }
    Some(result)
}

/// `이 프로젝트의 기억 지우기`: the cards and the receipts go.
pub fn forget(root: &Path) {
    let _guard = STORE.lock().expect("memory store lock poisoned");
    for path in [store_path(root), receipts_path(root), holds_path(root)]
        .into_iter()
        .flatten()
    {
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("Memory: cannot remove {}: {error}", path.display());
        }
    }
}

/// A card handed to a session by a hook.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Receipt {
    pub session: String,
    pub card: u32,
    /// `index` at SessionStart, `body` with a request.
    pub how: String,
    pub at: i64,
}

pub fn receipts(root: &Path) -> Vec<Receipt> {
    let Some(path) = receipts_path(root) else {
        return Vec::new();
    };
    let Ok((text, _)) = read_tail(&path, RECEIPT_BYTES) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn holds_path(root: &Path) -> Option<PathBuf> {
    store_dir().map(|dir| dir.join(format!("{}.held.json", suggestions::project_key(root))))
}

/// A card an agent reported out of date with `naru memory stale`: the user's current words say
/// otherwise. It holds the card's revision (`at`) it was given, so a card changed since is out of
/// the hold. A held card leaves the index and the request matches; `naru memory show` still
/// prints it. Kept beside the store rather than in it, which the Warp fork rewrites.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hold {
    pub revision: i64,
    pub reason: String,
    pub since: i64,
    pub session: String,
}

fn read_holds(root: &Path) -> BTreeMap<u32, Hold> {
    holds_path(root)
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The holds that still apply: the card is there, not archived, and unchanged since.
pub fn holds(root: &Path, memory: &Memory) -> BTreeMap<u32, Hold> {
    live_holds(read_holds(root), memory)
}

fn live_holds(mut holds: BTreeMap<u32, Hold>, memory: &Memory) -> BTreeMap<u32, Hold> {
    holds.retain(|id, hold| {
        memory
            .card(*id)
            .is_some_and(|card| card.archived_at.is_none() && card.at == hold.revision)
    });
    holds
}

fn write_holds(root: &Path, holds: &BTreeMap<u32, Hold>) -> std::io::Result<()> {
    let path = holds_path(root).ok_or_else(|| std::io::Error::other("HOME is not set"))?;
    suggestions::write_private(&path, &serde_json::to_vec_pretty(holds)?)
}

const MAX_REASON_CHARS: usize = 300;

/// Holds card `id` for `session`, which must have been handed the card's body: a session that
/// never got the card cannot say the user contradicted it.
pub fn hold(root: &Path, id: u32, session: &str, reason: &str) -> Result<(), String> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err("say why the card is out of date".into());
    }
    let _guard = STORE.lock().expect("memory store lock poisoned");
    let memory = try_load(root).map_err(|error| format!("cannot read the memory: {error}"))?;
    let revision = revision_to_hold(&memory, &receipts(root), id, session)?;
    let mut holds = read_holds(root);
    holds.insert(
        id,
        Hold {
            revision,
            reason: cut(&suggestions::mask(reason), MAX_REASON_CHARS),
            since: suggestions::now(),
            session: session.to_owned(),
        },
    );
    write_holds(root, &holds).map_err(|error| format!("cannot save the hold: {error}"))
}

/// The revision of card `id` that `session` may hold.
fn revision_to_hold(
    memory: &Memory,
    receipts: &[Receipt],
    id: u32,
    session: &str,
) -> Result<i64, String> {
    let card = memory
        .card(id)
        .filter(|card| card.archived_at.is_none())
        .ok_or_else(|| format!("no memory card m{id} in this repository"))?;
    let given = receipts
        .iter()
        .any(|receipt| receipt.session == session && receipt.card == id && receipt.how == "body");
    if !given {
        return Err(format!(
            "this session was not given m{id}'s body; only a session a request brought it to can report it"
        ));
    }
    Ok(card.at)
}

/// `다시 쓰기`: the user says the card holds after all.
pub fn release(root: &Path, id: u32) {
    let _guard = STORE.lock().expect("memory store lock poisoned");
    let mut holds = read_holds(root);
    if holds.remove(&id).is_some()
        && let Err(error) = write_holds(root, &holds)
    {
        eprintln!("Memory: cannot release m{id}: {error}");
    }
}

/// When each card last went out with its body, else when it last changed.
pub fn last_used(memory: &Memory, receipts: &[Receipt]) -> HashMap<u32, i64> {
    let mut used: HashMap<u32, i64> = memory.cards.iter().map(|card| (card.id, card.at)).collect();
    for receipt in receipts.iter().filter(|receipt| receipt.how == "body") {
        if let Some(at) = used.get_mut(&receipt.card) {
            *at = (*at).max(receipt.at);
        }
    }
    used
}

/// Cards nobody was handed for sixty days go to 보관.
pub fn tidy(memory: &mut Memory, receipts: &[Receipt]) {
    let now = suggestions::now();
    let stale: Vec<u32> = last_used(memory, receipts)
        .into_iter()
        .filter(|(_, at)| now - at >= ARCHIVE_DAYS * DAY)
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        memory.archive(id, "60일 안 씀");
    }
}

/// Sessions of `root` that wait to be read: quiet for half an hour or no longer in a pane
/// (`live`), with turns after their last read.
pub fn due_sessions(root: &Path, live: &HashSet<String>) -> Vec<String> {
    let project = suggestions::load(root);
    if !(project.settings.enabled && project.settings.memory) {
        return Vec::new();
    }
    let memory = load(root);
    let now = suggestions::now();
    let mut due: Vec<(&String, &i64)> = project
        .sessions
        .iter()
        .chain(memory.codex.iter())
        .filter(|(session, last)| {
            memory
                .captured
                .get(*session)
                .is_none_or(|read| read < *last)
                && (now - **last >= QUIET_SECS || !live.contains(*session))
        })
        .collect();
    due.sort_by_key(|(_, last)| **last);
    due.into_iter()
        .map(|(session, _)| session.clone())
        .collect()
}

/// The session's requests and the agent's replies, oldest first, masked and cut.
fn talk(agent: CLIAgent, jsonl: &str) -> Vec<(Option<i64>, String, String)> {
    let mut lines = Vec::new();
    for (at, request, reply) in requests(agent, jsonl) {
        lines.push((
            at,
            "USER".into(),
            cut(&suggestions::mask(&request), REQUEST_CHARS),
        ));
        if let Some(reply) = reply {
            lines.push((
                at,
                "AGENT".into(),
                cut(&suggestions::mask(&reply), REPLY_CHARS),
            ));
        }
    }
    lines
}

/// Notes the last write of a Codex session's rollout, so it is read once it goes quiet.
pub fn note_codex(root: &Path, session: &str) {
    let Some(written) = find_transcript(CLIAgent::Codex, session)
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok())
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|at| at.as_secs() as i64)
    else {
        return;
    };
    if load(root).codex.get(session) == Some(&written) {
        return;
    }
    update(root, |memory| {
        memory.codex.insert(session.to_string(), written);
    });
}

/// The repository documents a card may disagree with, each cut to its share of the budget.
fn doc_heads(root: &Path) -> Vec<(String, String)> {
    let mut left = DOCS_BYTES;
    let mut heads = Vec::new();
    for doc in DOCS {
        let Ok(text) = std::fs::read_to_string(root.join(doc)) else {
            continue;
        };
        let mut head = suggestions::mask(&text);
        let mut end = head.len().min(DOC_BYTES).min(left);
        while !head.is_char_boundary(end) {
            end -= 1;
        }
        head.truncate(end);
        left -= head.len();
        heads.push((doc.to_string(), head));
        if left == 0 {
            break;
        }
    }
    heads
}

/// The documents `root` has that a card can be taken to.
pub fn docs(root: &Path) -> Vec<String> {
    DOCS.iter()
        .filter(|doc| root.join(doc).is_file())
        .map(|doc| doc.to_string())
        .collect()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Capture {
    pub cards: Vec<CardEdit>,
    pub corrections: Vec<Correction>,
    pub repeats: Vec<Repeat>,
    pub ignored: Vec<u32>,
    pub conflicts: Vec<Conflict>,
    /// Concepts the user did not know, for the 학습함.
    pub learn: Vec<crate::learn::LearnFind>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CardEdit {
    Add {
        kind: String,
        title: String,
        body: String,
        #[serde(default)]
        aliases: Vec<String>,
    },
    Update {
        id: u32,
        title: Option<String>,
        body: Option<String>,
        aliases: Option<Vec<String>>,
    },
    Merge {
        id: u32,
        into: u32,
    },
}

#[derive(Debug, Deserialize)]
pub struct Correction {
    pub card: u32,
}

#[derive(Debug, Deserialize)]
pub struct Repeat {
    pub tag: String,
    #[serde(default)]
    pub personal: bool,
    #[serde(default)]
    pub line: String,
    #[serde(default)]
    pub excerpt: String,
    pub times: usize,
}

fn clean_aliases(aliases: Vec<String>) -> Vec<String> {
    aliases
        .into_iter()
        .map(|alias| cut(&suggestions::mask(alias.trim()), MAX_TITLE_CHARS))
        .filter(|alias| !alias.is_empty())
        .take(MAX_ALIASES)
        .collect()
}

/// Applies a read of `session` and returns what 제안 gets from it. Edits naming unknown cards or
/// kinds are dropped; ids of cards added here follow `next_card` in order.
pub fn apply_capture(
    memory: &mut Memory,
    session: &str,
    capture: Capture,
    given: &HashMap<u32, String>,
    docs: &[String],
) -> Vec<MemorySignal> {
    let now = suggestions::now();
    for edit in capture.cards {
        match edit {
            CardEdit::Add {
                kind,
                title,
                body,
                aliases,
            } => {
                let Some(kind) = Kind::parse(&kind) else {
                    eprintln!("Memory: card with unknown kind {kind:?} dropped");
                    continue;
                };
                let (title, body) = (title.trim(), body.trim());
                if title.is_empty() || body.is_empty() || memory.cards.len() >= MAX_CARDS {
                    continue;
                }
                let id = memory.next_card.max(1);
                memory.next_card = id + 1;
                memory.note(id, format!("m{id} 추가"));
                memory.cards.push(Card {
                    id,
                    kind,
                    title: cut(&suggestions::mask(title), MAX_TITLE_CHARS),
                    body: cut(&suggestions::mask(body), MAX_BODY_CHARS),
                    aliases: clean_aliases(aliases),
                    sources: vec![session.to_string()],
                    at: now,
                    archived_at: None,
                    promoted: Vec::new(),
                });
            }
            CardEdit::Update {
                id,
                title,
                body,
                aliases,
            } => {
                if memory.card(id).is_none() {
                    continue;
                }
                memory.note(id, format!("m{id} 고침"));
                let Some(card) = memory.card_mut(id) else {
                    continue;
                };
                if let Some(title) = title.filter(|title| !title.trim().is_empty()) {
                    card.title = cut(&suggestions::mask(title.trim()), MAX_TITLE_CHARS);
                }
                if let Some(body) = body.filter(|body| !body.trim().is_empty()) {
                    card.body = cut(&suggestions::mask(body.trim()), MAX_BODY_CHARS);
                }
                if let Some(aliases) = aliases {
                    card.aliases = clean_aliases(aliases);
                }
                if !card.sources.iter().any(|source| source == session) {
                    card.sources.push(session.to_string());
                }
                card.at = now;
            }
            CardEdit::Merge { id, into } => {
                if id == into || memory.card(id).is_none() || memory.card(into).is_none() {
                    continue;
                }
                let sources = memory
                    .card(id)
                    .map(|card| card.sources.clone())
                    .unwrap_or_default();
                memory.note(into, format!("m{id}을 m{into}에 합침"));
                if let Some(card) = memory.card_mut(into) {
                    for source in sources {
                        if !card.sources.contains(&source) {
                            card.sources.push(source);
                        }
                    }
                }
                memory.archive(id, &format!("m{into}에 합침"));
            }
        }
    }
    for conflict in capture.conflicts {
        if memory.card(conflict.card).is_some()
            && docs.contains(&conflict.doc)
            && !memory
                .conflicts
                .iter()
                .any(|known| known.card == conflict.card && known.doc == conflict.doc)
        {
            memory.conflicts.push(Conflict {
                says: cut(
                    &suggestions::mask(conflict.says.trim()),
                    MAX_TITLE_CHARS * 2,
                ),
                ..conflict
            });
        }
    }
    let title =
        |memory: &Memory, id: u32| memory.card(id).map(|card| format!("m{id} {}", card.title));
    let mut signals = Vec::new();
    for correction in capture.corrections {
        if let Some(tag) = title(memory, correction.card) {
            signals.push(MemorySignal {
                signal: Signal::Correction,
                tag,
                said: None,
                given: None,
            });
        }
    }
    for card in capture.ignored.into_iter().collect::<HashSet<_>>() {
        if let (Some(tag), Some(how)) = (title(memory, card), given.get(&card)) {
            signals.push(MemorySignal {
                signal: Signal::MemoryIgnored,
                tag,
                said: None,
                given: Some(how.clone()),
            });
        }
    }
    for repeat in capture.repeats {
        let said = Said {
            personal: repeat.personal,
            line: cut(&suggestions::mask(repeat.line.trim()), 400),
            excerpt: cut(&suggestions::mask(repeat.excerpt.trim()), 120),
        };
        for _ in 0..repeat.times.clamp(1, MAX_REPEATS) {
            signals.push(MemorySignal {
                signal: Signal::RepeatedRequest,
                tag: repeat.tag.clone(),
                said: Some(said.clone()),
                given: None,
            });
        }
    }
    signals
}

fn prompt(
    memory: &Memory,
    talk: &[(Option<i64>, String, String)],
    read_until: Option<i64>,
    given: &HashMap<u32, String>,
    docs: &[(String, String)],
    studying: &[String],
) -> String {
    let cards: Vec<String> = memory
        .cards
        .iter()
        .filter(|card| card.archived_at.is_none())
        .map(|card| {
            let given = given.get(&card.id).map_or(String::new(), |how| {
                format!(" (this session was given its {how})")
            });
            format!(
                "m{} [{:?}] {}{given}\n  aliases: {}\n  {}",
                card.id,
                card.kind,
                card.title,
                card.aliases.join(", "),
                cut(&card.body, 300).replace('\n', " ")
            )
        })
        .collect();
    let mut lines = Vec::new();
    for (at, who, text) in talk {
        let old = matches!((read_until, at), (Some(until), Some(at)) if *at <= until);
        let mark = if old { " (already read)" } else { "" };
        lines.push(format!("{who}{mark}: {}", text.replace('\n', " ")));
    }
    let mut talk_text = lines.join("\n");
    if talk_text.len() > MAX_TALK_BYTES {
        let mut start = talk_text.len() - MAX_TALK_BYTES;
        while !talk_text.is_char_boundary(start) {
            start += 1;
        }
        talk_text = talk_text[start..].to_string();
    }
    let docs_text: Vec<String> = docs
        .iter()
        .map(|(doc, head)| format!("--- {doc}\n{head}"))
        .collect();
    format!(
        "You keep the project memory of a coding repository: short cards of what the user and \
         their coding agents settled on, so later agents follow it without being told again.\n\
         Read the session below and answer with one JSON object only:\n\
         {{\"cards\":[{{\"op\":\"add\",\"kind\":\"rule|decision|lesson|fact|preference\",\
         \"title\":\"<=60 chars\",\"body\":\"what to do and why\",\"aliases\":[\"words a request \
         about it would use\"]}},{{\"op\":\"update\",\"id\":3,\"body\":\"...\"}},{{\"op\":\"merge\",\
         \"id\":4,\"into\":3}}],\
         \"corrections\":[{{\"card\":3}}],\
         \"repeats\":[{{\"tag\":\"what it means, short\",\"personal\":true,\"line\":\"one \
         instruction line that would have spared it\",\"excerpt\":\"the user's words\",\
         \"times\":2}}],\
         \"ignored\":[3],\
         \"conflicts\":[{{\"card\":3,\"doc\":\"AGENTS.md\",\"says\":\"what the document says \
         instead\"}}],\
         \"learn\":[{{\"quote\":\"the user's words, copied exactly\",\"title\":\"the concept, \
         <=40 chars, in the user's language\",\"why\":\"question|confused|corrected|unknown\"}}]}}\n\
         Rules:\n\
         - Only what the user decided, corrected or asked for, or a lesson the session paid for. \
         Not a summary of the work. Most sessions add nothing; empty lists are a fine answer.\n\
         - Never copy secrets, tokens, keys or personal data into a card.\n\
         - Before adding, update the card that already says it. Cards added here get ids from \
         m{next} in order, and corrections, ignored and conflicts may name them.\n\
         - corrections: the user corrected an agent and a card now holds the right way.\n\
         - repeats: the user asked for the same thing more than once in this session or as they \
         have before. times = how often in this session. personal = a preference of the user's \
         own (language, tone, how to report) rather than a fact of this repository.\n\
         - ignored: cards this session was given that the user still had to say again.\n\
         - conflicts: a card the documents below say otherwise about.\n\
         - learn: a concept the user did not know, judged by the user's own words only: they asked \
         what it is or how it works (question), said they were confused (confused), had it wrong \
         and an agent set it right (corrected), or said they don't know it (unknown). An agent \
         explaining at length is not enough. At most 3. quote is copied exactly from one USER \
         line not marked (already read). Skip concepts already on the STUDY LIST.\n\
         - Lines marked (already read) were read before: use them as context only, report \
         nothing from them again.\n\n\
         CARDS\n{cards}\n\nDOCUMENTS\n{docs}\n\nSTUDY LIST\n{studying}\n\nSESSION\n{talk_text}\n",
        next = memory.next_card.max(1),
        cards = if cards.is_empty() {
            "(none)".to_string()
        } else {
            cards.join("\n")
        },
        docs = if docs_text.is_empty() {
            "(none)".to_string()
        } else {
            docs_text.join("\n")
        },
        studying = if studying.is_empty() {
            "(none)".to_string()
        } else {
            studying.join("\n")
        },
    )
}

fn parse(output: &str) -> Option<Capture> {
    let start = output.find('{')?;
    let end = output.rfind('}')?;
    serde_json::from_str(output.get(start..=end)?).ok()
}

/// Reads `session` of `root` with the model and keeps what it found: the 학습함 proposals it added,
/// or `None` when it could not read.
pub fn capture(root: &Path, session: &str, claude: &Path) -> Option<usize> {
    let project = suggestions::load(root);
    let codex_last = load(root).codex.get(session).copied();
    let (agent, last) = match (codex_last, project.sessions.get(session).copied()) {
        (Some(last), _) => (CLIAgent::Codex, last),
        (None, Some(last)) => (CLIAgent::Claude, last),
        (None, None) => return None,
    };
    let mark_read = || {
        update(root, |memory| {
            memory.captured.insert(session.to_string(), last);
        });
    };
    let Some(path) = find_transcript(agent, session) else {
        // A transcript that is gone will never be read; it is not tried again.
        mark_read();
        return Some(0);
    };
    let Ok((jsonl, _)) = read_tail(&path, TAIL_BYTES) else {
        return None;
    };
    let talk = talk(agent, &jsonl);
    if !talk.iter().any(|(_, who, _)| who == "USER") {
        mark_read();
        return Some(0);
    }
    let naru = naru_dir()?;
    let memory = load(root);
    let receipts = receipts(root);
    let given: HashMap<u32, String> = receipts
        .iter()
        .filter(|receipt| receipt.session == session)
        .map(|receipt| (receipt.card, receipt.how.clone()))
        .collect();
    let read_until = memory.captured.get(session).copied();
    let docs_read = doc_heads(root);
    let docs: Vec<String> = docs_read.iter().map(|(doc, _)| doc.clone()).collect();
    let studying = crate::learn::known_titles(&naru);
    let request = prompt(&memory, &talk, read_until, &given, &docs_read, &studying);
    let mut answer = ask_claude(claude, CAPTURE_MODEL, &request, CAPTURE_TIMEOUT)
        .map_err(|error| eprintln!("Memory: capture of {session}: {error}"))
        .ok()
        .and_then(|output| parse(&output))?;
    let finds = std::mem::take(&mut answer.learn);
    let signals = update(root, |memory| {
        let signals = apply_capture(memory, session, answer, &given, &docs);
        memory.captured.insert(session.to_string(), last);
        tidy(memory, &receipts);
        signals
    })?;
    eprintln!("Memory: captured a session, signals={}", signals.len());
    if !signals.is_empty() {
        suggestions::record_memory(root, session, signals);
    }
    if finds.is_empty() {
        return Some(0);
    }
    let turns = transcript::turns(agent, &jsonl);
    Some(crate::learn::propose(
        &naru, root, agent, session, &path, &turns, read_until, finds,
    ))
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
