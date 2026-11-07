//! What Claude Code and Codex write to disk about a session: where the file is, the requests and
//! replies in it, and how its newest turn ended when no hook said so.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::agent::Agent;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Turn {
    /// When the request was sent, as the transcript wrote it (RFC 3339).
    pub at: Option<String>,
    pub request: String,
    /// The agent's last words before the next request.
    pub reply: Option<String>,
    /// The request answers a question the agent ended its previous turn with.
    pub answers: bool,
}

pub fn readable(agent: Agent) -> bool {
    matches!(agent, Agent::Claude | Agent::Codex)
}

pub fn codex_sessions(home: &Path) -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"))
        .join("sessions")
}

fn subdirs(parent: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
}

/// The session's transcript file under `home`.
pub fn find(agent: Agent, session_id: &str, home: &Path) -> Option<PathBuf> {
    match agent {
        Agent::Codex => {
            let suffix = format!("-{}.jsonl", Uuid::parse_str(session_id).ok()?);
            // ponytail: walks every day directory; index by id if lookups get slow.
            subdirs(&codex_sessions(home))
                .flat_map(|year| subdirs(&year).collect::<Vec<_>>())
                .flat_map(|month| subdirs(&month).collect::<Vec<_>>())
                .flat_map(|day| fs::read_dir(day).into_iter().flatten().flatten())
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(&suffix))
                })
        }
        Agent::Claude => {
            if session_id.is_empty()
                || !session_id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                return None;
            }
            fs::read_dir(home.join(".claude/projects"))
                .ok()?
                .flatten()
                .map(|dir| dir.path().join(format!("{session_id}.jsonl")))
                .find(|path| path.is_file())
        }
        _ => None,
    }
}

/// The path the agent's hook reported, or the one found from its session id.
pub fn locate(
    agent: Agent,
    reported: Option<&str>,
    session_id: Option<&str>,
    home: &Path,
) -> Option<PathBuf> {
    reported
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .or_else(|| find(agent, session_id?, home))
}

/// The last `max` bytes of a file, starting at a whole line, and whether anything was cut.
pub fn read_tail(path: &Path, max: u64) -> std::io::Result<(String, bool)> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let truncated = len > max;
    if truncated {
        file.seek(SeekFrom::Start(len - max))?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    // A seek lands mid-line; the partial first line is unparseable.
    let text = match (truncated, text.find('\n')) {
        (true, Some(newline)) => text[newline + 1..].to_owned(),
        _ => text.into_owned(),
    };
    Ok((text, truncated))
}

fn entries(jsonl: &str) -> impl Iterator<Item = Value> + '_ {
    jsonl
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
}

