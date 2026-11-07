//! Agents that start other agents and write each other letters: short ids (a1, a2, …), letters
//! typed into a resting agent, the watch on supervised children, sleeping agents, and the
//! transcript checks for turns no hook reports the end of.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent::Agent;
use crate::core::{Core, Daemon};
use crate::grid::Draft;
use crate::letters::{self, Letters, Record, State};
use crate::pane::Pane;
use crate::session::{Restore, Session, Status};
use crate::state::{Direction, Node, PaneId, Workspace};
use crate::transcript;
use crate::ws::ServerMsg;

/// How long the user must leave the screen alone before a letter is typed into the pane they
/// are looking at.
const USER_IDLE: Duration = Duration::from_secs(3);
/// Delay between pasting text and pressing Enter, so the agent's input takes the paste first.
const ENTER_DELAY: Duration = Duration::from_millis(150);
/// A supervised child working this long without an agent event is reported to its parent.
const STALL: Duration = Duration::from_secs(20 * 60);
/// How long a parent has to act on its child's question or stall before the user is asked.
const ESCALATE: Duration = Duration::from_secs(60);
/// How long a letter may wait for its recipient before it is dropped and its sender told.
const LETTER_TTL: Duration = Duration::from_secs(60 * 60);
/// The largest letter body, so a runaway sender cannot flood a recipient's input.
const MAX_LETTER_BYTES: usize = 16 * 1024;
/// How long a resting agent may leave a typed letter untaken before it is typed again, and then
/// before its senders are told.
const RECORD_WAIT: Duration = Duration::from_secs(15);
/// How long a working agent must be silent before its transcript is read, and how often.
const QUIET: Duration = Duration::from_secs(5);
const TURN_TAIL: u64 = 256 * 1024;
/// How long after Codex's `Stop` its rollout is read, so the turn's last records are written.
const SETTLE: Duration = Duration::from_secs(1);
/// Sleepers older than this are forgotten: their pane was most likely closed while asleep.
const SLEEPER_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const DEFAULT_LINES: usize = 120;
const MAX_LINES: usize = 2000;
const CLIP: usize = 400;
/// A `naru agent wait` that has not asked for this long is gone (it asks every 2 seconds).
const WAITER_GONE: Duration = Duration::from_secs(10);
/// How long a waited-on child rests without a report before its waiter is told.
const REST_WAIT: Duration = Duration::from_secs(60);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Work handed to the recipient.
    Request,
    /// A question the sender cannot go on without.
    Block,
    /// The result of delegated work.
    Report,
    Progress,
    /// Naru telling an agent about its letters or children.
    #[serde(skip_deserializing)]
    Watch,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Request => "request",
            Kind::Block => "question",
            Kind::Report => "report",
            Kind::Progress => "progress",
            Kind::Watch => "watch",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Letter {
    /// Its number on record; Naru's own notes have none.
    id: Option<u32>,
    /// `None` is the user.
    from: Option<PaneId>,
    to: PaneId,
    kind: Kind,
    text: String,
    now: bool,
    at: Instant,
}

/// Runtime bookkeeping; parents (in the workspace), sleepers (`~/.naru/asleep.json`), and ids
/// and letters (`~/.naru/team.json`) outlive the daemon.
#[derive(Default)]
pub struct Team {
    ids: BTreeMap<PaneId, u32>,
    next_id: u32,
    /// Panes that ran an agent already; the next one is a new run with a new id.
    runs: BTreeSet<PaneId>,
    pending: Vec<Letter>,
    /// Letters typed into a pane that has not yet taken them in, and when.
    typed: BTreeMap<PaneId, (Instant, Vec<Letter>)>,
    /// Panes whose untaken letters were typed again once.
    retyped: BTreeSet<PaneId>,
    /// Panes whose last letter was typed but never taken in.
    unconfirmed: BTreeSet<PaneId>,
    /// Agents put to sleep → the command that wakes them.
    asleep: BTreeMap<PaneId, String>,
    /// A child's unanswered question to its parent, and when it asked.
    questions: BTreeMap<PaneId, (String, Instant)>,
    /// When each pane's agent last sent an event.
    heard: BTreeMap<PaneId, Instant>,
    /// Children whose parent Naru told that they stalled, and when.
    stalls: BTreeMap<PaneId, Instant>,
    /// Children that reported back; not watched until handed more work.
    reported: BTreeSet<PaneId>,
    /// Codex agents whose finished turn is read at the instant; letters to them wait.
    reading: BTreeMap<PaneId, Instant>,
    /// When each silent working agent's transcript was last read.
    checked: BTreeMap<PaneId, Instant>,
    pub letters: Letters,
    /// Sleepers woken for a letter, and when; letters to a sleeper expire only once it was woken.
    waking: BTreeMap<PaneId, Instant>,
    /// Ids or letters changed since `team.json` was last written.
    dirty: bool,
    /// Each agent's plan (`~/.naru/plans.json`).
    pub plans: crate::plans::Plans,
    /// Children a parent waits on with `naru agent wait`.
    waits: BTreeMap<PaneId, Wait>,
}

/// A parent's `naru agent wait` on a child: the report or question it caught instead of typing it
/// into the parent.
struct Wait {
    parent: PaneId,
    asked: Instant,
    caught: Option<Letter>,
}

/// `naru agent spawn`'s request.
pub struct Spawn {
    pub command: String,
    /// On a node, `None` is the node user's home.
    pub cwd: Option<String>,
    pub parent: bool,
    pub title: Option<String>,
    /// The ssh host to run on; `None` runs on this machine.
    pub machine: Option<String>,
    /// The step of the caller's plan the child takes.
    pub plan_node: Option<String>,
}

/// What a screen shows about a pane's place in its team.
#[derive(Serialize, Default, Clone, PartialEq, Debug)]
pub struct PaneTeam {
    pub id: Option<String>,
    /// Its unanswered question to its parent.
    pub question: Option<String>,
    /// How long it has been quiet, once Naru told its parent it stalled.
    pub stalled: Option<String>,
    /// Why it now waits on the user: its parent left a question or stall alone.
    pub alert: Option<String>,
    /// Letters waiting for it to stop working.
    pub held: usize,
    pub asleep: bool,
    /// Its last letter was typed in but never taken.
    pub not_taken: bool,
    pub plan: Option<crate::plans::PlanView>,
    /// Its plan waits for the user's review while it rests.
    pub review: Option<String>,
}

impl Team {
    pub fn id(&mut self, pane: PaneId) -> String {
        if let Some(id) = self.ids.get(&pane) {
            return format!("a{id}");
        }
        self.next_id += 1;
        self.ids.insert(pane, self.next_id);
        self.dirty = true;
        format!("a{}", self.next_id)
    }

    fn known_id(&self, pane: PaneId) -> Option<String> {
        self.ids.get(&pane).map(|id| format!("a{id}"))
    }

    pub fn pane_for(&self, id: &str) -> Option<PaneId> {
        let number: u32 = id.strip_prefix('a')?.parse().ok()?;
        self.ids
            .iter()
            .find(|(_, n)| **n == number)
            .map(|(pane, _)| *pane)
    }

