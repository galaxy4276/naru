//! Naru AI in the daemon: the 제안 and Memory jobs the tick runs, and what the panel shows and
//! presses. The stores and their rules live in `suggestions`, `memory` and `global_instruction`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent::Agent;
use crate::core::Daemon;
use crate::global_instruction;
use crate::memory::{self, Memory};
use crate::session::Status;
use crate::state::PaneId;
use crate::suggestions::{self, Fix, Line, Project, Settings, Stage};
use crate::titles::claude_path;

/// Ticks (one a second) between looks at running fixes, waits and auto-prepare.
const LOOK_TICKS: u32 = 5;
/// Ticks between looks for sessions Memory should read.
const CAPTURE_TICKS: u32 = 60;
/// Ticks between looks for a project whose day's investigation is due.
const DUE_TICKS: u32 = 300;
/// Reads one `다시 확인` runs at most.
const CHECK_NOW_SESSIONS: usize = 5;

#[derive(Default)]
pub struct Ai {
    ticks: AtomicU32,
    investigating: Mutex<Option<PathBuf>>,
    capturing: AtomicBool,
    claude_missing: AtomicBool,
    /// Fixes an agent is making, by project and line: its pane, and whether it has started work.
    fixing: Mutex<HashMap<(PathBuf, u32), (PaneId, bool)>>,
    /// Sessions already noted as waiting, with how.
    waits_seen: Mutex<HashSet<(String, &'static str)>>,
    github: Mutex<HashMap<PathBuf, bool>>,
    issuing: Mutex<HashSet<(PathBuf, u32)>>,
    /// The repository of the pane the panel last looked at, which auto-prepare works in.
    front: Mutex<Option<PathBuf>>,
}

/// A press in the Naru AI panel.
#[derive(Debug, Deserialize)]
#[serde(tag = "a", rename_all = "snake_case")]
pub enum Act {
    Fix {
        line: u32,
    },
    Issue {
        line: u32,
    },
    Skip {
        line: u32,
    },
    Keep {
        line: u32,
    },
    Revert {
        line: u32,
    },
    Restore {
        line: u32,
    },
    ShowDiff {
        line: u32,
    },
    /// `seen` is the stamp of the file the preview showed, as a decimal string.
    ApplyGlobal {
        line: u32,
        seen: String,
    },
    RevertGlobal {
        line: u32,
    },
    Settings {
        settings: Settings,
    },
    Archive {
        card: u32,
    },
    Revive {
        card: u32,
    },
    /// Puts a card an agent held back into the index and the request matches.
    Release {
        card: u32,
    },
    /// `N개 문서에 반영`: a 제안 line per document the card is not in yet.
    Promote {
        card: u32,
    },
    Undo {
        change: u32,
    },
    /// A conflict by index, and whether the card is right.
    Settle {
        index: usize,
        card_right: bool,
    },
    CheckNow,
    Forget,
}

#[derive(Serialize)]
pub struct Received {
    how: String,
    at: i64,
}

#[derive(Serialize)]
pub struct Global {
    lines: usize,
    bytes: usize,
    /// The file's stamp as a decimal string; JavaScript numbers cannot hold a u64.
    stamp: String,
    /// Why each new global line cannot go in, by line.
    refused: BTreeMap<u32, String>,
    error: Option<String>,
}

/// What the panel shows for the repository of one pane.
#[derive(Serialize)]
pub struct View {
    root: PathBuf,
    project: Project,
    memory: Memory,
    /// Cards agents reported out of date, still held.
    held: BTreeMap<u32, memory::Hold>,
    /// Each card's newest receipt.
    received: BTreeMap<u32, Received>,
    /// When each card last went out with its body, else last changed.
    used: HashMap<u32, i64>,
    docs: Vec<String>,
    github: bool,
    investigating: bool,
    capturing: bool,
    claude_missing: bool,
    /// Lines an agent is fixing, with its pane.
    fixing: BTreeMap<u32, PaneId>,
    global: Global,
    badge: usize,
    /// Lines the user can pick from.
    choosable: Vec<u32>,
    auto_waiting: bool,
}

/// An agent pane on this machine.
struct LocalSession {
    id: String,
    agent: Agent,
    status: Status,
    /// Whether it waits on a tool's permission rather than a question.
    tool: bool,
    root: Option<PathBuf>,
}

fn repo_of(cwd: Option<&str>) -> Option<PathBuf> {
    suggestions::repo_root(Path::new(cwd?))
}

fn global_view(project: &Project) -> Global {
    let (text, error) = match global_instruction::read() {
        Ok(text) => (text, None),
        Err(error) => (
            String::new(),
            Some(format!("파일을 읽지 못했어요: {error}")),
        ),
    };
    let refused = project
        .lines
        .iter()
        .filter_map(|line| match &line.fix {
            Some(Fix::Global { line: words }) => global_instruction::check_line(words)
                .err()
                .map(|why| (line.id, why)),
            Some(Fix::Doc { .. } | Fix::Issue { .. }) | None => None,
        })
        .collect();
    Global {
        lines: text.lines().filter(|line| !line.trim().is_empty()).count(),
        bytes: text.len(),
        stamp: global_instruction::stamp(&text).to_string(),
        refused,
        error,
    }
}

fn set_line(root: &Path, id: u32, edit: impl FnOnce(&mut Line)) {
    suggestions::update(root, |project| project.line_mut(id).map(edit));
}

fn mark_applied(line: &mut Line) {
    line.stage = Stage::Applied;
    line.applied_at = Some(suggestions::now());
    line.came_back = false;
    line.refusal = None;
}

fn refuse(root: &Path, id: u32, why: &str) {
    set_line(root, id, |line| line.refusal = Some(why.to_owned()));
}

/// Settles a fix whose agent stopped or whose pane is gone: what it changed, or that it changed
/// nothing.
fn finish_fix(root: &Path, id: u32) {
    let Some(Some(Fix::Doc { path, .. })) = suggestions::load(root)
        .line(id)
        .filter(|line| line.stage == Stage::Fixing)
        .map(|line| line.fix.clone())
    else {
        return;
    };
    // Outside the store lock: git can be slow and every write waits on it.
    let change = suggestions::file_change(root, &path);
    set_line(root, id, |line| {
        if line.stage == Stage::Fixing {
            line.stage = if change.is_some() {
                Stage::Ready
            } else {
                Stage::NoChange
            };
            line.change = change;
            line.fixing_since = None;
        }
    });
}

/// `'…'` for a POSIX shell.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Whether a fix `자동으로 준비해 두기` started still waits on the user.
fn auto_waiting(project: &Project) -> bool {
    project.lines.iter().any(|line| {
        line.auto && matches!(line.stage, Stage::Fixing | Stage::Ready | Stage::NoChange)
    })
}

impl Daemon {
    /// Naru AI's share of the one-second tick.
    pub fn ai_tick(self: &Arc<Self>) {
        let ticks = self.ai.ticks.fetch_add(1, Ordering::SeqCst) + 1;
        if ticks.is_multiple_of(LOOK_TICKS) {
            self.record_agent_waits();
            self.check_fixing();
            self.auto_prepare();
        }
        if ticks.is_multiple_of(CAPTURE_TICKS) {
            self.note_codex_sessions();
            self.in_background(|daemon| daemon.start_capture(false));
        }
        if ticks.is_multiple_of(DUE_TICKS) {
            self.in_background(Daemon::start_due_investigation);
            self.in_background(Daemon::start_learn_map);
        }
    }

