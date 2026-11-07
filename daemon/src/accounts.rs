//! The bottom bar's agent accounts: who each CLI is signed in as, how much of its limits is used,
//! and switching between saved logins.
//!
//! Limits come only from sources that cost no model turn: the statusline cache the user's Claude
//! statusline writes from Anthropic's usage endpoint, the newest Codex rollout's `rate_limits`, and
//! `agy -p /usage`. A limit read while an account was signed in is kept for that account in
//! `~/.naru/account-usage.json`, so an account that is not signed in shows its last value and when
//! it was read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claude_accounts;
use crate::core::Daemon;
use crate::file_accounts::{FileAgent, Live};
use crate::state::PaneId;
use crate::suggestions::write_private;
use crate::transcript::read_tail;

const TAIL_BYTES: u64 = 256 * 1024;
/// Codex day folders searched for the newest rollout.
const CODEX_DAYS: usize = 7;
/// `agy -p /usage` takes seconds, so it runs in the background at most this often.
const AGY_EVERY: Duration = Duration::from_secs(10 * 60);
const AGY_TIMEOUT: Duration = Duration::from_secs(60);
/// A login started from the screen that has not finished by then is dropped.
const LOGIN_WAIT: Duration = Duration::from_secs(30 * 60);
/// The id of a login that is signed in but saved as no account.
const LIVE: &str = "live";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    Agy,
    Grok,
}

impl AgentKind {
    fn key(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Agy => "agy",
            AgentKind::Grok => "grok",
        }
    }

    fn name(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude",
            AgentKind::Codex => "Codex",
            AgentKind::Agy => "agy",
            AgentKind::Grok => "Grok",
        }
    }
}

/// One limit window, such as the five-hour or the weekly one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Window {
    pub minutes: u64,
    /// Share used, 0–100.
    pub percent: f64,
    /// Unix seconds.
    pub resets_at: Option<i64>,
}

/// Limits as a source recorded them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Seen {
    pub windows: Vec<Window>,
    /// Unix seconds.
    pub at: i64,
}

#[derive(Serialize, Debug)]
pub struct AccountView {
    pub id: String,
    /// The name the user gave it, else its email.
    pub label: String,
    pub detail: Option<String>,
    pub active: bool,
    /// Signed in but saved as no account: it has to be saved before switching away from it.
    pub unsaved: bool,
    pub usage: Option<Seen>,
}

#[derive(Serialize, Debug)]
pub struct AgentView {
    pub agent: AgentKind,
    pub installed: bool,
    pub accounts: Vec<AccountView>,
    /// Saved logins of this agent can be switched between.
    pub switchable: bool,
    /// The label of the button that signs in, when the agent has one.
    pub add: Option<&'static str>,
    /// What the agent cannot show, e.g. limits it does not report.
    pub note: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "a", rename_all = "snake_case")]
pub enum AccountAct {
    Switch {
        agent: AgentKind,
        id: String,
    },
    /// Opens a pane that signs a new account in.
    Add {
        agent: AgentKind,
    },
    /// Saves the signed-in login as an account.
    SaveCurrent {
        agent: AgentKind,
    },
    Remove {
        agent: AgentKind,
        id: String,
    },
    Rename {
        agent: AgentKind,
        id: String,
        name: String,
    },
}

/// What an action did, for the screen to show.
#[derive(Serialize, Debug, Default)]
pub struct ActDone {
    pub notice: Option<String>,
    /// A pane the action opened, which the screen focuses.
    pub opened: Option<PaneId>,
}

#[derive(Default)]
pub struct Accounts {
    /// Logins started from the screen and not finished yet.
    pending: Mutex<Vec<Pending>>,
    agy: Mutex<AgyState>,
    /// One switch or save at a time.
    changing: Mutex<()>,
}

struct Pending {
    agent: AgentKind,
    id: String,
    started: Instant,
}

#[derive(Default)]
struct AgyState {
    /// The limits last read, with the key of the agy login they were read as.
    last: Option<(Option<String>, Result<Seen, String>)>,
    checked: Option<Instant>,
    running: bool,
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn rfc3339(value: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|time| time.timestamp())
}

