//! The daemon's single source of truth: the workspace, the live panes behind it, and what each
//! pane's shell and agent report. Every change is saved at once and pushed to every screen.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use portable_pty::PtySize;
use tokio::sync::broadcast;

use crate::agent::Agent;
use crate::naru;
use crate::osc::Osc;
use crate::pane::{self, Pane, PaneEvent};
use crate::session::{Event, EventKind, Restore, Session, Status};
use crate::state::{Direction, Id, PaneId, Workspace};
use crate::team::{PaneTeam, Team};
use crate::ws::ServerMsg;

pub struct Daemon {
    pub token: String,
    pub home: PathBuf,
    pub state_file: PathBuf,
    pub zsh_dir: PathBuf,
    pub claude_projects: PathBuf,
    /// `~/.naru`, which the naru helper reads its token from and its replies in.
    pub naru_dir: PathBuf,
    pub naru_token: String,
    core: Mutex<Core>,
    /// Serialized messages every attached screen receives.
    pub events: broadcast::Sender<Arc<str>>,
    /// Set once the daemon is shutting down: panes ending now are not the user closing them.
    terminating: AtomicBool,
    /// When a screen last sent input: the user is at the keyboard.
    last_input: Mutex<Option<Instant>>,
    /// The team state last sent, so a tick sends only what changed.
    sent_team: Mutex<BTreeMap<PaneId, PaneTeam>>,
    pub titles: crate::titles::Titles,
    /// Claude transcripts for ⌘K conversation search.
    pub conversations: crate::conversations::Index,
    /// Learning sessions this run opened, by 학습함 mark.
    pub studies: Mutex<BTreeMap<u32, PaneId>>,
    pub ai: crate::ai::Ai,
    pub accounts: crate::accounts::Accounts,
}

#[derive(Default)]
pub struct Core {
    pub workspace: Workspace,
    pub live: BTreeMap<PaneId, Arc<Pane>>,
    /// What to put into a pane once its shell first reads a command line.
    pub pending: BTreeMap<PaneId, Restore>,
    /// When a restored agent may start; a restart resumes a few at a time.
    pub resume_at: BTreeMap<PaneId, Instant>,
    /// Restored panes whose shell is at its prompt, waiting for their `resume_at`.
    pub held: BTreeSet<PaneId>,
    pub team: Team,
}

/// Agents a restart resumes together, and the wait before the next few. Some thirty agents
/// starting at once pinned the CPU for the first minute.
const RESUME_BATCH: usize = 3;
const RESUME_GAP: Duration = Duration::from_secs(3);

const START_SIZE: PtySize = PtySize {
    rows: 24,
    cols: 80,
    pixel_width: 0,
    pixel_height: 0,
};

impl Daemon {
    pub fn new(
        token: String,
        home: PathBuf,
        state_file: PathBuf,
        zsh_dir: PathBuf,
        workspace: Workspace,
        naru_token: String,
    ) -> Arc<Daemon> {
        Arc::new(Daemon {
            token,
            claude_projects: home.join(".claude/projects"),
            naru_dir: home.join(".naru"),
            home,
            state_file,
            zsh_dir,
            naru_token,
            core: Mutex::new(Core {
                workspace,
                ..Core::default()
            }),
            events: broadcast::channel(1024).0,
            terminating: AtomicBool::new(false),
            last_input: Mutex::new(None),
            sent_team: Mutex::new(BTreeMap::new()),
            conversations: Default::default(),
            titles: Default::default(),
            studies: Mutex::new(BTreeMap::new()),
            ai: Default::default(),
            accounts: Default::default(),
        })
    }