    /// Model calls take minutes; they must not hold up the tick or a press's answer.
    fn in_background(self: &Arc<Self>, job: impl FnOnce(&Daemon) + Send + 'static) {
        let daemon = self.clone();
        std::thread::spawn(move || job(&daemon));
    }

    /// Agent panes on this machine.
    fn local_sessions(&self) -> Vec<LocalSession> {
        let found: Vec<(LocalSession, Option<String>)> = {
            let core = self.core();
            core.workspace
                .panes
                .values()
                .filter(|record| record.node.is_none())
                .filter_map(|record| {
                    let session = record.session.as_ref()?;
                    let local = LocalSession {
                        id: session.session_id.clone()?,
                        agent: session.agent,
                        status: session.status.clone(),
                        tool: session.tool_name.is_some(),
                        root: None,
                    };
                    Some((local, record.cwd.clone()))
                })
                .collect()
        };
        // Outside the lock: finding the repository stats every ancestor folder.
        found
            .into_iter()
            .map(|(session, cwd)| LocalSession {
                root: repo_of(cwd.as_deref()),
                ..session
            })
            .collect()
    }

    /// Notes, once per session, a Claude agent that stopped to ask or to wait on a permission.
    fn record_agent_waits(&self) {
        for LocalSession {
            id,
            agent,
            status,
            tool,
            root,
        } in self.local_sessions()
        {
            let (Agent::Claude, Status::Blocked { .. }, Some(root)) = (agent, status, root) else {
                continue;
            };
            let tag = if tool { "blocked" } else { "question" };
            if self
                .ai
                .waits_seen
                .lock()
                .expect("waits lock")
                .insert((id.clone(), tag))
            {
                suggestions::record_wait(&root, &id, tag);
            }
        }
    }

