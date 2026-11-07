//! 비용: what the agents in each project spent over the last seven days, at API prices, read from
//! the transcripts Claude Code (`~/.claude/projects`) and Codex (`~/.codex/sessions`) write. No
//! model is called. A call whose model has no price here is counted apart, never as `$0`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use chrono::{Duration, Local, NaiveDate, TimeZone};
use serde::Serialize;
use serde_json::Value;

use crate::state::{PaneId, Workspace};

pub const DAYS: usize = 7;
const TOP: usize = 3;
const TITLE_CHARS: usize = 60;
const MTOK: f64 = 1_000_000.0;

/// USD per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Price {
    input: f64,
    output: f64,
    cache_read: f64,
    write_5m: f64,
    write_1h: f64,
}

/// Anthropic first-party prices (claude-api skill, cached 2026-10-06). Cache writes are 1.25× input
/// for the 5-minute TTL and 2× for the hour; reads are what the model's row says.
fn anthropic(input: f64, output: f64, cache_read: f64) -> Price {
    Price {
        input,
        output,
        cache_read,
        write_5m: input * 1.25,
        write_1h: input * 2.0,
    }
}

fn claude_price(model: &str, prompt_tokens: u64) -> Option<Price> {
    Some(match model {
        "claude-fable-5-1" => anthropic(10.0, 50.0, 0.25),
        "claude-opus-5-5" => anthropic(4.0, 20.0, 0.20),
        "claude-sonnet-5-5" | "claude-sonnet-5" => anthropic(2.0, 10.0, 0.20),
        // Haiku 5.5 doubles past 100K prompt tokens ($0.50 / $2.50); reads at the usual 0.1×.
        "claude-haiku-5-5" if prompt_tokens > 100_000 => anthropic(0.50, 2.50, 0.05),
        "claude-haiku-5-5" => anthropic(0.10, 0.50, 0.01),
        _ => return None,
    })
}