    pub fn core(&self) -> MutexGuard<'_, Core> {
        self.core.lock().expect("core lock")
    }

    pub fn last_input(&self) -> Option<Instant> {
        *self.last_input.lock().expect("input lock")
    }

    fn state_text(&self, core: &Core) -> Arc<str> {
        let team = core.team.view(&core.workspace);
        let text = ServerMsg::State {
            workspace: &core.workspace,
            team: &team,
        }
        .text();
        *self.sent_team.lock().expect("team lock") = team;
        text
    }

    pub fn state_message(&self) -> Arc<str> {
        self.state_text(&self.core())
    }

    /// Saves, then tells every screen; call after any change to the workspace. Saved before the
    /// screens hear of it, so a crash never loses something a screen already showed.
    pub fn changed(&self, core: &Core) {
        if self.terminating.load(Ordering::SeqCst) {
            return;
        }
        if let Err(error) = core.workspace.save(&self.state_file) {
            eprintln!("cannot save {}: {error:#}", self.state_file.display());
        }
        let _ = self.events.send(self.state_text(core));
    }

    /// Tells every screen when only runtime team state changed, which a restart does not keep.
    pub fn send_if_team_changed(&self, core: &Core) {
        if core.team.view(&core.workspace) != *self.sent_team.lock().expect("team lock") {
            let _ = self.events.send(self.state_text(core));
        }
    }

    pub fn terminate(&self) {
        self.terminating.store(true, Ordering::SeqCst);
    }

    pub fn pane(&self, id: PaneId) -> anyhow::Result<Arc<Pane>> {
        self.core()
            .live
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow!("no pane {id}"))
    }

    /// Saved with the next change; a resize alone does not write the workspace.
    pub fn resize(&self, id: PaneId, cols: u16, rows: u16) -> anyhow::Result<()> {
        let mut core = self.core();
        let pane = core
            .live
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow!("no pane {id}"))?;
        if let Some(record) = core.workspace.panes.get_mut(&id) {
            record.size = Some((cols, rows));
        }
        drop(core);
        pane.resize(cols, rows)
    }

    /// Input from a screen.
    pub fn input(&self, id: PaneId, data: String) -> anyhow::Result<()> {
        *self.last_input.lock().expect("input lock") = Some(Instant::now());
        self.pane(id)?.write(data.into_bytes())?;
        let mut core = self.core();
        if mark_read(&mut core, id) {
            self.changed(&core);
        }
        Ok(())
    }

    /// Starts a shell for every saved pane and queues each agent's resume. Panes whose shell
    /// cannot start are dropped.
    pub fn restore(self: &Arc<Self>) {
        let mut core = self.core();
        core.workspace.repair();
        let ids: Vec<PaneId> = core.workspace.panes.keys().copied().collect();
        for id in ids {
            let record = core.workspace.panes.get_mut(&id).expect("listed pane");
            record.running = None;
            let restore = if record.node.is_some() {
                // The agent runs on the node; the pane's shell connects again and resumes it there.
                record.session = None;
                Some(Restore::Run(crate::nodes::ATTACH.to_owned()))
            } else {
                let restore = record.session.as_ref().and_then(Session::restore);
                match record.session.as_mut() {
                    Some(session) if restore.is_some() => {
                        // The agent comes back waiting at its prompt; only a turn the user had
                        // not handled still reads as done.
                        session.seen = !session.shows_done();
                        session.status = Status::Success;
                    }
                    // Nothing will run the agent again, so the pane is a plain shell.
                    _ => record.session = None,
                }
                restore
            };
            if let Some(restore) = restore {
                core.pending.insert(id, restore);
            }
            if let Err(error) = self.spawn_or_remove(&mut core, id) {
                eprintln!("cannot restore pane {id}: {error:#}");
                core.pending.remove(&id);
            }
        }
        let start = Instant::now();
        let runs: Vec<PaneId> = core
            .workspace
            .resume_order()
            .into_iter()
            .filter(|id| {
                matches!(core.pending.get(id), Some(Restore::Run(_)))
                    && core
                        .workspace
                        .panes
                        .get(id)
                        .is_some_and(|r| r.node.is_none())
            })
            .collect();
        for (index, id) in runs.into_iter().enumerate() {
            let turn = u32::try_from(index / RESUME_BATCH).unwrap_or(u32::MAX);
            core.resume_at.insert(id, start + RESUME_GAP * turn);
        }
        if !core.resume_at.is_empty() {
            self.release_resumes();
        }
        if core.workspace.tabs.is_empty() {
            let (_, pane) = core.workspace.add_tab(None, None);
            if let Err(error) = self.spawn(&mut core, pane) {
                eprintln!("cannot open a shell: {error:#}");
            }
        }
        self.restore_team(&mut core);
        self.changed(&core);
    }

    /// Resumes held panes as their turn comes, and the one the user focuses at once.
    fn release_resumes(self: &Arc<Self>) {
        let daemon = Arc::downgrade(self);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let Some(daemon) = daemon.upgrade() else {
                    return;
                };
                let mut guard = daemon.core();
                let core = &mut *guard;
                let now = Instant::now();
                let focused = core.workspace.focused();
                // A shell not at its prompt yet when its turn comes runs the resume as it gets there.
                core.resume_at.retain(|id, at| {
                    core.pending.contains_key(id) && (*at > now || core.held.contains(id))
                });
                let due: Vec<PaneId> = core
                    .held
                    .iter()
                    .copied()
                    .filter(|id| {
                        focused == Some(*id) || core.resume_at.get(id).is_none_or(|at| *at <= now)
                    })
                    .collect();
                for id in due {
                    core.held.remove(&id);
                    core.resume_at.remove(&id);
                    if let (Some(restore), Some(pane)) =
                        (core.pending.remove(&id), core.live.get(&id))
                    {
                        put_restore(id, pane, restore);
                    }
                }
                if core.resume_at.is_empty() {
                    core.held.clear();
                    return;
                }
            }
        });
    }

    fn spawn(self: &Arc<Self>, core: &mut Core, id: PaneId) -> anyhow::Result<()> {
        let cwd = core
            .workspace
            .panes
            .get(&id)
            .and_then(|record| record.cwd.clone())
            .map(PathBuf::from)
            .filter(|dir| dir.is_dir())
            .unwrap_or_else(|| self.home.clone());
        let size = core
            .workspace
            .panes
            .get(&id)
            .and_then(|record| record.size)
            .map_or(START_SIZE, |(cols, rows)| PtySize {
                cols,
                rows,
                ..START_SIZE
            });
        let daemon = Arc::downgrade(self);
        let pane = Pane::spawn(
            pane::shell_command(&cwd, &self.zsh_dir),
            size,
            move |event| on_pane_event(&daemon, id, event),
        )?;
        core.live.insert(id, pane);
        Ok(())
    }

    /// Starts the shell of a pane just added to the workspace, or takes the pane out again.
    pub fn spawn_or_remove(self: &Arc<Self>, core: &mut Core, id: PaneId) -> anyhow::Result<()> {
        let started = self.spawn(core, id);
        if started.is_err() {
            core.workspace.remove_pane(id);
        }
        started
    }

    /// A new card, made active, that runs `run` once its shell is ready.
    pub fn new_tab(
        self: &Arc<Self>,
        group: Option<Id>,
        cwd: Option<String>,
        run: Option<String>,
    ) -> anyhow::Result<PaneId> {
        let mut core = self.core();
        let group = group.or_else(|| cwd.as_deref().and_then(|cwd| core.workspace.group_for(cwd)));
        let (_, pane) = core.workspace.add_tab(group, cwd);
        self.spawn_or_remove(&mut core, pane)?;
        if let Some(command) = run {
            core.pending.insert(pane, Restore::Run(command));
        }
        self.changed(&core);
        Ok(pane)
    }

    /// A card titled `title` at the end of `cwd`'s group that runs `run`, leaving the active card
    /// as it was.
    pub fn background_tab(
        self: &Arc<Self>,
        cwd: String,
        run: String,
        title: String,
    ) -> anyhow::Result<PaneId> {
        let mut core = self.core();
        let group = core.workspace.group_for(&cwd);
        let at = core.workspace.end_of_group(group);
        let (tab, pane) = core.workspace.insert_tab(at, group, Some(cwd));
        if let Some(tab) = core.workspace.tabs.iter_mut().find(|t| t.id == tab) {
            tab.title = Some(title);
        }
        self.spawn_or_remove(&mut core, pane)?;
        core.pending.insert(pane, Restore::Run(run));
        self.changed(&core);
        Ok(pane)
    }

    pub fn split(self: &Arc<Self>, at: PaneId, direction: Direction) -> anyhow::Result<PaneId> {
        let mut core = self.core();
        let pane = core.workspace.split(at, direction, true)?;
        self.spawn_or_remove(&mut core, pane)?;
        self.changed(&core);
        Ok(pane)
    }

    pub fn add_group(&self, name: String, path: Option<String>) {
        let mut core = self.core();
        core.workspace.add_group(name, path);
        self.changed(&core);
    }

    pub fn rename_group(&self, group: Id, name: &str) -> anyhow::Result<()> {
        let mut core = self.core();
        core.workspace.rename_group(group, name)?;
        self.changed(&core);
        Ok(())
    }

    /// Focusing a pane is looking at it: its finished turn or question no longer reads as new.
    pub fn focus(&self, pane: PaneId) -> anyhow::Result<()> {
        let mut core = self.core();
        core.workspace.focus(pane)?;
        mark_read(&mut core, pane);
        self.changed(&core);
        Ok(())
    }

    pub fn close_pane(&self, pane: PaneId) -> anyhow::Result<()> {
        self.pane(pane)?.kill();
        Ok(())
    }

    /// "Back to waiting": the finished turn no longer reads as done.
    pub fn mark_seen(&self, pane: PaneId) -> anyhow::Result<()> {
        self.update_session(pane, |session| {
            session.seen = true;
            session.read = true;
        })
    }

    pub fn set_title(&self, pane: PaneId, title: Option<String>) -> anyhow::Result<()> {
        let mut core = self.core();
        let Some(record) = core.workspace.panes.get_mut(&pane) else {
            bail!("no pane {pane}");
        };
        record.title = title
            .map(|title| title.trim().to_owned())
            .filter(|t| !t.is_empty());
        self.changed(&core);
        Ok(())
    }

    pub fn view_seen(&self, view: Id) -> anyhow::Result<()> {
        let mut core = self.core();
        let Some(found) = core.workspace.views.iter_mut().find(|v| v.id == view) else {
            bail!("no view {view}");
        };
        found.seen = true;
        self.changed(&core);
        Ok(())
    }

    pub fn view_path(&self, view: Id) -> anyhow::Result<std::path::PathBuf> {
        let core = self.core();
        let Some(found) = core.workspace.views.iter().find(|v| v.id == view) else {
            bail!("no view {view}");
        };
        Ok(found.path.clone().into())
    }

    pub fn close_view(&self, view: Id) {
        let mut core = self.core();
        core.workspace.views.retain(|v| v.id != view);
        self.changed(&core);
    }

    fn update_session(
        &self,
        pane: PaneId,
        update: impl FnOnce(&mut Session),
    ) -> anyhow::Result<()> {
        let mut core = self.core();
        let Some(session) = core
            .workspace
            .panes
            .get_mut(&pane)
            .and_then(|record| record.session.as_mut())
        else {
            bail!("no agent in pane {pane}");
        };
        update(session);
        self.changed(&core);
        Ok(())
    }

    /// `naru open`: a file shown in the document lane, or an HTML page in the user's browser.
    /// Shows a file named in a pane's output: in the document lane, or the browser for HTML.
    /// Each live local pane's shell pid.
    pub fn shells(&self) -> Vec<(PaneId, u32)> {
        let core = self.core();
        core.live
            .iter()
            .filter(|(pane, _)| {
                core.workspace
                    .panes
                    .get(pane)
                    .is_none_or(|record| record.node.is_none())
            })
            .filter_map(|(pane, live)| Some((*pane, live.pid()?)))
            .collect()
    }

    pub fn open_terminal_path(&self, pane: PaneId, text: &str) {
        let cwd = self
            .core()
            .workspace
            .panes
            .get(&pane)
            .and_then(|record| record.cwd.clone());
        match terminal_path(text, cwd.as_deref().map(Path::new), &self.home) {
            Some(path) => self.show_page(&path, Some(pane)),
            None => {
                let text = format!("파일을 찾지 못했어요: {text}");
                let _ = self.events.send(ServerMsg::Notice { text }.text());
            }
        }
    }

    /// Background subagents of the Claude transcript at `path` still running. Only a transcript
    /// under `~/.claude/projects` is read: the path comes from whatever printed the event.
    fn pending_agents(&self, path: &Path) -> usize {
        const TAIL: u64 = 2 * 1024 * 1024;
        if !path.starts_with(&self.claude_projects)
            || path.extension().is_none_or(|ext| ext != "jsonl")
            || path
                .components()
                .any(|part| part == std::path::Component::ParentDir)
        {
            return 0;
        }
        crate::transcript::read_tail(path, TAIL).map_or(0, |(tail, _)| {
            crate::transcript::claude_pending_agents(&tail)
        })
    }

    fn view_open(&self, from: PaneId, body: &str) {
        let Some((path, proof)) = naru::parse_view(body) else {
            return;
        };
        if !proof {
            return self.show_page(&path, Some(from));
        }
        let proof = proof && naru::has_findings(&path);
        let mut core = self.core();
        core.workspace
            .add_view(path.to_string_lossy().into_owned(), proof, Some(from));
        self.changed(&core);
    }

    fn on_osc(self: &Arc<Self>, id: PaneId, osc: Osc) {
        if let Osc::Notify { title, body } = &osc {
            match title.as_str() {
                naru::AGENT => return self.naru_request(id, body),
                naru::VIEW_OPEN => return self.view_open(id, body),
                _ => {}
            }
        }
        // Read before the lock: a pane's output must not wait on the disk.
        let pending_agents = match &osc {
            Osc::Notify { title, body } => Event::parse(title, body)
                .filter(|event| event.agent == Agent::Claude && event.kind == EventKind::Stop)
                .and_then(|event| event.transcript_path)
                .map_or(0, |path| self.pending_agents(Path::new(&path))),
            _ => 0,
        };
        let mut core = self.core();
        let Core {
            workspace,
            live,
            pending,
            resume_at,
            held,
            team,
        } = &mut *core;
        let focused = workspace.focused();
        let Some(record) = workspace.panes.get_mut(&id) else {
            return;
        };
        let changed = match osc {
            Osc::Notify { title, body } => match Event::parse(&title, &body) {
                Some(mut event) => {
                    event.pending_agents = pending_agents;
                    let created = record.session.is_none();
                    let session = record.session.get_or_insert_with(|| {
                        let detected = record
                            .running
                            .as_deref()
                            .and_then(Agent::detect)
                            .filter(|(agent, _)| *agent == event.agent);
                        let mut session = Session::new(
                            event.agent,
                            detected.as_ref().and_then(|(_, launcher)| launcher.clone()),
                            detected.and(record.running.clone()),
                        );
                        session.remote = record.node.is_some()
                            || record.running.as_deref().is_some_and(is_remote_login);
                        session
                    });
                    let applied = session.apply(&event, &self.claude_projects);
                    if event.agent == Agent::Codex
                        && event.kind == EventKind::Stop
                        && session.status == Status::Success
                    {
                        team.read_codex_turn(id);
                    }
                    if let Some(node) = record.node.as_mut() {
                        node.cwd = session.cwd.clone().or(node.cwd.take());
                        node.resume = session.resume_command();
                    }
                    if created && team.start_run(id) {
                        self.woke(id);
                    }
                    team.heard(id, event.kind == EventKind::PromptSubmit);
                    applied || created
                }
                None => {
                    let message = ServerMsg::Osc {
                        pane: id,
                        osc: &Osc::Notify { title, body },
                    };
                    let _ = self.events.send(message.text());
                    false
                }
            },
            Osc::Command { line } => {
                if let Some((agent, launcher)) = Agent::detect(&line) {
                    match &mut record.session {
                        // The restored agent's own resume line, or a guess the user accepted.
                        Some(session) if session.agent == agent => {
                            session.launcher = launcher;
                            session.launch_command = Some(line.clone());
                        }
                        _ => {
                            record.session =
                                Some(Session::new(agent, launcher, Some(line.clone())));
                            if team.start_run(id) {
                                self.woke(id);
                            }
                        }
                    }
                }
                record.running = Some(line);
                pending.remove(&id);
                true
            }
            Osc::Prompt { mark, .. } => match mark.as_str() {
                // The line editor is reading, so the restore line is not echoed twice.
                "B" => {
                    let waits = focused != Some(id)
                        && resume_at.get(&id).is_some_and(|at| *at > Instant::now());
                    if waits && pending.contains_key(&id) {
                        held.insert(id);
                    } else if let (Some(restore), Some(pane)) = (pending.remove(&id), live.get(&id))
                    {
                        resume_at.remove(&id);
                        put_restore(id, pane, restore);
                    }
                    false
                }
                // The command ended; an agent it ran has exited with it.
                "D" => {
                    record.running = None;
                    record.session = None;
                    true
                }
                _ => false,
            },
            Osc::Cwd { path } => {
                let changed = record.cwd.as_deref() != Some(&path);
                record.cwd = Some(path);
                changed
            }
            Osc::Title { title } => {
                let changed = record.program_title.as_deref() != Some(&title);
                record.program_title = Some(title);
                changed
            }
        };
        self.deliver_letters(&mut core);
        if changed {
            self.changed(&core);
        } else {
            self.send_if_team_changed(&core);
        }
    }

    fn on_exit(&self, id: PaneId) {
        if self.terminating.load(Ordering::SeqCst) {
            return;
        }
        let mut core = self.core();
        core.live.remove(&id);
        core.pending.remove(&id);
        core.resume_at.remove(&id);
        core.held.remove(&id);
        core.team.forget(id);
        core.workspace.remove_pane(id);
        self.deliver_letters(&mut core);
        self.changed(&core);
    }
}