    /// Finishes the fixes whose agent stopped after working, or whose pane closed.
    fn check_fixing(&self) {
        // Each pane's agent status, and whether it has had a turn: a fix that ran between two
        // looks never shows as working.
        let statuses: HashMap<PaneId, Option<(Status, bool)>> = {
            let core = self.core();
            core.workspace
                .panes
                .iter()
                .map(|(pane, record)| {
                    let session = record.session.as_ref();
                    (
                        *pane,
                        session.map(|s| (s.status.clone(), s.query.is_some())),
                    )
                })
                .collect()
        };
        let done: Vec<(PathBuf, u32)> = {
            let mut fixing = self.ai.fixing.lock().expect("fixing lock");
            let mut done = Vec::new();
            for (key, (pane, worked)) in fixing.iter_mut() {
                match statuses.get(pane) {
                    None => done.push(key.clone()),
                    Some(Some((Status::InProgress, _))) => *worked = true,
                    Some(Some((
                        Status::Success | Status::Failed { .. } | Status::Cancelled,
                        turned,
                    ))) if *worked || *turned => done.push(key.clone()),
                    Some(_) => {}
                }
            }
            for key in &done {
                fixing.remove(key);
            }
            done
        };
        for (root, id) in done {
            finish_fix(&root, id);
        }
    }

    /// `자동으로 준비해 두기`: starts the oldest offered document fix once nothing it started
    /// waits on the user.
    fn auto_prepare(self: &Arc<Self>) {
        let Some(root) = self.ai.front.lock().expect("front lock").clone() else {
            return;
        };
        let project = suggestions::load(&root);
        let settings = &project.settings;
        if !(settings.enabled && settings.suggestions && settings.auto_prepare)
            || auto_waiting(&project)
        {
            return;
        }
        let next = project
            .lines
            .iter()
            .filter(|line| {
                line.is_choosable()
                    && line.stage == Stage::New
                    && line.refusal.is_none()
                    && matches!(line.fix, Some(Fix::Doc { .. }))
            })
            .min_by_key(|line| line.raised_at.unwrap_or(0))
            .map(|line| line.id);
        if let Some(id) = next {
            self.start_fix(&root, id, true);
        }
    }

    /// Notes the rollout writes of open Codex sessions, so Memory reads them once they go quiet.
    fn note_codex_sessions(&self) {
        for session in self.local_sessions() {
            if let (Agent::Codex, Some(root)) = (session.agent, &session.root) {
                memory::note_codex(root, &session.id);
            }
        }
    }

    /// Reads the sessions that wait for Memory: quiet ones, one per call, or with `now` the front
    /// repository's newest few whatever their quiet.
    fn start_capture(&self, now: bool) {
        let Some(claude) = claude_path(&self.home) else {
            return;
        };
        if self.ai.capturing.swap(true, Ordering::SeqCst) {
            return;
        }
        let live: HashSet<String> = if now {
            HashSet::new()
        } else {
            self.local_sessions()
                .into_iter()
                .map(|session| session.id)
                .collect()
        };
        let front = self.ai.front.lock().expect("front lock").clone();
        let roots = match (front, now) {
            (Some(root), true) => vec![root],
            _ => suggestions::known_roots(),
        };
        let limit = if now { CHECK_NOW_SESSIONS } else { 1 };
        let mut read = 0;
        'roots: for root in roots {
            let mut due = memory::due_sessions(&root, &live);
            if now {
                due.reverse();
            }
            for session in due {
                if memory::capture(&root, &session, &claude).is_some_and(|added| added > 0) {
                    self.learn_changed(None);
                }
                read += 1;
                if read >= limit {
                    break 'roots;
                }
            }
        }
        self.ai.capturing.store(false, Ordering::SeqCst);
    }