    /// A new agent started in `pane`. Its first run keeps the id the pane already had, such as
    /// the one `naru agent spawn` returned, and so does an agent woken from sleep. `true` when it
    /// woke.
    pub fn start_run(&mut self, pane: PaneId) -> bool {
        let woke = self.asleep.remove(&pane).is_some();
        if !self.runs.insert(pane) && !woke {
            self.dirty |= self.ids.remove(&pane).is_some();
        }
        if woke {
            // The hour a letter may wait starts when its recipient is awake to take it.
            self.waking.remove(&pane);
            for letter in self.pending.iter_mut().filter(|letter| letter.to == pane) {
                letter.at = Instant::now();
            }
        }
        woke
    }

    /// Takes up the saved ids of the panes still open and the saved letters. Each of those panes
    /// counts as having run an agent, so a restored agent keeps its id and the next new one gets
    /// another.
    pub fn restore(&mut self, saved: letters::Saved, workspace: &Workspace) {
        for (pane, id) in saved.agents {
            if workspace.panes.contains_key(&pane) {
                self.ids.insert(pane, id);
                self.runs.insert(pane);
            }
        }
        self.next_id = self.ids.values().copied().fold(saved.next_agent, u32::max);
        self.letters = Letters::new(saved.next_letter, saved.letters);
    }

    /// `team.json`'s new contents, once something in it changed.
    pub fn take_saved(&mut self) -> Option<Vec<u8>> {
        if !std::mem::take(&mut self.dirty) {
            return None;
        }
        let saved = json!({
            "next_agent": self.next_id,
            "agents": self.ids,
            "next_letter": self.letters.next(),
            "letters": self.letters.all(),
        });
        serde_json::to_vec(&saved).ok()
    }

    fn mark(&mut self, letters: &[Letter], state: State, why: Option<&str>) {
        for id in letters.iter().filter_map(|letter| letter.id) {
            self.dirty |= self.letters.set(id, state, why);
        }
    }

    /// An agent event from `pane`: it is alive, and a new prompt means someone answered it and
    /// any letter typed into it was taken.
    pub fn heard(&mut self, pane: PaneId, prompt: bool) {
        self.heard.insert(pane, Instant::now());
        self.stalls.remove(&pane);
        if prompt {
            self.questions.remove(&pane);
            if let Some((_, taken)) = self.typed.remove(&pane) {
                self.mark(&taken, State::Taken, None);
            }
            self.unconfirmed.remove(&pane);
            self.retyped.remove(&pane);
        }
    }

    /// Starts watching a supervised child.
    pub fn watch(&mut self, child: PaneId) {
        self.heard.insert(child, Instant::now());
        self.reported.remove(&child);
    }

    /// Codex's turn ended; letters wait until its rollout says whether it left something open.
    pub fn read_codex_turn(&mut self, pane: PaneId) {
        self.reading.insert(pane, Instant::now() + SETTLE);
    }

    pub fn forget(&mut self, pane: PaneId) {
        self.waits.remove(&pane);
        self.heard.remove(&pane);
        self.checked.remove(&pane);
        self.reading.remove(&pane);
        self.typed.remove(&pane);
    }

    fn quiet_label(&self, child: PaneId) -> String {
        let quiet = self
            .heard
            .get(&child)
            .map_or(STALL.as_secs() / 60, |at| minutes_since(*at));
        format!("{quiet}분째 움직임 없음")
    }

    /// Why a supervised child now waits on the user: its parent left its question or stall alone
    /// for [`ESCALATE`].
    fn alert(&self, child: PaneId) -> Option<String> {
        if self.reported.contains(&child) {
            return None;
        }
        if let Some((_, at)) = self
            .questions
            .get(&child)
            .filter(|(_, at)| at.elapsed() >= ESCALATE)
        {
            return Some(format!(
                "질문 · 부모가 {}분째 답하지 않음",
                minutes_since(*at)
            ));
        }
        self.stalls
            .get(&child)
            .filter(|at| at.elapsed() >= ESCALATE)
            .map(|_| format!("멈춤 · {}", self.quiet_label(child)))
    }

    pub fn plan_view(
        &self,
        plan: &crate::plans::Plan,
        workspace: &Workspace,
    ) -> crate::plans::PlanView {
        let asking = self.questions.keys().copied().collect();
        crate::plans::view(plan, workspace, &asking, |pane| self.known_id(pane))
    }

    pub fn view(&self, workspace: &Workspace) -> BTreeMap<PaneId, PaneTeam> {
        workspace
            .panes
            .keys()
            .map(|pane| {
                let view = PaneTeam {
                    id: self.known_id(*pane),
                    question: self.questions.get(pane).map(|(text, _)| text.clone()),
                    stalled: self
                        .stalls
                        .contains_key(pane)
                        .then(|| self.quiet_label(*pane)),
                    alert: self.alert(*pane),
                    held: self.pending.iter().filter(|l| l.to == *pane).count(),
                    asleep: self.asleep.contains_key(pane),
                    not_taken: self.unconfirmed.contains(pane),
                    plan: self
                        .plans
                        .get(pane)
                        .map(|plan| self.plan_view(plan, workspace)),
                    review: self
                        .plans
                        .get(pane)
                        .and_then(|plan| crate::plans::review_line(plan, workspace, *pane)),
                };
                (*pane, view)
            })
            .filter(|(_, view)| *view != PaneTeam::default())
            .collect()
    }
}

/// Whole minutes since `at`, at least one.
fn minutes_since(at: Instant) -> u64 {
    (at.elapsed().as_secs() / 60).max(1)
}

/// Drops control characters other than newline and tab, so text cannot end the bracketed paste
/// it is typed in or press keys of its own.
pub fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// A letter as it is typed in.
struct Typed {
    id: Option<u32>,
    /// The sender's id and title; `None` is the user.
    from: Option<(String, String)>,
    kind: Kind,
    text: String,
    reply_to: Option<u32>,
}

fn letter_text(letters: &[Typed]) -> String {
    letters
        .iter()
        .map(|letter| {
            let Typed {
                id,
                from,
                kind,
                text,
                reply_to,
            } = letter;
            let kind = kind.word();
            match (from, id) {
                (None, _) => {
                    let number = id.map(|id| format!(" l{id}")).unwrap_or_default();
                    format!("[Naru letter{number} from the user · {kind}]\n{text}")
                }
                (Some((from, title)), Some(id)) => {
                    let answers = reply_to
                        .map(|original| format!(" · reply to l{original}"))
                        .unwrap_or_default();
                    format!(
                        "[Naru letter l{id} from {from} \"{title}\" · {kind}{answers}]\n{text}\n\
                         (reply with: naru reply l{id} \"<message>\")"
                    )
                }
                (Some((from, title)), None) => format!(
                    "[Naru letter from {from} \"{title}\" · {kind}]\n{text}\n\
                     (reply with: naru tell {from} \"<message>\")"
                ),
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The bytes for each key name `naru agent send --key` accepts. Anything else is refused, so an
/// agent cannot put arbitrary control sequences into another agent's input.
fn key_bytes(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "enter" => b"\r",
        "esc" => b"\x1b",
        "tab" => b"\t",
        "backspace" => b"\x7f",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        "ctrl-c" => b"\x03",
        "ctrl-d" => b"\x04",
        _ => return None,
    })
}

fn status_word(status: &Status) -> &'static str {
    match status {
        Status::InProgress => "working",
        Status::Success => "done",
        Status::Failed { .. } => "failed",
        Status::Blocked { .. } => "waiting",
        Status::Cancelled => "cancelled",
    }
}

fn status_name(status: &Status) -> &'static str {
    match status {
        Status::InProgress => "InProgress",
        Status::Success => "Success",
        Status::Failed { .. } => "Failed",
        Status::Blocked { .. } => "Blocked",
        Status::Cancelled => "Cancelled",
    }
}