fn mtime(path: &Path) -> Option<i64> {
    let modified = path.metadata().ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64)
}

/// Where the installers put each CLI; the daemon's own PATH is the bare one an app gets.
pub fn program(home: &Path, name: &str) -> Option<PathBuf> {
    [
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".grok/bin"),
    ]
    .into_iter()
    .map(|dir| dir.join(name))
    .find(|path| path.is_file())
}

/// Reads the statusline cache's `five_hour` and `seven_day` windows.
pub fn parse_claude_cache(text: &str) -> Option<Seen> {
    let cache: Value = serde_json::from_str(text).ok()?;
    let windows: Vec<Window> = [("five_hour", 300), ("seven_day", 7 * 24 * 60)]
        .into_iter()
        .filter_map(|(key, minutes)| {
            let window = &cache[key];
            Some(Window {
                minutes,
                percent: window["utilization"].as_f64()?,
                resets_at: rfc3339(&window["resets_at"]),
            })
        })
        .collect();
    Some(Seen {
        windows,
        at: rfc3339(&cache["cached_at"])?,
    })
    .filter(|seen| !seen.windows.is_empty())
}

/// Reads the newest `rate_limits` record in a Codex rollout tail.
pub fn parse_codex_tail(tail: &str) -> Option<Seen> {
    tail.lines().rev().find_map(|line| {
        if !line.contains("\"rate_limits\"") {
            return None;
        }
        let entry: Value = serde_json::from_str(line).ok()?;
        let limits = &entry["payload"]["rate_limits"];
        let windows: Vec<Window> = ["primary", "secondary"]
            .into_iter()
            .filter_map(|key| {
                let window = &limits[key];
                Some(Window {
                    minutes: window["window_minutes"].as_u64()?,
                    percent: window["used_percent"].as_f64()?,
                    resets_at: window["resets_at"].as_i64(),
                })
            })
            .collect();
        Some(Seen {
            windows,
            at: rfc3339(&entry["timestamp"])?,
        })
        .filter(|seen| !seen.windows.is_empty())
    })
}

/// Reads `agy -p /usage --output-format json`: each model group's buckets, the most used group
/// per window.
pub fn parse_agy(text: &str, at: i64) -> anyhow::Result<Seen> {
    let reply: Value = serde_json::from_str(text).context("agy did not answer in JSON")?;
    ensure!(
        reply["status"] == "SUCCESS",
        "agy /usage failed: {}",
        reply["status"]
    );
    let mut windows: Vec<Window> = Vec::new();
    for bucket in reply["command"]["data"]["groups"]
        .as_array()
        .context("agy /usage has no groups")?
        .iter()
        .flat_map(|group| group["buckets"].as_array().into_iter().flatten())
    {
        let minutes = match bucket["window"].as_str() {
            Some("5h") => 300,
            Some("weekly") => 7 * 24 * 60,
            _ => continue,
        };
        let Some(remaining) = bucket["remaining_fraction"].as_f64() else {
            continue;
        };
        let window = Window {
            minutes,
            percent: ((1.0 - remaining) * 1000.0).round() / 10.0,
            resets_at: rfc3339(&bucket["reset_time"]),
        };
        match windows.iter_mut().find(|w| w.minutes == minutes) {
            Some(known) if known.percent >= window.percent => {}
            Some(known) => *known = window,
            None => windows.push(window),
        }
    }
    ensure!(!windows.is_empty(), "agy /usage reported no limits");
    windows.sort_by_key(|w| w.minutes);
    Ok(Seen { windows, at })
}