    fn start_due_investigation(&self) {
        let Some(claude) = claude_path(&self.home) else {
            self.ai.claude_missing.store(true, Ordering::SeqCst);
            return;
        };
        self.ai.claude_missing.store(false, Ordering::SeqCst);
        let Some(root) = suggestions::known_roots()
            .into_iter()
            .find(|root| suggestions::is_due(&suggestions::load(root)))
        else {
            return;
        };
        {
            let mut running = self.ai.investigating.lock().expect("investigating lock");
            if running.is_some() {
                return;
            }
            *running = Some(root.clone());
        }
        suggestions::investigate(&root, &claude);
        *self.ai.investigating.lock().expect("investigating lock") = None;
    }

    /// What the panel shows for `pane`'s repository; `None` when the pane is not in one.
    pub fn ai_view(&self, pane: PaneId) -> anyhow::Result<Option<View>> {
        let cwd = {
            let core = self.core();
            let record = core
                .workspace
                .panes
                .get(&pane)
                .ok_or_else(|| anyhow::anyhow!("no pane {pane}"))?;
            // A node pane's folder is on another machine.
            record.cwd.clone().filter(|_| record.node.is_none())
        };
        let root = repo_of(cwd.as_deref());
        *self.ai.front.lock().expect("front lock") = root.clone();
        let Some(root) = root else {
            return Ok(None);
        };
        let fixing: BTreeMap<u32, PaneId> = self
            .ai
            .fixing
            .lock()
            .expect("fixing lock")
            .iter()
            .filter(|((at, _), _)| *at == root)
            .map(|((_, id), (pane, _))| (*id, *pane))
            .collect();
        // A fix a restart left running has no agent to finish it any more.
        let mut project = suggestions::load(&root);
        let orphans: Vec<u32> = project
            .lines
            .iter()
            .filter(|line| line.stage == Stage::Fixing && !fixing.contains_key(&line.id))
            .map(|line| line.id)
            .collect();
        if !orphans.is_empty() {
            for id in orphans {
                finish_fix(&root, id);
            }
            project = suggestions::load(&root);
        }
        let cached = self
            .ai
            .github
            .lock()
            .expect("github lock")
            .get(&root)
            .copied();
        let github = cached.unwrap_or_else(|| {
            let ready = suggestions::github_ready(&root);
            self.ai
                .github
                .lock()
                .expect("github lock")
                .insert(root.clone(), ready);
            ready
        });
        let memory = memory::load(&root);
        let receipts = memory::receipts(&root);
        let mut newest: Vec<_> = receipts.iter().collect();
        newest.sort_by_key(|receipt| receipt.at);
        let received = newest
            .into_iter()
            .map(|receipt| {
                let how = receipt.how.clone();
                (
                    receipt.card,
                    Received {
                        how,
                        at: receipt.at,
                    },
                )
            })
            .collect();
        let used = memory::last_used(&memory, &receipts);
        let held = memory::holds(&root, &memory);
        let memory_on = project.settings.enabled && project.settings.memory;
        let badge = project.badge() + if memory_on { memory.conflicts.len() } else { 0 };
        Ok(Some(View {
            global: global_view(&project),
            choosable: project
                .lines
                .iter()
                .filter(|line| line.is_choosable())
                .map(|line| line.id)
                .collect(),
            auto_waiting: auto_waiting(&project),
            docs: memory::docs(&root),
            investigating: self
                .ai
                .investigating
                .lock()
                .expect("investigating lock")
                .as_ref()
                == Some(&root),
            capturing: self.ai.capturing.load(Ordering::SeqCst),
            claude_missing: self.ai.claude_missing.load(Ordering::SeqCst),
            root,
            project,
            memory,
            held,
            received,
            used,
            github,
            fixing,
            badge,
        }))
    }

    /// `naru memory stale <m#> <reason>` from `pane`: holds the card for the session running there.
    pub fn op_memory_stale(&self, pane: PaneId, card: u32, reason: &str) -> Result<Value, String> {
        let (cwd, session) = {
            let core = self.core();
            let record = core.workspace.panes.get(&pane).ok_or("this pane is gone")?;
            let session = record.session.as_ref();
            (
                session
                    .and_then(|session| session.cwd.clone())
                    .or_else(|| record.cwd.clone()),
                session.and_then(|session| session.session_id.clone()),
            )
        };
        let session = session.ok_or("Naru does not know this pane's agent session yet")?;
        let root = repo_of(cwd.as_deref()).ok_or("this pane is not in a git repository")?;
        memory::hold(&root, card, &session, reason)?;
        Ok(json!({ "card": format!("m{card}") }))
    }