/// Not OpenAI's own page: third-party tables (requesty.ai, cloudzero.com) checked 2026-11-07.
/// ponytail: gpt-6.1-sol's long-context tier ($4 / $0.20 / $15) is not applied, its threshold
/// was not published there; a long session reads low.
fn codex_price(model: &str) -> Option<Price> {
    let (input, cache_read, output) = match model {
        "gpt-6.1-sol" => (2.0, 0.10, 10.0),
        "gpt-6-sol" => (2.0, 0.20, 10.0),
        "gpt-5.6-terra" => (2.0, 0.20, 12.0),
        _ => return None,
    };
    Some(Price {
        input,
        output,
        cache_read,
        write_5m: input,
        write_1h: input,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Usage {
    input: u64,
    output: u64,
    cache_read: u64,
    write_5m: u64,
    write_1h: u64,
}

impl Usage {
    fn tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.write_5m + self.write_1h
    }

    fn cost(&self, price: Price) -> f64 {
        (self.input as f64 * price.input
            + self.output as f64 * price.output
            + self.cache_read as f64 * price.cache_read
            + self.write_5m as f64 * price.write_5m
            + self.write_1h as f64 * price.write_1h)
            / MTOK
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Call {
    /// Unix seconds.
    at: i64,
    model: String,
    tokens: u64,
    /// `None`: no price for this model (or a fast-mode Claude call, whose cache prices are unknown).
    cost: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Read {
    session: String,
    agent: &'static str,
    cwd: Option<String>,
    first: Option<String>,
    calls: Vec<Call>,
}

fn at(line: &Value) -> Option<i64> {
    let text = line.get("timestamp")?.as_str()?;
    Some(chrono::DateTime::parse_from_rfc3339(text).ok()?.timestamp())
}

fn count(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn clip(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.char_indices().nth(TITLE_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

/// One Claude transcript. Claude writes each content block of a reply as its own line with the
/// same `message.id` and usage, so a message counts once.
fn read_claude(text: &str, session: String) -> Read {
    let mut read = Read {
        session,
        agent: "claude",
        ..Default::default()
    };
    let mut seen = HashSet::new();
    for line in text.lines() {
        let Ok(line) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if read.cwd.is_none() {
            read.cwd = line.get("cwd").and_then(Value::as_str).map(str::to_owned);
        }
        if read.first.is_none()
            && line.get("type").and_then(Value::as_str) == Some("user")
            && line.get("isMeta").and_then(Value::as_bool) != Some(true)
            && let Some(said) = line.pointer("/message/content").and_then(Value::as_str)
            && !said.trim_start().starts_with('<')
            && !said.trim().is_empty()
        {
            read.first = Some(clip(said));
        }
        let Some(message) = line.get("message") else {
            continue;
        };
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let (Some(model), Some(when)) = (message.get("model").and_then(Value::as_str), at(&line))
        else {
            continue;
        };
        if model.starts_with('<') {
            continue;
        }
        if let Some(id) = message.get("id").and_then(Value::as_str)
            && !seen.insert(id.to_owned())
        {
            continue;
        }
        let written = count(usage.get("cache_creation_input_tokens"));
        let (write_5m, write_1h) = match usage.get("cache_creation") {
            Some(split) => (
                count(split.get("ephemeral_5m_input_tokens")),
                count(split.get("ephemeral_1h_input_tokens")),
            ),
            None => (written, 0),
        };
        let used = Usage {
            input: count(usage.get("input_tokens")),
            output: count(usage.get("output_tokens")),
            cache_read: count(usage.get("cache_read_input_tokens")),
            write_5m,
            write_1h,
        };
        let prompt = used.input + used.cache_read + used.write_5m + used.write_1h;
        let fast = usage.get("speed").and_then(Value::as_str) == Some("fast");
        read.calls.push(Call {
            at: when,
            model: model.to_owned(),
            tokens: used.tokens(),
            cost: (!fast)
                .then(|| claude_price(model, prompt))
                .flatten()
                .map(|price| used.cost(price)),
        });
    }
    read
}

/// One Codex rollout. `token_count` events carry the session's running total; each call is the
/// growth since the last one, priced at the model the latest `turn_context` named.
fn read_codex(text: &str) -> Read {
    let mut read = Read {
        agent: "codex",
        ..Default::default()
    };
    let mut model: Option<String> = None;
    let mut last = Usage::default();
    for line in text.lines() {
        let Ok(line) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = line.get("payload");
        match line.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                let payload = payload.unwrap_or(&Value::Null);
                if let Some(id) = payload.get("id").and_then(Value::as_str) {
                    read.session = id.to_owned();
                }
                read.cwd = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            Some("turn_context") => {
                model = payload
                    .and_then(|payload| payload.get("model"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            Some("event_msg") => {
                let payload = payload.unwrap_or(&Value::Null);
                match payload.get("type").and_then(Value::as_str) {
                    Some("user_message") if read.first.is_none() => {
                        read.first = payload
                            .get("message")
                            .and_then(Value::as_str)
                            .filter(|said| !said.trim().is_empty())
                            .map(clip);
                    }
                    Some("token_count") => {
                        let Some(total) = payload.pointer("/info/total_token_usage") else {
                            continue;
                        };
                        let cached = count(total.get("cached_input_tokens"));
                        let mut output = count(total.get("output_tokens"));
                        if payload.pointer("/info/last_usage_includes_reasoning")
                            == Some(&Value::Bool(false))
                        {
                            output += count(total.get("reasoning_output_tokens"));
                        }
                        let now = Usage {
                            input: count(total.get("input_tokens")).saturating_sub(cached),
                            cache_read: cached,
                            output,
                            ..Default::default()
                        };
                        let grown = Usage {
                            input: now.input.saturating_sub(last.input),
                            cache_read: now.cache_read.saturating_sub(last.cache_read),
                            output: now.output.saturating_sub(last.output),
                            ..Default::default()
                        };
                        last = now;
                        let (Some(model), Some(when)) = (model.as_deref(), at(&line)) else {
                            continue;
                        };
                        if grown.tokens() == 0 {
                            continue;
                        }
                        read.calls.push(Call {
                            at: when,
                            model: model.to_owned(),
                            tokens: grown.tokens(),
                            cost: codex_price(model).map(|price| grown.cost(price)),
                        });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    read
}

/// Transcripts already read, by path, with the modification time and size they had.
type Cached = (SystemTime, u64, Arc<Read>);
static READS: Mutex<BTreeMap<PathBuf, Cached>> = Mutex::new(BTreeMap::new());

fn read_cached(path: &Path, parse: impl FnOnce(&str) -> Read) -> Option<Arc<Read>> {
    let meta = fs::metadata(path).ok()?;
    let stamp = (meta.modified().ok()?, meta.len());
    if let Some((modified, len, read)) = READS.lock().expect("cost cache").get(path)
        && (*modified, *len) == stamp
    {
        return Some(read.clone());
    }
    let text = fs::read_to_string(path).ok()?;
    let read = Arc::new(parse(&text));
    READS
        .lock()
        .expect("cost cache")
        .insert(path.to_owned(), (stamp.0, stamp.1, read.clone()));
    Some(read)
}

fn files_since(dir: &Path, since: SystemTime, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            files_since(&path, since, out);
        } else if path.extension().is_some_and(|ext| ext == "jsonl")
            && entry
                .metadata()
                .and_then(|meta| meta.modified())
                .is_ok_and(|modified| modified >= since)
        {
            out.push(path);
        }
    }
}

/// Every transcript written since `since`. A Claude subagent's file
/// (`<session>/subagents/agent-….jsonl`) counts toward the session that started it.
fn transcripts(home: &Path, since: SystemTime) -> Vec<Arc<Read>> {
    let mut claude = Vec::new();
    files_since(&home.join(".claude/projects"), since, &mut claude);
    let mut codex = Vec::new();
    files_since(&crate::transcript::codex_sessions(home), since, &mut codex);
    let mut reads = Vec::new();
    for path in claude {
        let parent = path.parent().and_then(Path::file_name);
        let session = if parent.is_some_and(|name| name == "subagents") {
            path.parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
        } else {
            path.file_stem()
        };
        let session = session.map(|name| name.to_string_lossy().into_owned());
        let Some(session) = session else { continue };
        reads.extend(read_cached(&path, |text| read_claude(text, session)));
    }
    for path in codex {
        reads.extend(read_cached(&path, read_codex));
    }
    reads
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ModelCost {
    pub model: String,
    /// `None` when the model has no price.
    pub cost: Option<f64>,
    pub calls: u32,
    pub tokens: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SessionCost {
    pub title: String,
    pub agent: &'static str,
    pub cost: f64,
    pub tokens: u64,
    /// Calls with no price, left out of `cost`.
    pub unpriced: u32,
    /// The open pane running it, if any.
    pub pane: Option<PaneId>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
pub struct ProjectCost {
    pub name: String,
    /// `None` for work outside every project.
    pub path: Option<String>,
    pub total: f64,
    /// Calls with no price, left out of `total`.
    pub unpriced: u32,
    /// One amount per day, oldest first, matching [`Costs::days`].
    pub days: Vec<f64>,
    pub models: Vec<ModelCost>,
    pub sessions: usize,
    /// The costliest sessions.
    pub top: Vec<SessionCost>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Costs {
    /// `11/01` … today.
    pub days: Vec<String>,
    pub projects: Vec<ProjectCost>,
}

/// The project a folder belongs to: the group with the longest path containing it.
fn project_of<'a>(
    groups: &'a [(String, String)],
    cwd: Option<&str>,
) -> Option<&'a (String, String)> {
    let cwd = cwd?;
    groups
        .iter()
        .filter(|(_, path)| {
            let path = path.trim_end_matches('/');
            cwd == path || cwd.starts_with(&format!("{path}/"))
        })
        .max_by_key(|(_, path)| path.len())
}

fn total(reads: &[Arc<Read>], workspace: &Workspace, first_day: NaiveDate) -> Costs {
    let groups: Vec<(String, String)> = workspace
        .groups
        .iter()
        .filter_map(|group| Some((group.name.clone(), group.path.clone()?)))
        .collect();
    let panes: HashMap<&str, PaneId> = workspace
        .panes
        .iter()
        .filter_map(|(pane, record)| Some((record.session.as_ref()?.session_id.as_deref()?, *pane)))
        .collect();
    let start = Local
        .from_local_datetime(&first_day.and_hms_opt(0, 0, 0).expect("midnight"))
        .earliest()
        .map_or(i64::MAX, |at| at.timestamp());
    let day_of = |at: i64| -> Option<usize> {
        if at < start {
            return None;
        }
        let date = Local.timestamp_opt(at, 0).single()?.date_naive();
        let index = (date - first_day).num_days();
        (0..DAYS as i64).contains(&index).then_some(index as usize)
    };
    // (project path) → (project, sessions by id)
    let mut projects: BTreeMap<Option<String>, (ProjectCost, BTreeMap<String, SessionCost>)> =
        BTreeMap::new();
    let mut models: BTreeMap<(Option<String>, String), ModelCost> = BTreeMap::new();
    for read in reads {
        let calls: Vec<(usize, &Call)> = read
            .calls
            .iter()
            .filter_map(|call| Some((day_of(call.at)?, call)))
            .collect();
        if calls.is_empty() {
            continue;
        }
        let found = project_of(&groups, read.cwd.as_deref());
        let key = found.map(|(_, path)| path.clone());
        let (project, sessions) = projects.entry(key.clone()).or_insert_with(|| {
            (
                ProjectCost {
                    name: found.map_or_else(|| "프로젝트 밖".to_owned(), |(name, _)| name.clone()),
                    path: key.clone(),
                    days: vec![0.0; DAYS],
                    ..Default::default()
                },
                BTreeMap::new(),
            )
        });
        let pane = panes.get(read.session.as_str()).copied();
        let session = sessions
            .entry(read.session.clone())
            .or_insert_with(|| SessionCost {
                title: pane
                    .map(|pane| crate::team::pane_title(workspace, pane))
                    .filter(|title| !title.is_empty())
                    .or_else(|| read.first.clone())
                    .unwrap_or_else(|| read.session.chars().take(8).collect()),
                agent: read.agent,
                cost: 0.0,
                tokens: 0,
                unpriced: 0,
                pane,
            });
        for (day, call) in calls {
            let model = models
                .entry((key.clone(), call.model.clone()))
                .or_insert_with(|| ModelCost {
                    model: call.model.clone(),
                    cost: Some(0.0),
                    calls: 0,
                    tokens: 0,
                });
            model.calls += 1;
            model.tokens += call.tokens;
            session.tokens += call.tokens;
            match call.cost {
                Some(cost) => {
                    project.total += cost;
                    project.days[day] += cost;
                    session.cost += cost;
                    if let Some(sum) = model.cost.as_mut() {
                        *sum += cost;
                    }
                }
                None => {
                    project.unpriced += 1;
                    session.unpriced += 1;
                    model.cost = None;
                }
            }
        }
    }
    for ((key, _), model) in models {
        if let Some((project, _)) = projects.get_mut(&key) {
            project.models.push(model);
        }
    }
    let mut list: Vec<ProjectCost> = projects
        .into_values()
        .map(|(mut project, sessions)| {
            project.sessions = sessions.len();
            let mut top: Vec<SessionCost> = sessions.into_values().collect();
            top.sort_by(|a, b| b.cost.total_cmp(&a.cost).then(b.tokens.cmp(&a.tokens)));
            top.truncate(TOP);
            project.top = top;
            project
                .models
                .sort_by(|a, b| b.cost.unwrap_or(-1.0).total_cmp(&a.cost.unwrap_or(-1.0)));
            project
        })
        .collect();
    list.sort_by(|a, b| b.total.total_cmp(&a.total));
    Costs {
        days: (0..DAYS)
            .map(|index| {
                (first_day + Duration::days(index as i64))
                    .format("%m/%d")
                    .to_string()
            })
            .collect(),
        projects: list,
    }
}

/// The last seven days, today included, by project.
pub fn costs(home: &Path, workspace: &Workspace) -> Costs {
    let first_day = Local::now().date_naive() - Duration::days(DAYS as i64 - 1);
    let since = Local
        .from_local_datetime(&first_day.and_hms_opt(0, 0, 0).expect("midnight"))
        .earliest()
        .map_or(SystemTime::UNIX_EPOCH, SystemTime::from);
    total(&transcripts(home, since), workspace, first_day)
}

#[cfg(test)]
#[path = "cost_tests.rs"]
mod tests;