/// Rollouts of the newest Codex day folders, newest first by creation.
fn codex_rollouts(sessions: &Path) -> Vec<(SystemTime, PathBuf)> {
    let list = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect()
    };
    let mut days: Vec<PathBuf> = list(sessions)
        .iter()
        .flat_map(|year| list(year))
        .flat_map(|month| list(&month))
        .collect();
    days.sort();
    let mut files: Vec<(SystemTime, PathBuf)> = days
        .iter()
        .rev()
        .take(CODEX_DAYS)
        .flat_map(|day| list(day))
        .filter_map(|file| Some((file.metadata().ok()?.modified().ok()?, file)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
}

/// The limits the newest Codex session started at or after `since` recorded. A session started
/// earlier may still run under the login that was signed in before.
fn codex_usage(codex_home: &Path, since: i64) -> Option<Seen> {
    codex_rollouts(&codex_home.join("sessions"))
        .into_iter()
        .filter(|(_, path)| {
            path.metadata()
                .and_then(|meta| meta.created())
                .ok()
                .and_then(|created| created.duration_since(UNIX_EPOCH).ok())
                .is_some_and(|created| created.as_secs() as i64 >= since)
        })
        .find_map(|(_, path)| parse_codex_tail(&read_tail(&path, TAIL_BYTES).ok()?.0))
}

// ---------- the usage and name stores ----------

type UsageStore = BTreeMap<String, Seen>;

fn store_key(agent: AgentKind, id: &str) -> String {
    format!("{}:{id}", agent.key())
}

fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Keeps `seen` for the account when it is newer than what is kept.
fn record(store: &mut UsageStore, agent: AgentKind, id: &str, seen: Seen) -> bool {
    let key = store_key(agent, id);
    if store.get(&key).is_some_and(|kept| kept.at >= seen.at) {
        return false;
    }
    store.insert(key, seen);
    true
}

// ---------- the daemon side ----------

impl AgentKind {
    /// The login file Naru swaps; Claude's login lives in the Keychain instead.
    fn file_agent(self, home: &Path) -> Option<FileAgent> {
        match self {
            AgentKind::Claude => None,
            AgentKind::Codex => Some(FileAgent::codex(home)),
            AgentKind::Agy => Some(FileAgent::agy(home)),
            AgentKind::Grok => Some(FileAgent::grok(home)),
        }
    }

    /// The pane line that signs a new account in, writing its login into `dir`.
    fn login_command(self, dir: &Path) -> anyhow::Result<String> {
        let quoted = |sub: &str| shell_words::quote(&dir.join(sub).to_string_lossy()).into_owned();
        Ok(match self {
            AgentKind::Claude => bail!("Claude logins start in claude_accounts"),
            AgentKind::Codex => {
                // Codex keeps a login in the Keychain unless its config says file.
                write_private(
                    &dir.join("home/config.toml"),
                    b"cli_auth_credentials_store = \"file\"\n",
                )?;
                format!("CODEX_HOME={} codex login", quoted("home"))
            }
            AgentKind::Grok => format!("GROK_HOME={} grok login", quoted("home")),
            // agy has no login command; it asks to sign in when it starts without a login.
            AgentKind::Agy => format!("HOME={} agy", quoted("home")),
        })
    }
}

impl Daemon {
    fn accounts_root(&self, agent: AgentKind) -> PathBuf {
        self.naru_dir.join("accounts").join(agent.key())
    }

    fn usage_path(&self) -> PathBuf {
        self.naru_dir.join("account-usage.json")
    }

    fn names_path(&self) -> PathBuf {
        self.naru_dir.join("account-names.json")
    }

    /// Every agent's accounts and limits, finishing logins that completed since the last look.
    pub fn accounts_view(self: &Arc<Self>) -> Vec<AgentView> {
        self.finish_logins();
        self.refresh_agy_if_stale();
        let mut store: UsageStore = read_json(&self.usage_path());
        let before = store.clone();
        let names: BTreeMap<String, String> = read_json(&self.names_path());
        let mut views = vec![self.claude_view(&mut store, &names)];
        views.extend(
            [AgentKind::Codex, AgentKind::Agy, AgentKind::Grok]
                .map(|agent| self.file_view(agent, &mut store, &names)),
        );
        if store != before
            && let Err(error) = write_private(
                &self.usage_path(),
                &serde_json::to_vec(&store).unwrap_or_default(),
            )
        {
            eprintln!("accounts: cannot save usage: {error}");
        }
        views
    }

    fn claude_view(&self, store: &mut UsageStore, names: &BTreeMap<String, String>) -> AgentView {
        let agent = AgentKind::Claude;
        let root = claude_accounts::root(&self.home);
        let personal = self.home.join(".claude");
        let current = claude_accounts::current_email(&personal);
        let (saved, errors) =
            claude_accounts::list(&root).unwrap_or_else(|e| (Vec::new(), vec![format!("{e:#}")]));
        let live = saved.iter().find(|a| {
            current
                .as_deref()
                .is_some_and(|email| email.eq_ignore_ascii_case(&a.email))
        });
        let live_id = live
            .map(|a| a.id.to_string())
            .or_else(|| current.as_ref().map(|_| LIVE.into()));
        let cache = std::fs::read_to_string(personal.join("statusline_cache.json"))
            .ok()
            .and_then(|text| parse_claude_cache(&text));
        // A cache written before the last switch belongs to the login that was signed in then.
        let since = live
            .and_then(|_| mtime(&root.join("selected-claude")))
            .unwrap_or(0);
        if let (Some(id), Some(seen)) = (&live_id, cache.clone())
            && seen.at >= since
        {
            record(store, agent, id, seen);
        }
        let mut accounts: Vec<AccountView> = saved
            .iter()
            .map(|account| {
                let id = account.id.to_string();
                AccountView {
                    label: names
                        .get(&store_key(agent, &id))
                        .cloned()
                        .unwrap_or_else(|| account.email.clone()),
                    detail: detail(
                        [Some(account.email.as_str()), account.plan.as_deref()],
                        names.contains_key(&store_key(agent, &id)),
                    ),
                    active: live_id.as_deref() == Some(id.as_str()),
                    unsaved: false,
                    usage: store.get(&store_key(agent, &id)).cloned(),
                    id,
                }
            })
            .collect();
        if live.is_none()
            && let Some(email) = &current
        {
            accounts.insert(
                0,
                AccountView {
                    id: LIVE.into(),
                    label: email.clone(),
                    detail: Some("저장 안 된 로그인".into()),
                    active: true,
                    unsaved: true,
                    usage: store.get(&store_key(agent, LIVE)).cloned(),
                },
            );
        }
        AgentView {
            agent,
            installed: crate::titles::claude_path(&self.home).is_some(),
            switchable: saved.len() > 1 || (saved.len() == 1 && live.is_none()),
            add: Some("계정 추가"),
            note: cache.is_none().then(|| {
                "한도는 Claude statusline이 남기는 ~/.claude/statusline_cache.json에서 읽어요. 그 파일이 없어요".into()
            }),
            error: (!errors.is_empty()).then(|| errors.join("\n")),
            accounts,
        }
    }

    fn file_view(
        &self,
        agent: AgentKind,
        store: &mut UsageStore,
        names: &BTreeMap<String, String>,
    ) -> AgentView {
        let files = agent.file_agent(&self.home).expect("a file agent");
        let root = self.accounts_root(agent);
        let saved = files.accounts(&root);
        let (live, error) = match files.read_live() {
            Ok(live) => (live, None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        let owner = files.owner(&root, &saved, live.as_ref());
        let live_id = owner
            .map(|account| account.id.clone())
            .or_else(|| live.as_ref().map(|_| LIVE.into()));
        if let Some(id) = &live_id
            && let Some(seen) = self.live_usage(agent, &files, live.as_ref())
        {
            record(store, agent, id, seen);
        }
        let mut accounts: Vec<AccountView> = saved
            .iter()
            .map(|account| {
                let key = store_key(agent, &account.id);
                let login = account.login.clone().unwrap_or_default();
                AccountView {
                    label: names
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| account.label(files.fallback)),
                    detail: detail(
                        [
                            login.email.as_deref(),
                            login.plan.as_deref(),
                            login.detail.as_deref(),
                        ],
                        names.contains_key(&key),
                    ),
                    active: live_id.as_deref() == Some(account.id.as_str()),
                    unsaved: false,
                    usage: store.get(&key).cloned(),
                    id: account.id.clone(),
                }
            })
            .collect();
        if owner.is_none()
            && let Some(live) = &live
        {
            let login = live.login.clone().unwrap_or_default();
            accounts.insert(
                0,
                AccountView {
                    id: LIVE.into(),
                    label: login.email.unwrap_or_else(|| "지금 로그인".into()),
                    detail: Some(
                        [login.plan, login.detail, Some("저장 안 된 로그인".into())]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · "),
                    ),
                    active: true,
                    unsaved: true,
                    usage: store.get(&store_key(agent, LIVE)).cloned(),
                },
            );
        }
        let installed = program(&self.home, agent.key()).is_some();
        AgentView {
            agent,
            installed,
            switchable: saved.len() > 1 || (saved.len() == 1 && owner.is_none()),
            add: Some("계정 추가"),
            note: self.limits_note(agent, installed),
            error: error.or_else(|| self.agy_error(agent)),
            accounts,
        }
    }

    /// Limits read for the signed-in login of `agent`, if its source names that login.
    fn live_usage(&self, agent: AgentKind, files: &FileAgent, live: Option<&Live>) -> Option<Seen> {
        match agent {
            // A session started before the last switch may still run under the login before it.
            AgentKind::Codex => {
                codex_usage(&self.home.join(".codex"), mtime(&files.live).unwrap_or(0))
            }
            AgentKind::Agy => {
                let state = self.accounts.agy.lock().expect("agy lock poisoned");
                let key = live?.login.as_ref()?.key.clone();
                match &state.last {
                    Some((read_as, Ok(seen))) if *read_as == key => Some(seen.clone()),
                    _ => None,
                }
            }
            AgentKind::Claude | AgentKind::Grok => None,
        }
    }

    fn limits_note(&self, agent: AgentKind, installed: bool) -> Option<String> {
        match agent {
            AgentKind::Agy if installed => {
                let state = self.accounts.agy.lock().expect("agy lock poisoned");
                Some(if state.last.is_none() {
                    "한도를 읽는 중이에요".into()
                } else {
                    "한도는 10분마다 agy /usage로 읽어요".into()
                })
            }
            AgentKind::Grok => Some("Grok은 한도를 알려 주지 않아요.".into()),
            AgentKind::Claude | AgentKind::Codex | AgentKind::Agy => None,
        }
    }

    fn agy_error(&self, agent: AgentKind) -> Option<String> {
        if agent != AgentKind::Agy {
            return None;
        }
        let state = self.accounts.agy.lock().expect("agy lock poisoned");
        match &state.last {
            Some((_, Err(error))) => Some(error.clone()),
            _ => None,
        }
    }

    fn refresh_agy_if_stale(self: &Arc<Self>) {
        let Some(agy) = program(&self.home, "agy") else {
            return;
        };
        {
            let mut state = self.accounts.agy.lock().expect("agy lock poisoned");
            if state.running || state.checked.is_some_and(|at| at.elapsed() < AGY_EVERY) {
                return;
            }
            state.running = true;
        }
        let read_as = FileAgent::agy(&self.home)
            .read_live()
            .ok()
            .flatten()
            .and_then(|live| live.login?.key);
        let daemon = self.clone();
        std::thread::spawn(move || {
            let result = run_agy_usage(&agy).map_err(|error| format!("{error:#}"));
            let mut state = daemon.accounts.agy.lock().expect("agy lock poisoned");
            // Keep the last good limits through a failed read; the error still shows.
            if result.is_ok() || !matches!(state.last, Some((_, Ok(_)))) {
                state.last = Some((read_as, result));
            }
            state.checked = Some(Instant::now());
            state.running = false;
        });
    }

    /// Whether a login finished, and whether any is still waited for.
    fn finish_logins(&self) -> (bool, bool) {
        let mut pending = self.accounts.pending.lock().expect("pending lock poisoned");
        let before = pending.len();
        pending.retain(|login| {
            let done = match login.agent.file_agent(&self.home) {
                None => uuid::Uuid::parse_str(&login.id)
                    .map_err(anyhow::Error::from)
                    .and_then(|id| {
                        claude_accounts::finish_login(&claude_accounts::root(&self.home), id)
                    })
                    .map(|account| account.map(|a| a.email)),
                Some(files) => files.finish_login(&self.accounts_root(login.agent), &login.id),
            };
            let name = login.agent.name();
            let text = match done {
                Ok(None) if login.started.elapsed() < LOGIN_WAIT => return true,
                Ok(None) => {
                    if login.agent != AgentKind::Claude {
                        let _ = std::fs::remove_dir_all(
                            self.accounts_root(login.agent).join(&login.id),
                        );
                    }
                    format!("{name} 로그인이 30분 안에 끝나지 않아 기다리기를 멈췄어요")
                }
                Ok(Some(label)) => format!("{name} 계정을 더했어요: {label}"),
                Err(error) => format!("{name} 계정을 더하지 못했어요: {error:#}"),
            };
            let _ = self
                .events
                .send(crate::ws::ServerMsg::Notice { text }.text());
            false
        });
        (pending.len() < before, !pending.is_empty())
    }

    pub fn account_act(self: &Arc<Self>, act: AccountAct) -> anyhow::Result<ActDone> {
        let _changing = self
            .accounts
            .changing
            .lock()
            .expect("accounts lock poisoned");
        let notice = |text: String| {
            Ok(ActDone {
                notice: Some(text),
                opened: None,
            })
        };
        match act {
            AccountAct::Switch { agent, id } => {
                let label = match agent.file_agent(&self.home) {
                    None => self.switch_claude(&id)?,
                    Some(files) => {
                        ensure!(
                            agent != AgentKind::Codex
                                || !codex_uses_keychain(&self.home.join(".codex")),
                            "Codex가 로그인을 키체인에 두는 설정이라 바꿀 수 없어요. ~/.codex/config.toml의 cli_auth_credentials_store를 \"file\"로 바꿔 주세요"
                        );
                        files.switch(&self.accounts_root(agent), &id)?
                    }
                };
                let names: BTreeMap<String, String> = read_json(&self.names_path());
                let label = names.get(&store_key(agent, &id)).cloned().unwrap_or(label);
                let name = agent.name();
                if agent == AgentKind::Agy {
                    // The limits on screen were read as the account switched away from.
                    self.accounts.agy.lock().expect("agy lock poisoned").checked = None;
                }
                let after = if agent == AgentKind::Claude {
                    "실행 중인 Claude도 다음 요청부터 이 계정을 써요".to_owned()
                } else {
                    format!("새로 여는 {name}부터 이 계정을 써요")
                };
                notice(format!("{name} 계정을 {label}(으)로 바꿨어요. {after}"))
            }
            AccountAct::SaveCurrent { agent } => {
                let label = match agent.file_agent(&self.home) {
                    None => {
                        claude_accounts::save_current(
                            &claude_accounts::root(&self.home),
                            &self.home.join(".claude"),
                        )?
                        .email
                    }
                    Some(files) => files
                        .save_current(&self.accounts_root(agent))?
                        .label(files.fallback),
                };
                notice(format!("{label}(으)로 저장했어요"))
            }
            AccountAct::Add { agent } => {
                let command = match agent.file_agent(&self.home) {
                    None => {
                        let root = claude_accounts::root(&self.home);
                        let (id, config) =
                            claude_accounts::begin_login(&root, &self.home.join(".claude"))?;
                        self.wait_for_login(agent, id.to_string());
                        claude_accounts::login_command(&config)?
                    }
                    Some(_) => {
                        let id = uuid::Uuid::new_v4().to_string();
                        let dir = self.accounts_root(agent).join(&id);
                        std::fs::create_dir_all(&dir)?;
                        let command = agent.login_command(&dir)?;
                        self.wait_for_login(agent, id);
                        command
                    }
                };
                let title = format!("{} 계정 추가", agent.name());
                let pane =
                    self.background_tab(self.home.to_string_lossy().into_owned(), command, title)?;
                let after = if agent == AgentKind::Agy {
                    "agy에서 로그인한 뒤 agy를 닫으면"
                } else {
                    "로그인을 마치면"
                };
                Ok(ActDone {
                    notice: Some(format!(
                        "{} 로그인 pane을 열었어요. {after} 목록에 나타나요",
                        agent.name()
                    )),
                    opened: Some(pane),
                })
            }
            AccountAct::Remove { agent, id } => {
                match agent.file_agent(&self.home) {
                    None => claude_accounts::remove(
                        &claude_accounts::root(&self.home),
                        uuid::Uuid::parse_str(&id).context("Claude 계정 id가 아니에요")?,
                    )?,
                    Some(files) => files.remove(&self.accounts_root(agent), &id)?,
                }
                let key = store_key(agent, &id);
                for path in [self.names_path(), self.usage_path()] {
                    let mut store: BTreeMap<String, Value> = read_json(&path);
                    if store.remove(&key).is_some() {
                        write_private(&path, &serde_json::to_vec(&store)?)?;
                    }
                }
                notice("계정을 뺐어요".into())
            }
            AccountAct::Rename { agent, id, name } => {
                let name = name.trim();
                ensure!(name.chars().count() <= 40, "이름은 40자까지예요");
                let path = self.names_path();
                let mut names: BTreeMap<String, String> = read_json(&path);
                if name.is_empty() {
                    names.remove(&store_key(agent, &id));
                } else {
                    names.insert(store_key(agent, &id), name.to_owned());
                }
                write_private(&path, &serde_json::to_vec(&names)?)?;
                Ok(ActDone::default())
            }
        }
    }

    fn switch_claude(&self, id: &str) -> anyhow::Result<String> {
        let root = claude_accounts::root(&self.home);
        let personal = self.home.join(".claude");
        let (saved, _) = claude_accounts::list(&root)?;
        let current = claude_accounts::current_email(&personal);
        ensure!(
            current
                .as_deref()
                .is_none_or(|email| saved.iter().any(|a| a.email.eq_ignore_ascii_case(email))),
            "지금 Claude 로그인이 저장된 계정이 아니에요. 먼저 계정으로 저장해 주세요"
        );
        let target = saved
            .iter()
            .find(|a| a.id.to_string() == id)
            .context("저장된 Claude 계정이 없어요")?;
        claude_accounts::switch(&root, target.id, &personal)?;
        Ok(target.email.clone())
    }

    /// Watches for the login every two seconds, so the account shows up as soon as it is done.
    fn wait_for_login(self: &Arc<Self>, agent: AgentKind, id: String) {
        let mut pending = self.accounts.pending.lock().expect("pending lock poisoned");
        let watching = !pending.is_empty();
        pending.push(Pending {
            agent,
            id,
            started: Instant::now(),
        });
        drop(pending);
        if watching {
            return;
        }
        let daemon = self.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let (finished, waiting) = daemon.finish_logins();
                if finished {
                    let agents = daemon.accounts_view();
                    let message = crate::ws::ServerMsg::Accounts {
                        agents,
                        notice: None,
                        opened: None,
                        error: None,
                    };
                    let _ = daemon.events.send(message.text());
                }
                if !waiting {
                    break;
                }
            }
        });
    }
}