fn clip(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(CLIP).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// What a pane is called in letters and listings.
pub fn pane_title(workspace: &Workspace, pane: PaneId) -> String {
    let Some(record) = workspace.panes.get(&pane) else {
        return String::new();
    };
    let tab = workspace
        .tab_of(pane)
        .and_then(|index| workspace.tabs[index].title.clone());
    record
        .title
        .clone()
        .or_else(|| record.auto_title.clone())
        .or(tab)
        .or_else(|| record.program_title.clone())
        .or_else(|| {
            record
                .cwd
                .as_deref()
                .and_then(|cwd| Path::new(cwd).file_name())
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

/// Writes `bytes` after [`ENTER_DELAY`], so the paste before them is taken first.
fn later(pane: &Arc<Pane>, bytes: Vec<u8>) {
    let pane = pane.clone();
    std::thread::spawn(move || {
        std::thread::sleep(ENTER_DELAY);
        let _ = pane.write(bytes);
    });
}

/// An agent put to sleep, in `~/.naru/asleep.json` by its pane id in hex.
#[derive(Serialize, Deserialize)]
struct Sleeper {
    agent: Agent,
    launcher: Option<String>,
    session_id: String,
    /// Unix seconds when it was put to sleep.
    #[serde(default)]
    since: u64,
    /// How it was started, so Codex wakes with the flags its hooks need.
    #[serde(default)]
    launch_command: Option<String>,
}

impl Sleeper {
    /// Built again on load rather than saved, so a value edited on disk still passes the checks.
    fn resume_command(&self) -> Option<String> {
        self.agent.resume_command_for_launch(
            self.launcher.as_deref(),
            &self.session_id,
            self.launch_command.as_deref(),
        )
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn sleepers_path(naru_dir: &Path) -> PathBuf {
    naru_dir.join("asleep.json")
}

fn read_sleepers(naru_dir: &Path) -> HashMap<String, Sleeper> {
    let path = sleepers_path(naru_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            eprintln!("ignoring unreadable {}: {error}", path.display());
            HashMap::new()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(error) => {
            eprintln!("cannot read {}: {error}", path.display());
            HashMap::new()
        }
    }
}

fn update_sleepers(naru_dir: &Path, change: impl FnOnce(&mut HashMap<String, Sleeper>)) {
    let path = sleepers_path(naru_dir);
    let mut sleepers = read_sleepers(naru_dir);
    change(&mut sleepers);
    let result = serde_json::to_string(&sleepers)
        .map_err(std::io::Error::other)
        .and_then(|text| std::fs::write(&path, text));
    if let Err(error) = result {
        eprintln!("cannot write {}: {error}", path.display());
    }
}

fn sleeper_key(pane: PaneId) -> String {
    pane.simple().to_string()
}

/// The transcript to read for a turn check, found outside the core lock.
struct TurnRead {
    pane: PaneId,
    agent: Agent,
    reported: Option<String>,
    session_id: Option<String>,
    heard: Option<Instant>,
}

impl TurnRead {
    fn tail(&self, home: &Path) -> Option<String> {
        let path = transcript::locate(
            self.agent,
            self.reported.as_deref(),
            self.session_id.as_deref(),
            home,
        )?;
        transcript::read_tail(&path, TURN_TAIL)
            .ok()
            .map(|(tail, _)| tail)
    }
}

impl Daemon {
    fn user_idle(&self) -> bool {
        self.last_input().is_none_or(|at| at.elapsed() >= USER_IDLE)
    }

    /// Brings back the sleepers saved for the restored panes and watches restored children.
    pub fn restore_team(&self, core: &mut Core) {
        let now = unix_now();
        update_sleepers(&self.naru_dir, |sleepers| {
            sleepers.retain(|_, sleeper| now.saturating_sub(sleeper.since) < SLEEPER_TTL.as_secs())
        });
        let sleepers = read_sleepers(&self.naru_dir);
        let Core {
            workspace, team, ..
        } = core;
        team.restore(letters::load(&self.naru_dir), workspace);
        team.plans = crate::plans::load(&self.naru_dir, workspace);
        for (pane, record) in &workspace.panes {
            if let Some(resume) = sleepers
                .get(&sleeper_key(*pane))
                .and_then(Sleeper::resume_command)
            {
                team.asleep.insert(*pane, resume);
            }
            if record.parent.is_some() {
                team.watch(*pane);
            }
        }
    }

    /// Forgets a sleeper that woke.
    pub fn woke(&self, pane: PaneId) {
        update_sleepers(&self.naru_dir, |sleepers| {
            sleepers.remove(&sleeper_key(pane));
        });
    }

    /// Drops undelivered letters and tells each live sender, once per recipient, why.
    fn drop_letters(core: &mut Core, letters: Vec<Letter>, why: &str) {
        core.team.mark(&letters, State::Dropped, Some(why));
        let mut told = BTreeSet::new();
        for letter in letters {
            let Some(from) = letter.from else {
                continue;
            };
            if letter.kind == Kind::Watch
                || !told.insert((from, letter.to))
                || !core.live.contains_key(&from)
            {
                continue;
            }
            let id = core
                .team
                .known_id(letter.to)
                .unwrap_or_else(|| "its recipient".to_owned());
            let number = letter.id.map(|n| format!(" l{n}")).unwrap_or_default();
            core.team.pending.push(Letter {
                id: None,
                from: Some(letter.to),
                to: from,
                kind: Kind::Watch,
                text: format!("Naru: your letter{number} to {id} was dropped: {why}."),
                now: false,
                at: Instant::now(),
            });
        }
    }

    /// Whether letters to `pane` must wait: its just-ended turn is still being read, or its input
    /// line holds text. A screen with no input line Naru knows does not hold them.
    fn input_holds_letters(core: &Core, pane: PaneId, session: Option<&Session>) -> bool {
        if core.team.reading.contains_key(&pane) {
            return true;
        }
        let reads_input = session.is_some_and(|s| matches!(s.agent, Agent::Claude | Agent::Codex));
        reads_input
            && core
                .live
                .get(&pane)
                .is_some_and(|live| matches!(live.draft(), Draft::Letter | Draft::Other))
    }

    /// Types each recipient's held letters into it once it has stopped working, or at once for a
    /// letter sent with `--now`. The pane the user is looking at only gets them once the user
    /// leaves the screen alone.
    pub fn deliver_letters(&self, core: &mut Core) {
        if core.team.pending.is_empty() {
            return;
        }
        let all = std::mem::take(&mut core.team.pending);
        let team = &core.team;
        // A sleeper's letters wait without a deadline until Naru wakes it for one of them.
        let (expired, waiting): (Vec<_>, Vec<_>) = all.into_iter().partition(|letter| {
            let since = match (
                team.asleep.contains_key(&letter.to),
                team.waking.get(&letter.to),
            ) {
                (true, None) => return false,
                (true, Some(woken)) => letter.at.max(*woken),
                (false, _) => letter.at,
            };
            since.elapsed() >= LETTER_TTL
        });
        core.team.pending = waiting;
        Self::drop_letters(core, expired, "waited 60 minutes without being delivered");
        let focused = core.workspace.focused();
        let idle = self.user_idle();
        let recipients: BTreeSet<PaneId> = core.team.pending.iter().map(|l| l.to).collect();
        for recipient in recipients {
            let Some(pane) = core.live.get(&recipient).cloned() else {
                let (closed, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut core.team.pending)
                    .into_iter()
                    .partition(|letter| letter.to == recipient);
                core.team.pending = waiting;
                Self::drop_letters(core, closed, "its recipient's pane was closed");
                continue;
            };
            let session = core
                .workspace
                .panes
                .get(&recipient)
                .and_then(|record| record.session.as_ref());
            if session.is_none() {
                Self::wake_for_letters(core, recipient, &pane, focused, idle);
                continue;
            }
            let urgent = core
                .team
                .pending
                .iter()
                .any(|letter| letter.to == recipient && letter.now);
            let ready = session.is_some_and(Session::at_rest)
                || (urgent && session.is_some_and(|s| s.status == Status::InProgress));
            if !ready
                || (Some(recipient) == focused && !idle)
                || Self::input_holds_letters(core, recipient, session)
            {
                continue;
            }
            let reports_prompts = session.is_some_and(Session::reports_prompts);
            let (letters, held): (Vec<_>, Vec<_>) = std::mem::take(&mut core.team.pending)
                .into_iter()
                .partition(|letter| letter.to == recipient);
            core.team.pending = held;
            // An agent that reports no prompts never says it took a letter in.
            let state = if reports_prompts {
                State::Typed
            } else {
                State::Taken
            };
            core.team.mark(&letters, state, None);
            if reports_prompts {
                let team = &mut core.team;
                team.unconfirmed.remove(&recipient);
                let (at, typed) = team
                    .typed
                    .entry(recipient)
                    .or_insert_with(|| (Instant::now(), Vec::new()));
                *at = Instant::now();
                typed.extend_from_slice(&letters);
            }
            let letters: Vec<_> = letters
                .into_iter()
                .map(|letter| Typed {
                    id: letter.id,
                    from: letter
                        .from
                        .map(|from| (core.team.id(from), pane_title(&core.workspace, from))),
                    kind: letter.kind,
                    reply_to: letter
                        .id
                        .and_then(|id| core.team.letters.get(id))
                        .and_then(|record| record.reply_to),
                    text: letter.text,
                })
                .collect();
            let paste = format!("\x1b[200~{}\x1b[201~", letter_text(&letters));
            if pane.write(paste.into_bytes()).is_ok() {
                later(&pane, b"\r".to_vec());
            }
        }
    }

    /// Wakes a sleeper that has a request or question waiting, once, unless the user is typing
    /// into it. Reports and progress wait until something else wakes it.
    fn wake_for_letters(
        core: &mut Core,
        recipient: PaneId,
        pane: &Arc<Pane>,
        focused: Option<PaneId>,
        idle: bool,
    ) {
        let team = &core.team;
        let wanted = team.pending.iter().any(|letter| {
            letter.to == recipient && matches!(letter.kind, Kind::Request | Kind::Block)
        });
        if !wanted || team.waking.contains_key(&recipient) || (Some(recipient) == focused && !idle)
        {
            return;
        }
        let Some(resume) = team.asleep.get(&recipient) else {
            return;
        };
        if pane.write(format!("{resume}\r").into_bytes()).is_ok() {
            core.team.waking.insert(recipient, Instant::now());
        }
    }

    /// A letter typed into a resting agent that never took it in: the agent may show a screen of
    /// its own (a trust or update prompt) that ate it, so it is tried once more; a letter still in
    /// the input only needs its Enter, and text someone else typed there is left alone.
    fn check_letters_taken(core: &mut Core, recipient: PaneId) {
        let Some(typed_at) = core.team.typed.get(&recipient).map(|(at, _)| *at) else {
            return;
        };
        let Some(session) = core
            .workspace
            .panes
            .get(&recipient)
            .and_then(|record| record.session.as_ref())
        else {
            core.team.typed.remove(&recipient);
            return;
        };
        let rested_long_enough = typed_at.elapsed() >= RECORD_WAIT
            && core
                .team
                .heard
                .get(&recipient)
                .is_none_or(|at| at.elapsed() >= RECORD_WAIT);
        // A working agent takes a letter typed during its turn once that turn ends.
        if !session.at_rest() || !rested_long_enough {
            return;
        }
        let Some((_, letters)) = core.team.typed.remove(&recipient) else {
            return;
        };
        let pane = core.live.get(&recipient).cloned();
        let draft = pane.as_ref().map_or(Draft::Unseen, |pane| pane.draft());
        let team = &mut core.team;
        if draft != Draft::Other && team.retyped.insert(recipient) {
            if draft == Draft::Letter {
                if let Some(pane) = &pane {
                    let _ = pane.write(b"\r".to_vec());
                }
                team.typed.insert(recipient, (Instant::now(), letters));
            } else {
                team.mark(&letters, State::Held, None);
                team.pending.extend(letters);
            }
            return;
        }
        team.retyped.remove(&recipient);
        team.unconfirmed.insert(recipient);
        team.mark(&letters, State::Untaken, None);
        let mut senders: Vec<PaneId> = letters
            .iter()
            .filter(|letter| letter.kind != Kind::Watch)
            .filter_map(|letter| letter.from)
            .collect();
        senders.sort_unstable();
        senders.dedup();
        let id = team.id(recipient);
        for sender in senders {
            if !core.live.contains_key(&sender) {
                continue;
            }
            core.team.pending.push(Letter {
                id: None,
                from: Some(recipient),
                to: sender,
                kind: Kind::Watch,
                text: format!(
                    "Naru: your letter to {id} was typed into its input, but {id} has not taken \
                     it in. It may be showing a screen of its own; look with naru agent read {id}."
                ),
                now: false,
                at: Instant::now(),
            });
        }
    }

    /// Tells the parent of each supervised child that has worked [`STALL`] without an event.
    fn watch_children(core: &mut Core) {
        let children: Vec<(PaneId, PaneId)> = core
            .workspace
            .panes
            .iter()
            .filter_map(|(child, record)| Some((*child, record.parent?)))
            .filter(|(child, parent)| {
                core.live.contains_key(child) && core.live.contains_key(parent)
            })
            .collect();
        for (child, parent) in children {
            let team = &core.team;
            if team.reported.contains(&child) || team.stalls.contains_key(&child) {
                continue;
            }
            let working = core.workspace.panes[&child]
                .session
                .as_ref()
                .is_some_and(|session| session.status == Status::InProgress);
            let quiet = team
                .heard
                .get(&child)
                .is_some_and(|at| at.elapsed() >= STALL);
            if !working || !quiet {
                continue;
            }
            let minutes = team.quiet_label(child);
            let team = &mut core.team;
            team.stalls.insert(child, Instant::now());
            team.pending.push(Letter {
                id: None,
                from: Some(child),
                to: parent,
                kind: Kind::Watch,
                text: format!(
                    "Naru watch: this agent you started has sent no progress ({minutes}). Check on it \
                     with naru agent list, or tell it what to do. If you do nothing, Naru asks the \
                     user in a minute."
                ),
                now: false,
                at: Instant::now(),
            });
        }
    }

    /// Forgets waits whose `naru agent wait` stopped asking; a letter one caught goes to the
    /// parent the usual way, so nothing is lost when the wait is cancelled.
    fn release_waits(core: &mut Core) {
        let gone: Vec<PaneId> = core
            .team
            .waits
            .iter()
            .filter(|(_, wait)| wait.asked.elapsed() >= WAITER_GONE)
            .map(|(child, _)| *child)
            .collect();
        for child in gone {
            let Some(Wait {
                caught: Some(letter),
                ..
            }) = core.team.waits.remove(&child)
            else {
                continue;
            };
            let team = &mut core.team;
            team.mark(std::slice::from_ref(&letter), State::Held, None);
            team.pending.push(Letter {
                at: Instant::now(),
                ..letter
            });
        }
    }

    /// Runs once a second: letters, the watch, and turns no hook reported the end of.
    pub fn tick(&self) {
        let now = Instant::now();
        let (silent, settled) = {
            let mut core = self.core();
            let typed: Vec<PaneId> = core.team.typed.keys().copied().collect();
            for pane in typed {
                Self::check_letters_taken(&mut core, pane);
            }
            Self::watch_children(&mut core);
            Self::release_waits(&mut core);
            let mut silent = Vec::new();
            let Core {
                workspace, team, ..
            } = &mut *core;
            for (pane, record) in &workspace.panes {
                let Some(session) = &record.session else {
                    continue;
                };
                if record.node.is_some()
                    || !transcript::readable(session.agent)
                    || !matches!(session.status, Status::InProgress | Status::Blocked { .. })
                {
                    continue;
                }
                let heard = team.heard.get(pane).copied();
                if heard.is_some_and(|at| at.elapsed() < QUIET)
                    || team
                        .checked
                        .get(pane)
                        .is_some_and(|at| at.elapsed() < QUIET)
                {
                    continue;
                }
                team.checked.insert(*pane, now);
                silent.push(TurnRead {
                    pane: *pane,
                    agent: session.agent,
                    reported: session.transcript_path.clone(),
                    session_id: session.session_id.clone(),
                    heard,
                });
            }
            let mut settled = Vec::new();
            for (pane, due) in &team.reading {
                if *due > now {
                    continue;
                }
                let session = workspace.panes.get(pane).and_then(|r| r.session.as_ref());
                settled.push(TurnRead {
                    pane: *pane,
                    agent: Agent::Codex,
                    reported: session.and_then(|s| s.transcript_path.clone()),
                    session_id: session.and_then(|s| s.session_id.clone()),
                    heard: None,
                });
            }
            (silent, settled)
        };
        // Transcripts are read without the lock: a pane's output must not wait on the disk.
        let interrupted: Vec<(PaneId, Option<Instant>)> = silent
            .into_iter()
            .filter(|read| {
                read.tail(&self.home).is_some_and(|tail| match read.agent {
                    Agent::Codex => transcript::codex_interrupted(&tail),
                    _ => transcript::claude_interrupted(&tail),
                })
            })
            .map(|read| (read.pane, read.heard))
            .collect();
        let waits: Vec<(PaneId, Option<&'static str>)> = settled
            .into_iter()
            .map(|read| {
                let wait = read
                    .tail(&self.home)
                    .and_then(|tail| transcript::codex_waits(&tail));
                (read.pane, wait)
            })
            .collect();
        let mut core = self.core();
        let mut changed = false;
        for (pane, heard) in interrupted {
            // An event since the read started makes the read stale.
            if core.team.heard.get(&pane).copied() != heard {
                continue;
            }
            if let Some(session) = core
                .workspace
                .panes
                .get_mut(&pane)
                .and_then(|record| record.session.as_mut())
            {
                changed |= session.cancel_turn();
            }
        }
        for (pane, wait) in waits {
            core.team.reading.remove(&pane);
            if let (Some(message), Some(session)) = (
                wait,
                core.workspace
                    .panes
                    .get_mut(&pane)
                    .and_then(|record| record.session.as_mut()),
            ) {
                changed |= session.hold(message);
            }
        }
        self.deliver_letters(&mut core);
        if changed {
            self.changed(&core);
        } else {
            self.send_if_team_changed(&core);
        }
        let Some(saved) = core.team.take_saved() else {
            return;
        };
        let message = Self::letters_text(&core);
        drop(core);
        letters::save(&self.naru_dir, &saved);
        let _ = self.events.send(message);
    }

    fn letters_text(core: &Core) -> Arc<str> {
        ServerMsg::Letters {
            letters: core.team.letters.views(),
        }
        .text()
    }

    /// Every letter on record, for a screen that asks.
    pub fn letters_message(&self) -> Arc<str> {
        Self::letters_text(&self.core())
    }

    fn resolve(core: &Core, to: &str) -> Result<PaneId, String> {
        let pane = core
            .team
            .pane_for(to)
            .ok_or_else(|| format!("no agent {to}; see naru agent list"))?;
        if !core.live.contains_key(&pane) {
            return Err(format!("{to} is closed"));
        }
        Ok(pane)
    }

    fn resolve_other(core: &Core, from: PaneId, to: &str) -> Result<PaneId, String> {
        let pane = Self::resolve(core, to)?;
        if pane == from {
            return Err("cannot read or type into your own pane".into());
        }
        Ok(pane)
    }

    /// Opens `command` in a new session card, leaving the user's focus where it is: a child's or a
    /// node's card after the sender's family, any other card in the project of its folder.
    pub fn op_spawn(self: &Arc<Self>, from: PaneId, spawn: Spawn) -> Result<Value, String> {
        let Spawn {
            command,
            cwd,
            parent,
            title,
            machine,
            plan_node,
        } = spawn;
        if plan_node.is_some() && !parent {
            return Err("--plan-node needs --parent here: the step's child reports to you".into());
        }
        let (command, cwd, node) = match machine {
            Some(host) => {
                if !crate::nodes::is_host(&host) {
                    return Err(format!("not an ssh host: {host}"));
                }
                let first = Some(command).filter(|command| !command.trim().is_empty());
                let node = Node {
                    host,
                    cwd,
                    resume: None,
                    first,
                };
                (crate::nodes::ATTACH.to_owned(), None, Some(node))
            }
            None => {
                if command.trim().is_empty() {
                    return Err("empty command".into());
                }
                let cwd = cwd.ok_or("no working directory")?;
                if !Path::new(&cwd).is_dir() {
                    return Err(format!("not a directory: {cwd}"));
                }
                (command, Some(cwd), None)
            }
        };
        let mut core = self.core();
        if let Some(key) = &plan_node {
            let Core { team, live, .. } = &mut *core;
            crate::plans::handable(team.plans.get_mut(&from), key, |pane| {
                live.contains_key(&pane)
            })?;
        }
        let workspace = &mut core.workspace;
        let sender = workspace.tab_of(from).ok_or("the sending pane is gone")?;
        let (at, group) = if parent || node.is_some() {
            // Right after the sender and the cards that already hang under it, in the sender's
            // project even when the child works in another folder.
            let mut index = sender + 1;
            while workspace.tabs.get(index).is_some_and(|tab| {
                tab.layout
                    .panes()
                    .iter()
                    .any(|pane| workspace.descends_from(*pane, from))
            }) {
                index += 1;
            }
            (index, workspace.tabs[sender].group)
        } else {
            // Unrelated work keeps the project its folder puts it in, after that project's cards.
            let group = cwd.as_deref().and_then(|cwd| workspace.group_for(cwd));
            let at = group.map_or(workspace.tabs.len(), |group| {
                workspace.end_of_group(Some(group))
            });
            (at, group)
        };
        let (_, pane) = workspace.insert_tab(at, group, cwd);
        workspace.tabs[at].title = title.map(|t| sanitize(&t)).filter(|t| !t.is_empty());
        let record = workspace.panes.get_mut(&pane).expect("new pane");
        record.parent = parent.then_some(from);
        record.node = node;
        self.spawn_or_remove(&mut core, pane)
            .map_err(|error| format!("cannot start a shell: {error:#}"))?;
        core.pending.insert(pane, Restore::Run(command));
        if parent {
            core.team.watch(pane);
        }
        if let Some(key) = &plan_node {
            let Core { team, live, .. } = &mut *core;
            if let Ok(step) = crate::plans::handable(team.plans.get_mut(&from), key, |pane| {
                live.contains_key(&pane)
            }) {
                step.child = Some(pane);
                step.reported = false;
            }
            self.save_plans(&mut core);
        }
        let id = core.team.id(pane);
        self.changed(&core);
        Ok(json!({ "id": id }))
    }

    pub fn op_tell(
        &self,
        from: PaneId,
        to: &str,
        kind: Kind,
        text: &str,
        now: bool,
    ) -> Result<Value, String> {
        let mut core = self.core();
        let recipient = if to == "parent" {
            core.workspace
                .panes
                .get(&from)
                .and_then(|r| r.parent)
                .ok_or("this agent has no parent")?
        } else {
            core.team
                .pane_for(to)
                .ok_or_else(|| format!("no agent {to}; see naru agent list"))?
        };
        if recipient == from {
            return Err("cannot write to yourself".into());
        }
        if !core.live.contains_key(&recipient) {
            return Err(format!("{to} is closed"));
        }
        let (id, delivered) =
            self.send_letter(&mut core, Some(from), recipient, kind, text, now, None)?;
        let to = core.team.id(recipient);
        Ok(json!({ "id": format!("l{id}"), "to": to, "delivered": delivered }))
    }

    /// One ask of `naru agent wait <to>`: the child's report or question if one came since the
    /// wait began (it is not typed into the caller then), `closed`, `rested` once it has rested
    /// [`REST_WAIT`] without either, or else `waiting`.
    pub fn op_wait(&self, from: PaneId, to: &str) -> Result<Value, String> {
        let mut core = self.core();
        let child = core
            .team
            .pane_for(to)
            .ok_or_else(|| format!("no agent {to}; see naru agent list"))?;
        if core.workspace.panes.get(&child).and_then(|r| r.parent) != Some(from) {
            return Err(format!(
                "{to} is not your child; only a parent waits on an agent"
            ));
        }
        let team = &mut core.team;
        if let Some(letter) = team.waits.get_mut(&child).and_then(|w| w.caught.take()) {
            team.waits.remove(&child);
            let state = if letter.kind == Kind::Block {
                "question"
            } else {
                "report"
            };
            let id = letter.id.map(|n| format!("l{n}"));
            return Ok(json!({ "state": state, "letter": id, "text": letter.text }));
        }
        if !core.live.contains_key(&child) {
            core.team.waits.remove(&child);
            return Ok(json!({ "state": "closed" }));
        }
        let rested = core
            .workspace
            .panes
            .get(&child)
            .and_then(|r| r.session.as_ref())
            .is_some_and(Session::at_rest)
            && core
                .team
                .heard
                .get(&child)
                .is_some_and(|at| at.elapsed() >= REST_WAIT);
        if rested {
            core.team.waits.remove(&child);
            return Ok(json!({ "state": "rested" }));
        }
        core.team.waits.insert(
            child,
            Wait {
                parent: from,
                asked: Instant::now(),
                caught: None,
            },
        );
        Ok(json!({ "state": "waiting" }))
    }

    /// Answers letter `letter` (l7) to its sender. Only the agent it was written to may.
    pub fn op_reply(
        &self,
        from: PaneId,
        letter: &str,
        kind: Option<Kind>,
        text: &str,
        now: bool,
    ) -> Result<Value, String> {
        let number: u32 = letter
            .strip_prefix('l')
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("not a letter number: {letter}; letters are l1, l2, …"))?;
        let mut core = self.core();
        let original = core
            .team
            .letters
            .get(number)
            .cloned()
            .ok_or_else(|| format!("no letter {letter}; letters are kept for a week"))?;
        if original.to != from {
            return Err(format!("{letter} was not written to you"));
        }
        let recipient = original
            .from
            .ok_or_else(|| format!("{letter} came from the user; answer in your own pane"))?;
        if !core.live.contains_key(&recipient) {
            return Err(format!("{} is closed", original.from_name));
        }
        // An answer to a request is its result; anything else answered asks for more.
        let kind = kind.unwrap_or(if original.kind == Kind::Request {
            Kind::Report
        } else {
            Kind::Request
        });
        let (id, delivered) = self.send_letter(
            &mut core,
            Some(from),
            recipient,
            kind,
            text,
            now,
            Some(number),
        )?;
        let to = core.team.id(recipient);
        Ok(json!({ "id": format!("l{id}"), "to": to, "delivered": delivered }))
    }

    /// The user's letter to the agent in `to`, written in 한눈에 보기.
    pub fn letter_send(&self, to: PaneId, text: &str) -> anyhow::Result<()> {
        let mut core = self.core();
        if !core.live.contains_key(&to) {
            anyhow::bail!("that pane is closed");
        }
        self.send_letter(&mut core, None, to, Kind::Request, text, false, None)
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }

    /// Puts a dropped or untaken letter back in its recipient's queue, with a fresh hour.
    pub fn letter_retry(&self, id: u32) -> anyhow::Result<()> {
        let mut core = self.core();
        let record = core
            .team
            .letters
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("l{id} is no longer on record"))?;
        if !matches!(record.state, State::Dropped | State::Untaken) {
            anyhow::bail!("l{id} was not left undelivered");
        }
        if !core.live.contains_key(&record.to) {
            anyhow::bail!("{} is closed", record.to_name);
        }
        let team = &mut core.team;
        team.dirty |= team.letters.set(id, State::Held, None);
        team.unconfirmed.remove(&record.to);
        team.pending.push(Letter {
            id: Some(id),
            from: record.from,
            to: record.to,
            kind: record.kind,
            text: record.text,
            now: false,
            at: Instant::now(),
        });
        self.deliver_letters(&mut core);
        self.send_if_team_changed(&core);
        Ok(())
    }

    /// Records a letter and queues it for `recipient`, typing it in at once if it can be. `from`
    /// is `None` for the user. Returns its number and whether it was typed in.
    #[allow(clippy::too_many_arguments)]
    fn send_letter(
        &self,
        core: &mut Core,
        from: Option<PaneId>,
        recipient: PaneId,
        kind: Kind,
        text: &str,
        now: bool,
        reply_to: Option<u32>,
    ) -> Result<(u32, bool), String> {
        let text = sanitize(text);
        if text.is_empty() {
            return Err("empty letter".into());
        }
        if text.len() > MAX_LETTER_BYTES {
            return Err(format!(
                "letter is {} bytes, over the {MAX_LETTER_BYTES}-byte limit; write it to a file and \
                 send its path",
                text.len()
            ));
        }
        let mut plans_changed = false;
        if let Some(from) = from {
            let from_parent = core.workspace.panes.get(&from).and_then(|r| r.parent);
            let recipient_parent = core.workspace.panes.get(&recipient).and_then(|r| r.parent);
            let team = &mut core.team;
            if from_parent == Some(recipient) {
                match kind {
                    Kind::Block => {
                        team.questions.insert(from, (text.clone(), Instant::now()));
                        team.reported.remove(&from);
                    }
                    Kind::Report => {
                        team.questions.remove(&from);
                        team.stalls.remove(&from);
                        team.reported.insert(from);
                        plans_changed |=
                            crate::plans::letter(&mut team.plans, recipient, from, true);
                    }
                    Kind::Request | Kind::Progress | Kind::Watch => {}
                }
            }
            if recipient_parent == Some(from) {
                team.questions.remove(&recipient);
                team.stalls.remove(&recipient);
                if kind == Kind::Request {
                    team.reported.remove(&recipient);
                    plans_changed |= crate::plans::letter(&mut team.plans, from, recipient, false);
                }
            }
        }
        let at = letters::unix_now();
        let team = &mut core.team;
        let record = Record {
            id: 0,
            from,
            to: recipient,
            from_name: from.map_or_else(|| "나".to_owned(), |from| team.id(from)),
            to_name: team.id(recipient),
            from_title: from
                .map(|from| pane_title(&core.workspace, from))
                .unwrap_or_default(),
            to_title: pane_title(&core.workspace, recipient),
            kind,
            text: text.clone(),
            at,
            reply_to,
            state: State::Held,
            why: None,
            state_at: at,
        };
        let id = team.letters.add(record);
        team.dirty = true;
        let letter = Letter {
            id: Some(id),
            from,
            to: recipient,
            kind,
            text,
            now,
            at: Instant::now(),
        };
        // A parent waiting on this child takes its report or question from `naru agent wait`.
        let wait = from
            .filter(|_| matches!(kind, Kind::Report | Kind::Block))
            .and_then(|from| team.waits.get_mut(&from))
            .filter(|wait| {
                wait.parent == recipient
                    && wait.caught.is_none()
                    && wait.asked.elapsed() < WAITER_GONE
            });
        if let Some(wait) = wait {
            wait.caught = Some(letter.clone());
            team.mark(std::slice::from_ref(&letter), State::Taken, None);
            if plans_changed {
                self.save_plans(core);
            }
            self.send_if_team_changed(core);
            return Ok((id, true));
        }
        team.pending.push(letter);
        if plans_changed {
            self.save_plans(core);
        }
        self.deliver_letters(core);
        let held = core.team.pending.iter().any(|l| l.to == recipient);
        self.send_if_team_changed(core);
        Ok((id, !held))
    }

    pub fn op_list(&self, from: PaneId) -> Value {
        let mut core = self.core();
        json!({ "agents": Self::agents(&mut core, from) })
    }

    fn agents(core: &mut Core, from: PaneId) -> Vec<Value> {
        let Core {
            workspace, team, ..
        } = core;
        let mut rows = Vec::new();
        for tab in &workspace.tabs {
            let project = tab
                .group
                .and_then(|id| workspace.groups.iter().find(|group| group.id == id))
                .and_then(|group| group.path.clone());
            for pane in tab.layout.panes() {
                let record = &workspace.panes[&pane];
                if record.session.is_none() && pane != from && record.parent.is_none() {
                    continue;
                }
                let session = record.session.as_ref();
                rows.push(json!({
                    "id": team.id(pane),
                    "title": pane_title(workspace, pane),
                    "agent": session.map(|s| format!("{:?}", s.agent)),
                    "status": session.map(|s| status_name(&s.status)),
                    "parent": record.parent.map(|parent| team.id(parent)),
                    "you": pane == from,
                    "machine": record.node.as_ref().map(|node| &node.host),
                    "project": project,
                }));
            }
        }
        rows
    }

    pub fn op_projects(&self, from: PaneId) -> Value {
        let mut core = self.core();
        let projects: Vec<Value> = core
            .workspace
            .groups
            .iter()
            .filter_map(|group| {
                let path = group.path.as_ref()?;
                Some(json!({ "name": group.name, "path": path }))
            })
            .collect();
        json!({ "projects": projects, "agents": Self::agents(&mut core, from) })
    }

    /// What `to` is doing and what it waits on.
    pub fn op_show(&self, from: PaneId, to: &str) -> Result<Value, String> {
        let mut core = self.core();
        let pane = Self::resolve(&core, to)?;
        let Core {
            workspace, team, ..
        } = &mut *core;
        let record = &workspace.panes[&pane];
        let session = record.session.as_ref();
        let status = match session {
            Some(session) => Some(status_word(&session.status)),
            None => team.asleep.contains_key(&pane).then_some("asleep"),
        };
        let waiting_on = session.and_then(|session| match &session.status {
            Status::Blocked { message } | Status::Failed { message, .. } => message.clone(),
            Status::InProgress | Status::Success | Status::Cancelled => None,
        });
        let children: Vec<PaneId> = workspace
            .panes
            .iter()
            .filter(|(_, r)| r.parent == Some(pane))
            .map(|(child, _)| *child)
            .collect();
        let children: Vec<String> = children.into_iter().map(|child| team.id(child)).collect();
        Ok(json!({
            "id": to,
            "you": pane == from,
            "title": pane_title(workspace, pane),
            "agent": session.map(|s| format!("{:?}", s.agent)),
            "status": status,
            "waiting_on": waiting_on,
            "cwd": session.and_then(|s| s.cwd.clone()).or_else(|| record.cwd.clone()),
            "session_id": session.and_then(|s| s.session_id.clone()),
            "last_prompt": session.and_then(|s| s.query.as_deref()).map(clip),
            "last_reply": session
                .and_then(|s| s.response.as_deref().or(s.summary.as_deref()))
                .map(clip),
            "quiet_seconds": team.heard.get(&pane).map(|at| at.elapsed().as_secs()),
            "parent": record.parent.map(|parent| team.id(parent)),
            "children": children,
            "question_to_parent": team.questions.get(&pane).map(|(question, _)| question),
            "stalled": team.stalls.contains_key(&pane),
            "letters_held": team.pending.iter().filter(|letter| letter.to == pane).count(),
            "letter_not_taken": team.unconfirmed.contains(&pane),
        }))
    }

    /// Every pane of every tab, for `naru ps`.
    pub fn op_ps(&self, from: PaneId) -> Value {
        let mut core = self.core();
        let Core {
            workspace, team, ..
        } = &mut *core;
        let mut rows = Vec::new();
        for (index, tab) in workspace.tabs.iter().enumerate() {
            for pane in tab.layout.panes() {
                let record = &workspace.panes[&pane];
                let session = record.session.as_ref();
                let status = session
                    .map(|s| status_word(&s.status))
                    .or_else(|| team.asleep.contains_key(&pane).then_some("asleep"));
                rows.push(json!({
                    "tab": index + 1,
                    "id": team.id(pane),
                    "kind": "terminal",
                    "title": pane_title(workspace, pane),
                    "agent": session.map(|s| format!("{:?}", s.agent)),
                    "status": status,
                    "parent": record.parent.map(|parent| team.id(parent)),
                    "you": pane == from,
                }));
            }
        }
        json!({ "panes": rows })
    }

    /// Ends a resting agent so it holds no memory, keeping how to resume it for `naru agent wake`,
    /// across restarts too.
    pub fn op_sleep(&self, from: PaneId, to: &str) -> Result<Value, String> {
        let mut core = self.core();
        let pane = Self::resolve_other(&core, from, to)?;
        let session = core.workspace.panes[&pane]
            .session
            .as_ref()
            .ok_or_else(|| format!("{to} runs no agent"))?;
        if !matches!(session.agent, Agent::Claude | Agent::Codex) {
            return Err("only claude and codex agents can sleep".into());
        }
        if !session.at_rest() {
            return Err(format!(
                "{to} is {}; it can sleep once it stops",
                status_word(&session.status)
            ));
        }
        let session_id = session
            .session_id
            .clone()
            .ok_or_else(|| format!("{to} has not reported its session id, so it could not wake"))?;
        let sleeper = Sleeper {
            agent: session.agent,
            launcher: session.launcher.clone(),
            session_id,
            since: unix_now(),
            launch_command: session.launch_command.clone(),
        };
        let resume = sleeper
            .resume_command()
            .ok_or_else(|| format!("{to} cannot be resumed from its session id"))?;
        core.team.asleep.insert(pane, resume);
        update_sleepers(&self.naru_dir, |sleepers| {
            sleepers.insert(sleeper_key(pane), sleeper);
        });
        let live = core.live[&pane].clone();
        // Ctrl-U first, so a draft left in the agent's input does not swallow the command.
        live.write(b"\x15/exit".to_vec())
            .map_err(|e| e.to_string())?;
        later(&live, b"\r".to_vec());
        self.send_if_team_changed(&core);
        Ok(json!({ "to": to }))
    }

    pub fn op_wake(&self, from: PaneId, to: &str) -> Result<Value, String> {
        let core = self.core();
        let pane = Self::resolve_other(&core, from, to)?;
        let resume = core
            .team
            .asleep
            .get(&pane)
            .cloned()
            .ok_or_else(|| format!("{to} is not asleep"))?;
        if core.workspace.panes[&pane].session.is_some() {
            return Err(format!("{to} is still running its agent"));
        }
        core.live[&pane]
            .write(format!("{resume}\r").into_bytes())
            .map_err(|e| e.to_string())?;
        Ok(json!({ "to": to, "command": resume }))
    }

    /// Runs `command` in a new pane right of `from`, leaving the focus where it is.
    pub fn op_pane_open(self: &Arc<Self>, from: PaneId, command: &str) -> Result<Value, String> {
        let command = command.trim();
        if command.is_empty() {
            return Err("empty command".into());
        }
        let mut core = self.core();
        if core.workspace.tab_of(from).is_none() {
            return Err("the sending pane is gone".into());
        }
        let pane = core
            .workspace
            .split(from, Direction::Right, false)
            .map_err(|e| e.to_string())?;
        self.spawn_or_remove(&mut core, pane)
            .map_err(|error| format!("cannot start a shell: {error:#}"))?;
        core.pending.insert(pane, Restore::Run(command.to_owned()));
        let id = core.team.id(pane);
        self.changed(&core);
        Ok(json!({ "id": id }))
    }

    pub fn op_read(&self, from: PaneId, to: &str, lines: Option<usize>) -> Result<Value, String> {
        let core = self.core();
        let pane = Self::resolve_other(&core, from, to)?;
        let lines = lines.unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
        Ok(json!({ "to": to, "text": core.live[&pane].read(lines) }))
    }

    pub fn op_send(
        &self,
        from: PaneId,
        to: &str,
        text: &str,
        keys: &[String],
        enter: bool,
    ) -> Result<Value, String> {
        let core = self.core();
        let pane = Self::resolve_other(&core, from, to)?;
        let mut after = Vec::new();
        for key in keys {
            let bytes = key_bytes(key).ok_or_else(|| {
                format!(
                    "unknown key {key}; use enter, esc, tab, backspace, up, down, left, right, \
                     ctrl-c or ctrl-d"
                )
            })?;
            after.extend_from_slice(bytes);
        }
        if enter {
            after.push(b'\r');
        }
        let text = sanitize(text);
        if text.is_empty() && after.is_empty() {
            return Err("nothing to send".into());
        }
        let live = core.live[&pane].clone();
        if text.is_empty() {
            live.write(after).map_err(|e| e.to_string())?;
        } else {
            live.paste(&text).map_err(|e| e.to_string())?;
            if !after.is_empty() {
                later(&live, after);
            }
        }
        Ok(json!({ "to": to }))
    }
}

#[cfg(test)]
#[path = "team_tests.rs"]
mod tests;
