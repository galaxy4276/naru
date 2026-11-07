//! A pane's CLI agent session as its Warp plugin reports it over OSC 777 `warp://cli-agent`.
//! Rules ported from the Warp fork's `app/src/terminal/cli_agent_sessions/mod.rs`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent::{self, Agent};
use crate::transcript;

pub const SENTINEL: &str = "warp://cli-agent";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Status {
    InProgress,
    Success,
    Failed {
        error_type: Option<String>,
        message: Option<String>,
    },
    Blocked {
        message: Option<String>,
    },
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    SessionStart,
    PromptSubmit,
    ToolComplete,
    Stop,
    StopFailure,
    PermissionRequest,
    PermissionReplied,
    QuestionAsked,
    IdlePrompt,
    Unknown,
}

#[derive(Deserialize)]
struct Notification {
    v: Option<u32>,
    agent: Option<String>,
    event: String,
    session_id: Option<String>,
    cwd: Option<String>,
    project: Option<String>,
    turn_id: Option<String>,
    event_sequence: Option<u64>,
    session_start_source: Option<String>,
    query: Option<String>,
    response: Option<String>,
    transcript_path: Option<String>,
    summary: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    plugin_version: Option<String>,
    error_type: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub agent: Agent,
    pub kind: EventKind,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub turn_id: Option<String>,
    pub sequence: Option<u64>,
    pub start_source: Option<String>,
    pub query: Option<String>,
    pub response: Option<String>,
    pub transcript_path: Option<String>,
    pub summary: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input_preview: Option<String>,
    pub plugin_version: Option<String>,
    pub error_type: Option<String>,
    /// For a Claude `stop`: subagents still running in the background, read from the transcript
    /// before the event is applied.
    pub pending_agents: usize,
}