/// The file a path printed in a terminal names, without a trailing `:line:col`; `None` when it
/// is not an existing file.
pub fn terminal_path(text: &str, cwd: Option<&Path>, home: &Path) -> Option<PathBuf> {
    let mut text = text.trim();
    for _ in 0..2 {
        if let Some((head, tail)) = text.rsplit_once(':')
            && !tail.is_empty()
            && tail.bytes().all(|b| b.is_ascii_digit())
        {
            text = head;
        }
    }
    let path = if let Some(rest) = text.strip_prefix("~/") {
        home.join(rest)
    } else if text.starts_with('/') {
        PathBuf::from(text)
    } else {
        cwd?.join(text)
    };
    path.is_file().then_some(path)
}

fn put_restore(id: PaneId, pane: &Pane, restore: Restore) {
    let input = match restore {
        Restore::Run(command) => format!("{command}\r"),
        Restore::Type(command) => command,
    };
    if let Err(error) = pane.write(input.into_bytes()) {
        eprintln!("cannot restore pane {id}: {error:#}");
    }
}

fn is_remote_login(command: &str) -> bool {
    let first = command.split_whitespace().next().unwrap_or_default();
    matches!(
        first.rsplit('/').next(),
        Some("ssh" | "mosh" | "et" | "autossh")
    )
}

fn on_pane_event(daemon: &Weak<Daemon>, id: PaneId, event: PaneEvent) {
    let Some(daemon) = daemon.upgrade() else {
        return;
    };
    match event {
        PaneEvent::Osc(osc) => daemon.on_osc(id, osc),
        PaneEvent::Exited => daemon.on_exit(id),
    }
}

/// Whether the pane's session was unread until now.
fn mark_read(core: &mut Core, pane: PaneId) -> bool {
    let session = core
        .workspace
        .panes
        .get_mut(&pane)
        .and_then(|record| record.session.as_mut());
    match session {
        Some(session) if !session.read => {
            session.read = true;
            true
        }
        Some(_) | None => false,
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