    /// Applies a press for `pane`'s repository.
    pub fn ai_act(self: &Arc<Self>, pane: PaneId, act: Act) -> anyhow::Result<()> {
        let cwd = {
            let core = self.core();
            core.workspace
                .panes
                .get(&pane)
                .ok_or_else(|| anyhow::anyhow!("no pane {pane}"))?
                .cwd
                .clone()
        };
        let root = repo_of(cwd.as_deref())
            .ok_or_else(|| anyhow::anyhow!("이 pane은 git 저장소에 있지 않아요"))?;
        let line = |id: u32| suggestions::load(&root).line(id).cloned();
        match act {
            Act::Fix { line } => self.start_fix(&root, line, false),
            Act::Issue { line: id } => {
                let root = root.clone();
                self.in_background(move |daemon| daemon.start_issue(&root, id));
            }
            Act::Skip { line } => set_line(&root, line, |line| {
                line.refusal = None;
                suggestions::skip(line);
            }),
            Act::Keep { line } => set_line(&root, line, |line| {
                if line.stage == Stage::Ready {
                    mark_applied(line);
                }
            }),
            Act::Revert { line: id } => {
                let Some(found) = line(id) else { return Ok(()) };
                let (Some(Fix::Doc { path, .. }), Some(change)) = (&found.fix, &found.change)
                else {
                    return Ok(());
                };
                match suggestions::revert_change(&root, path, change) {
                    Ok(()) => set_line(&root, id, |line| {
                        line.stage = Stage::New;
                        line.change = None;
                        line.refusal = None;
                    }),
                    Err(why) => refuse(&root, id, why),
                }
            }
            Act::Restore { line } => {
                suggestions::update(&root, |project| suggestions::restore(project, line));
            }
            Act::ShowDiff { line: id } => self.show_diff(&root, id, pane)?,
            Act::ApplyGlobal { line: id, seen } => {
                let Some(found) = line(id) else { return Ok(()) };
                let Some(Fix::Global { line: words }) = &found.fix else {
                    return Ok(());
                };
                let seen: u64 = seen.parse()?;
                match global_instruction::apply(words, seen) {
                    Ok(()) => set_line(&root, id, mark_applied),
                    Err(why) => refuse(&root, id, &why),
                }
            }
            Act::RevertGlobal { line: id } => {
                let Some(found) = line(id) else { return Ok(()) };
                let Some(Fix::Global { line: words }) = &found.fix else {
                    return Ok(());
                };
                match global_instruction::revert(words) {
                    Ok(()) => set_line(&root, id, |line| {
                        line.stage = Stage::New;
                        line.applied_at = None;
                        line.refusal = None;
                    }),
                    Err(why) => refuse(&root, id, &why),
                }
            }
            Act::Settings { settings } => {
                suggestions::update(&root, |project| {
                    project.settings = settings;
                    suggestions::raise(project);
                });
            }
            Act::Archive { card } => {
                memory::update(&root, |memory| memory.archive(card, "내가 보관"));
            }
            Act::Revive { card } => {
                memory::update(&root, |memory| memory.revive(card));
            }
            Act::Release { card } => memory::release(&root, card),
            Act::Undo { change } => {
                memory::update(&root, |memory| memory.undo(change));
            }
            Act::Promote { card: id } => {
                let Some(card) = memory::load(&root).card(id).cloned() else {
                    return Ok(());
                };
                let mut opened = Vec::new();
                for doc in memory::docs(&root) {
                    if card.promoted.contains(&doc) {
                        continue;
                    }
                    let fix = Fix::Doc {
                        title: card.title.clone(),
                        path: doc.clone(),
                        draft: card.body.clone(),
                    };
                    if suggestions::add_line(&root, format!("기억 m{id}을 {doc}에 반영해요"), fix)
                        .is_some()
                    {
                        opened.push(doc);
                    }
                }
                memory::update(&root, |memory| {
                    if let Some(card) = memory.cards.iter_mut().find(|card| card.id == id) {
                        card.promoted.extend(opened);
                    }
                });
            }
            Act::Settle { index, card_right } => {
                let found = memory::load(&root);
                let Some((conflict, card)) = found.conflicts.get(index).and_then(|conflict| {
                    Some((conflict.clone(), found.card(conflict.card)?.clone()))
                }) else {
                    return Ok(());
                };
                if card_right {
                    let fix = Fix::Doc {
                        title: format!("{}를 기억에 맞추기", conflict.doc),
                        path: conflict.doc.clone(),
                        draft: card.body.clone(),
                    };
                    suggestions::add_line(
                        &root,
                        format!("문서와 기억이 달라요: {}", card.title),
                        fix,
                    );
                }
                memory::update(&root, |memory| {
                    memory.conflicts.retain(|known| {
                        !(known.card == conflict.card && known.doc == conflict.doc)
                    });
                    if !card_right {
                        memory.archive(card.id, "문서가 맞음");
                    }
                });
            }
            Act::CheckNow => self.in_background(|daemon| daemon.start_capture(true)),
            Act::Forget => memory::forget(&root),
        }
        Ok(())
    }