impl Event {
    /// `None` unless `title` is the plugin's sentinel and `body` a v1 event from a known agent.
    pub fn parse(title: &str, body: &str) -> Option<Event> {
        if title != SENTINEL {
            return None;
        }
        let raw: Notification = serde_json::from_str(body).ok()?;
        if raw.v.unwrap_or(1) != 1 {
            return None;
        }
        let kind = match raw.event.as_str() {
            "session_start" => EventKind::SessionStart,
            "prompt_submit" => EventKind::PromptSubmit,
            "tool_complete" => EventKind::ToolComplete,
            "stop" => EventKind::Stop,
            "stop_failure" => EventKind::StopFailure,
            "permission_request" => EventKind::PermissionRequest,
            "permission_replied" => EventKind::PermissionReplied,
            "question_asked" => EventKind::QuestionAsked,
            "idle_prompt" => EventKind::IdlePrompt,
            _ => EventKind::Unknown,
        };
        let tool_input_preview = raw.tool_input.as_ref().and_then(|input| {
            input
                .get("command")
                .or_else(|| input.get("file_path"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        });
        Some(Event {
            agent: Agent::from_event_name(raw.agent.as_deref()?)?,
            kind,
            session_id: raw.session_id,
            cwd: raw.cwd,
            project: raw.project,
            turn_id: raw.turn_id,
            sequence: raw.event_sequence,
            start_source: raw.session_start_source,
            query: raw.query,
            response: raw.response,
            transcript_path: raw.transcript_path,
            summary: raw.summary,
            tool_name: raw.tool_name,
            tool_input_preview,
            plugin_version: raw.plugin_version,
            error_type: raw.error_type,
            pending_agents: 0,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Restore {
    /// Run as soon as the shell reads its first command line.
    Run(String),
    /// Typed into the prompt for the user to run: a guess, not a known conversation.
    Type(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub agent: Agent,
    pub status: Status,
    pub session_id: Option<String>,
    /// First word of the command that started the agent (e.g. a shell function `cl`).
    pub launcher: Option<String>,
    /// The whole command line, kept for the flags a Codex resume must repeat.
    pub launch_command: Option<String>,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub query: Option<String>,
    pub response: Option<String>,
    pub transcript_path: Option<String>,
    pub summary: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input_preview: Option<String>,
    pub plugin_version: Option<String>,
    /// The user marked the finished turn as handled, so it reads as waiting instead of done.
    pub seen: bool,
    /// The user has looked at the pane since its status last changed.
    #[serde(default)]
    pub read: bool,
    /// When the status last changed, in Unix milliseconds.
    #[serde(default)]
    pub since: Option<u64>,
    /// The agent runs on another machine through ssh; resuming it here would start a new one.
    #[serde(default)]
    pub remote: bool,
    /// Subagents it started in the background that have not reported back; its turn ended but
    /// it is still at work.
    #[serde(default)]
    pub subagents: usize,
    #[serde(skip)]
    rich: bool,
    #[serde(skip)]
    order: EventOrder,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct EventOrder {
    turn_id: Option<String>,
    sequence: Option<u64>,
}

impl EventOrder {
    /// Drops events a plugin delivered late, after a newer event of the same session.
    fn accept(&mut self, event: &Event) -> bool {
        let sequence = event.sequence;
        if self
            .sequence
            .is_some_and(|previous| sequence.is_some_and(|next| next <= previous))
            || (self.sequence.is_some()
                && sequence.is_none()
                && matches!(
                    event.kind,
                    EventKind::SessionStart
                        | EventKind::PromptSubmit
                        | EventKind::Stop
                        | EventKind::StopFailure
                        | EventKind::IdlePrompt
                ))
        {
            return false;
        }
        if !matches!(
            event.kind,
            EventKind::PromptSubmit | EventKind::SessionStart
        ) && self.turn_id.is_some()
            && event.turn_id != self.turn_id
            && (event.turn_id.is_some() || sequence.is_none())
        {
            return false;
        }
        if event.kind == EventKind::PromptSubmit {
            self.turn_id = event.turn_id.clone();
        }
        self.sequence = sequence.or(self.sequence);
        true
    }
}

impl Session {
    /// A freshly started agent sits at its prompt: waiting, not done.
    pub fn new(agent: Agent, launcher: Option<String>, launch_command: Option<String>) -> Self {
        Session {
            agent,
            status: Status::Success,
            session_id: None,
            launcher,
            launch_command,
            cwd: None,
            project: None,
            query: None,
            response: None,
            transcript_path: None,
            summary: None,
            tool_name: None,
            tool_input_preview: None,
            plugin_version: None,
            seen: true,
            read: true,
            since: Some(now_ms()),
            remote: false,
            subagents: 0,
            rich: false,
            order: EventOrder::default(),
        }
    }

    /// A finished turn the user has not marked as handled.
    /// The agent's hooks report each prompt, so a letter typed in is seen being taken.
    pub fn reports_prompts(&self) -> bool {
        self.rich
    }

    /// Resting: done, failed or stopped. A blocked agent shows a permission or question prompt a
    /// letter must not answer.
    pub fn at_rest(&self) -> bool {
        matches!(
            self.status,
            Status::Success | Status::Failed { .. } | Status::Cancelled
        )
    }

    /// A finished turn that left the user something to answer, though its hook reported a stop.
    pub fn hold(&mut self, message: &str) -> bool {
        if self.status != Status::Success {
            return false;
        }
        self.set_status(Status::Blocked {
            message: Some(message.to_owned()),
        });
        true
    }

    /// Ends a turn the transcript shows was interrupted; Esc and a refused approval send no hook.
    pub fn cancel_turn(&mut self) -> bool {
        if !matches!(self.status, Status::InProgress | Status::Blocked { .. }) {
            return false;
        }
        self.set_status(Status::Cancelled);
        true
    }

    pub fn shows_done(&self) -> bool {
        self.status == Status::Success && !self.seen
    }

    /// What to put into the pane's shell when the workspace comes back.
    pub fn restore(&self) -> Option<Restore> {
        if self.remote {
            return None;
        }
        self.resume_command()
            .map(Restore::Run)
            .or_else(|| self.fallback_command().map(Restore::Type))
    }

    /// Command that reopens this conversation by itself; only for a reported, plain session ID.
    pub fn resume_command(&self) -> Option<String> {
        self.agent.resume_command_for_launch(
            self.launcher.as_deref(),
            self.session_id.as_deref()?,
            self.launch_command.as_deref(),
        )
    }

    /// What to type, without running it, when the conversation cannot be resumed by ID.
    pub fn fallback_command(&self) -> Option<String> {
        let launcher = self.launcher.as_deref();
        self.agent
            .session_picker_command(launcher)
            .or_else(|| self.agent.continue_command(launcher))
    }

    /// Applies a plugin event; `true` when anything a screen shows or a restart needs changed.
    pub fn apply(&mut self, event: &Event, claude_projects: &Path) -> bool {
        if event.agent != self.agent || !self.accepts(event, claude_projects) {
            return false;
        }
        let before = self.clone();
        self.rich = true;
        if let Some(path) = &event.transcript_path {
            self.transcript_path = Some(path.clone());
        }
        // A turn that ended without `stop` (e.g. Esc) is only announced by the idle prompt. Claude
        // also idles while its background subagents work; their results wake it.
        if event.kind == EventKind::IdlePrompt
            && self.status == Status::InProgress
            && self.subagents == 0
        {
            self.set_status(Status::Success);
        } else {
            self.apply_event(event);
        }
        *self != before
    }

    fn accepts(&mut self, event: &Event, claude_projects: &Path) -> bool {
        // Claude continues a resumed conversation under a new ID, so the first event after a
        // restore may replace the ID it started with; later strangers are another session's.
        if self.rich
            && self.session_id.is_some()
            && event.session_id.is_some()
            && event.session_id != self.session_id
            && !(event.kind == EventKind::SessionStart
                && matches!(
                    event.start_source.as_deref(),
                    Some("resume" | "fork" | "clear" | "clear_home")
                ))
            && !(self.agent == Agent::Claude
                && event
                    .session_id
                    .as_deref()
                    .is_some_and(|id| agent::claude_cleared_into(claude_projects, id)))
        {
            return false;
        }
        if matches!(
            event.kind,
            EventKind::Stop | EventKind::StopFailure | EventKind::IdlePrompt
        ) && event
            .query
            .as_deref()
            .is_some_and(|query| !transcript::is_task_notification(query))
            && self.query.is_some()
            && self.query.is_some()
            && event.query != self.query
        {
            return false;
        }
        self.order.accept(event)
    }

    fn apply_event(&mut self, event: &Event) {
        if event.kind == EventKind::SessionStart
            && matches!(event.start_source.as_deref(), Some("clear" | "clear_home"))
        {
            // A delayed clear hook must not overwrite a prompt submitted after /clear.
            return;
        }
        self.cwd = event.cwd.clone().or(self.cwd.take());
        self.project = event.project.clone().or(self.project.take());
        self.session_id = event
            .session_id
            .clone()
            .filter(|id| self.agent.resume_command(None, id).is_some())
            .or(self.session_id.take());
        let status = match event.kind {
            EventKind::PromptSubmit => {
                self.query = event.query.clone();
                self.response = None;
                self.clear_permission();
                Status::InProgress
            }
            EventKind::ToolComplete => match self.status {
                Status::Blocked { .. } => {
                    self.clear_permission();
                    Status::InProgress
                }
                // A tool finishing after the turn ended means a background task woke the agent.
                Status::Success | Status::Failed { .. } | Status::Cancelled => Status::InProgress,
                Status::InProgress => return,
            },
            EventKind::Stop => {
                // A turn a background task's result started keeps the user's request on show.
                if !event
                    .query
                    .as_deref()
                    .is_some_and(transcript::is_task_notification)
                {
                    self.query = event.query.clone();
                }
                self.response = event.response.clone();
                self.clear_permission();
                self.subagents = event.pending_agents;
                if event.pending_agents > 0 {
                    Status::InProgress
                } else {
                    Status::Success
                }
            }
            EventKind::StopFailure => {
                self.query = event.query.clone();
                self.response = event.response.clone();
                self.clear_permission();
                Status::Failed {
                    error_type: event.error_type.clone(),
                    message: event.response.clone(),
                }
            }
            EventKind::PermissionRequest => {
                self.summary = event.summary.clone();
                self.tool_name = event.tool_name.clone();
                self.tool_input_preview = event.tool_input_preview.clone();
                Status::Blocked {
                    message: event.summary.clone(),
                }
            }
            EventKind::QuestionAsked => Status::Blocked {
                message: event
                    .summary
                    .clone()
                    .or_else(|| Some("Waiting for your answer".to_owned())),
            },
            EventKind::PermissionReplied => {
                if !matches!(self.status, Status::Blocked { .. }) {
                    return;
                }
                self.clear_permission();
                Status::InProgress
            }
            EventKind::IdlePrompt | EventKind::Unknown => return,
            EventKind::SessionStart => {
                self.plugin_version = event.plugin_version.clone();
                // A session that just started or reopened sits at its prompt; nothing runs.
                let at_prompt = match event.start_source.as_deref() {
                    Some("startup" | "resume") => true,
                    // Claude's plugin hooks only startup|resume and does not forward the source.
                    None => self.agent == Agent::Claude,
                    Some(_) => false,
                };
                if self.status == Status::InProgress && at_prompt {
                    self.status = Status::Success;
                    self.seen = true;
                    self.read = true;
                }
                return;
            }
        };
        self.set_status(status);
    }

    fn set_status(&mut self, status: Status) {
        if status != self.status {
            // Each finished turn shows as done until the user marks it handled.
            self.seen = false;
            self.read = false;
            self.since = Some(now_ms());
        }
        self.status = status;
    }

    fn clear_permission(&mut self) {
        self.summary = None;
        self.tool_name = None;
        self.tool_input_preview = None;
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