/// Codex keeps its login in `auth.json` unless its config names another store.
fn codex_uses_keychain(codex_home: &Path) -> bool {
    std::fs::read_to_string(codex_home.join("config.toml")).is_ok_and(|config| {
        config
            .lines()
            .take_while(|line| !line.trim_start().starts_with('['))
            .any(|line| {
                let line = line.split('#').next().unwrap_or_default();
                let mut parts = line.splitn(2, '=');
                parts.next().map(str::trim) == Some("cli_auth_credentials_store")
                    && parts.next().map(|v| v.trim().trim_matches('"')) != Some("file")
            })
    })
}

/// `email · plan`, or just the plan when the label already is the email.
fn detail<const N: usize>(parts: [Option<&str>; N], named: bool) -> Option<String> {
    let parts: Vec<&str> = parts
        .into_iter()
        .enumerate()
        .filter_map(|(index, part)| part.filter(|_| named || index > 0))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn run_agy_usage(agy: &Path) -> anyhow::Result<Seen> {
    let mut child = Command::new(agy)
        .args([
            "-p",
            "/usage",
            "--output-format",
            "json",
            "--print-timeout",
            "40s",
        ])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("cannot start agy")?;
    let mut stdout = child.stdout.take().context("agy stdout unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut text).map(|_| text)
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > AGY_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "agy /usage가 {}초 안에 끝나지 않았어요",
                AGY_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    ensure!(status.success(), "agy /usage exited with {status}");
    let text = reader
        .join()
        .map_err(|_| anyhow::anyhow!("agy stdout reader panicked"))??;
    parse_agy(&text, unix_now())
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