    /// Opens a card behind the current one where claude makes line `id`'s document fix.
    fn start_fix(self: &Arc<Self>, root: &Path, id: u32, auto: bool) {
        let Some(line) = suggestions::load(root).line(id).cloned() else {
            return;
        };
        let Some(Fix::Doc { path, .. }) = &line.fix else {
            return;
        };
        if suggestions::file_is_dirty(root, path) {
            return refuse(root, id, "저장 안 한 변경이 있어요");
        }
        let Some(task) = suggestions::write_task(root, &line) else {
            return refuse(root, id, "시작하지 못했어요");
        };
        let command = format!(
            "claude {}",
            shell_quote(&format!("Read {} and do what it says.", task.display()))
        );
        let pane = match self.background_tab(
            root.to_string_lossy().into_owned(),
            command,
            format!("제안 반영하기 · s{id}"),
        ) {
            Ok(pane) => pane,
            Err(error) => {
                eprintln!("suggestion fix s{id}: {error:#}");
                return refuse(root, id, "시작하지 못했어요");
            }
        };
        self.ai
            .fixing
            .lock()
            .expect("fixing lock")
            .insert((root.to_path_buf(), id), (pane, false));
        set_line(root, id, |line| {
            line.stage = Stage::Fixing;
            line.fixing_since = Some(suggestions::now());
            line.change = None;
            line.refusal = None;
            line.auto = auto;
        });
    }

    fn start_issue(&self, root: &Path, id: u32) {
        let Some(Some(Fix::Issue { title, body })) = suggestions::load(root)
            .line(id)
            .map(|line| line.fix.clone())
        else {
            return;
        };
        if !suggestions::github_ready(root) {
            self.ai
                .github
                .lock()
                .expect("github lock")
                .insert(root.to_path_buf(), false);
            return refuse(root, id, "GitHub 연결 필요");
        }
        let key = (root.to_path_buf(), id);
        if !self
            .ai
            .issuing
            .lock()
            .expect("issuing lock")
            .insert(key.clone())
        {
            return;
        }
        refuse(root, id, "이슈 만드는 중…");
        let result = suggestions::open_issue(root, &title, &body);
        set_line(root, id, |line| match result {
            Ok(url) => {
                line.stage = Stage::Issue;
                line.issue = Some(url);
                line.refusal = None;
            }
            Err(why) => line.refusal = Some(why),
        });
        self.ai.issuing.lock().expect("issuing lock").remove(&key);
    }

    /// Shows what a fix changed in the document lane.
    fn show_diff(&self, root: &Path, id: u32, from: PaneId) -> anyhow::Result<()> {
        let Some(line) = suggestions::load(root).line(id).cloned() else {
            return Ok(());
        };
        let (Some(Fix::Doc { path, .. }), Some(change)) = (&line.fix, &line.change) else {
            return Ok(());
        };
        let file = std::env::temp_dir().join("naru-views").join(format!(
            "suggestion-{}-s{id}.md",
            suggestions::project_key(root)
        ));
        let text = format!(
            "# {path} · 바뀐 것\n\n+{} −{}{}\n\n```diff\n{}\n```\n",
            change.added,
            change.removed,
            if change.created { " · 새 파일" } else { "" },
            change.diff.trim_end()
        );
        suggestions::write_private(&file, text.as_bytes())?;
        let mut core = self.core();
        // The user asked for it, so it does not wait in 내 차례 to be looked at.
        core.workspace
            .add_view(file.to_string_lossy().into_owned(), false, Some(from))
            .seen = true;
        self.changed(&core);
        Ok(())
    }
}