/// The text parts of a transcript message, joined and trimmed.
pub fn message_text(message: &Value) -> Option<String> {
    let text = match message.get("content")? {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn is_user_request(text: &str) -> bool {
    // Command wrappers, hook output and interruptions are written as user turns but were not typed.
    !text.starts_with('<') && !text.starts_with("[Request interrupted")
}

fn timestamp(entry: &Value) -> Option<String> {
    entry["timestamp"].as_str().map(str::to_owned)
}

fn push(turns: &mut Vec<Turn>, entry: &Value, request: String) {
    let answers = turns.last().is_some_and(|turn| {
        turn.reply
            .as_deref()
            .is_some_and(|reply| reply.trim_end().ends_with(['?', '？']))
    });
    turns.push(Turn {
        at: timestamp(entry),
        request,
        reply: None,
        answers,
    });
}

/// Every request in a transcript with the agent's last words after it, oldest first.
pub fn turns(agent: Agent, jsonl: &str) -> Vec<Turn> {
    let mut turns = Vec::new();
    for entry in entries(jsonl) {
        let (kind, text) = match agent {
            Agent::Codex => {
                let payload = &entry["payload"];
                if payload["type"] != "item_completed" {
                    continue;
                }
                let item = &payload["item"];
                let text = item["content"].as_array().map(|parts| {
                    parts
                        .iter()
                        .filter_map(|part| part["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim()
                        .to_owned()
                });
                let kind = match item["type"].as_str() {
                    Some("UserMessage") => "user",
                    Some("AgentMessage") => "assistant",
                    _ => continue,
                };
                (kind, text.filter(|text| !text.is_empty()))
            }
            _ => {
                let flag = |key: &str| entry[key].as_bool() == Some(true);
                if flag("isSidechain") || flag("isMeta") || flag("isCompactSummary") {
                    continue;
                }
                let kind = match entry["type"].as_str() {
                    Some("user") => "user",
                    Some("assistant") => "assistant",
                    _ => continue,
                };
                (kind, message_text(&entry["message"]))
            }
        };
        let Some(text) = text else { continue };
        match kind {
            "user" if is_user_request(&text) => push(&mut turns, &entry, text),
            "assistant" => {
                if let Some(turn) = turns.last_mut() {
                    turn.reply = Some(text);
                }
            }
            _ => {}
        }
    }
    turns
}

/// Whether the newest turn in a Claude Code transcript ended in an interruption: Esc and a
/// refused approval send no hook.
pub fn claude_interrupted(tail: &str) -> bool {
    for line in tail.lines().rev() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        match entry["type"].as_str() {
            Some("assistant") => return false,
            Some("user") => {
                let content = &entry["message"]["content"];
                let texts: Vec<&str> = match content {
                    Value::String(text) => vec![text],
                    Value::Array(parts) => {
                        parts.iter().filter_map(|p| p["text"].as_str()).collect()
                    }
                    _ => Vec::new(),
                };
                return texts
                    .iter()
                    .any(|text| text.starts_with("[Request interrupted by user"));
            }
            _ => {}
        }
    }
    false
}

/// Whether a shell command waits on a Naru child (`naru agent wait`, `naru agent spawn --wait`).
fn waits_on_child(command: &str) -> bool {
    command.contains("naru agent wait")
        || (command.contains("naru agent spawn") && command.contains("--wait"))
}

/// Agents Claude waits on in the background that have not come back with a `<task-notification>`
/// yet: its own subagents (`Agent` tool results with `isAsync`) and Naru children it waits on with
/// a background `naru agent wait`. Other background shell commands are left out: a dev server runs
/// for as long as the session does.
pub fn claude_pending_agents(tail: &str) -> usize {
    let mut started: Vec<String> = Vec::new();
    let mut finished = std::collections::HashSet::new();
    let mut waits = std::collections::HashSet::new();
    for line in tail.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        if let Some(parts) = entry["message"]["content"].as_array() {
            for part in parts {
                if part["type"] == "tool_use"
                    && part["input"]["command"]
                        .as_str()
                        .is_some_and(waits_on_child)
                    && let Some(id) = part["id"].as_str()
                {
                    waits.insert(id.to_owned());
                }
            }
        }
        let result = &entry["toolUseResult"];
        if result["isAsync"].as_bool() == Some(true)
            && let Some(id) = result["agentId"].as_str()
        {
            started.push(id.to_owned());
        }
        if let Some(task) = result["backgroundTaskId"].as_str()
            && entry["message"]["content"][0]["tool_use_id"]
                .as_str()
                .is_some_and(|call| waits.contains(call))
        {
            started.push(task.to_owned());
        }
        let notice = entry["content"]
            .as_str()
            .or_else(|| entry["message"]["content"].as_str());
        if let Some(notice) = notice
            && let Some(rest) = notice.trim_start().strip_prefix("<task-notification>")
            && let Some((_, rest)) = rest.split_once("<task-id>")
            && let Some((id, _)) = rest.split_once("</task-id>")
        {
            finished.insert(id.trim().to_owned());
        }
    }
    started.sort();
    started.dedup();
    started.iter().filter(|id| !finished.contains(*id)).count()
}

/// A turn Claude started on its own to take in a background task's result.
pub fn is_task_notification(query: &str) -> bool {
    query.trim_start().starts_with("<task-notification>")
}

/// Whether the newest turn in a Codex rollout was aborted.
pub fn codex_interrupted(tail: &str) -> bool {
    for line in tail.lines().rev() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry["type"] != "event_msg" {
            continue;
        }
        match entry["payload"]["type"].as_str() {
            Some("turn_aborted") => return true,
            Some("task_started" | "task_complete") => return false,
            _ => {}
        }
    }
    false
}

/// What a finished Codex turn leaves the user to answer, though its hook reports a plain stop:
/// a plan a plan-mode turn proposed ("Implement this plan?"), or a question call, with no message
/// from a person after it.
pub fn codex_waits(tail: &str) -> Option<&'static str> {
    let entries: Vec<Value> = entries(tail).collect();
    let start = entries.iter().rposition(|entry| {
        entry["type"] == "event_msg" && entry["payload"]["type"] == "task_started"
    })?;
    let plan_mode = entries[start]["payload"]["collaboration_mode_kind"] == "plan";
    let mut plan = false;
    // `request_user_input` is answered by its call's output; the async form returns at once and
    // is answered only by the person's reply message.
    let mut open_calls = Vec::new();
    let mut async_question = false;
    for entry in &entries[start + 1..] {
        let payload = &entry["payload"];
        match (entry["type"].as_str(), payload["type"].as_str()) {
            (Some("event_msg"), Some("turn_aborted")) => return None,
            (Some("event_msg"), Some("item_completed")) if payload["item"]["type"] == "Plan" => {
                plan = true
            }
            (Some("event_msg"), Some("user_message")) => {
                plan = false;
                async_question = false;
            }
            (Some("response_item"), Some("function_call")) => match payload["name"].as_str() {
                Some("request_user_input") => open_calls.push(payload["call_id"].clone()),
                Some("request_user_input_async") => async_question = true,
                _ => {}
            },
            (Some("response_item"), Some("function_call_output")) => {
                open_calls.retain(|call| *call != payload["call_id"])
            }
            (Some("response_item"), Some("message")) => match payload["role"].as_str() {
                Some("user") => {
                    plan = false;
                    async_question = false;
                }
                Some("assistant") => {
                    plan |= payload["content"].as_array().is_some_and(|parts| {
                        parts.iter().any(|part| {
                            part["text"].as_str().is_some_and(|text| {
                                text.trim_start().starts_with("<proposed_plan>")
                            })
                        })
                    })
                }
                _ => {}
            },
            _ => {}
        }
    }
    if plan_mode && plan {
        Some("계획 승인 대기")
    } else if async_question || !open_calls.is_empty() {
        Some("질문 답 대기")
    } else {
        None
    }
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
