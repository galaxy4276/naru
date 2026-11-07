//! Automatic pane titles: haiku names each agent pane and says what it is doing, once its
//! conversation has gone quiet. A title the user typed always wins over these.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::agent::{Agent, is_plain_word};
use crate::core::Daemon;
use crate::state::PaneId;
use crate::{suggestions, transcript};

pub const MODEL: &str = "claude-haiku-5-5";
/// A conversation is titled once it has been quiet this long, so a burst of turns costs one call.
const QUIET: Duration = Duration::from_secs(30);
const WORKERS: usize = 3;
const CALL_TIMEOUT: Duration = Duration::from_secs(90);
const CANDIDATES_TIMEOUT: Duration = Duration::from_secs(60);
const TAIL_BYTES: u64 = 2 * 1024 * 1024;
/// User requests summarized, each with the agent's last answer to it.
const MAX_TURNS: usize = 10;
const MAX_TURN_CHARS: usize = 400;
const MAX_TITLE_CHARS: usize = 30;
const MAX_SUMMARY_CHARS: usize = 60;
const CANDIDATES: usize = 3;
const SCREEN_LINES: usize = 60;

#[derive(Default)]
pub struct Titles {
    /// Transcript modification time each session was last titled at.
    titled_at: Mutex<HashMap<String, u64>>,
    running: AtomicBool,
}

struct Target {
    pane: PaneId,
    agent: Agent,
    session_id: String,
    path: PathBuf,
    modified: u64,
    /// The repository a Claude pane works in, whose 제안 the turn's end is recorded for.
    repo: Option<PathBuf>,
}

/// The user's `claude`, where its installer puts it.
pub fn claude_path(home: &Path) -> Option<PathBuf> {
    Some(home.join(".local/bin/claude")).filter(|path| path.is_file())
}

/// Runs one `claude -p` call outside the user's hooks, plugins and CLAUDE.md and returns its
/// output.
pub fn ask_claude(
    claude: &Path,
    model: &str,
    prompt: &str,
    timeout: Duration,
) -> anyhow::Result<String> {
    ask_claude_with(claude, model, prompt, timeout, &[])
}

/// [`ask_claude`] with more command-line arguments, such as the tools the call may use.
pub fn ask_claude_with(
    claude: &Path,
    model: &str,
    prompt: &str,
    timeout: Duration,
    extra: &[&std::ffi::OsStr],
) -> anyhow::Result<String> {
    let mut command = Command::new(claude);
    command
        .args([
            "-p",
            "--model",
            model,
            "--no-session-persistence",
            "--disable-slash-commands",
            // The user's hooks, plugins, MCP servers and CLAUDE.md would otherwise run on every
            // call; safe mode skips them while keeping the subscription login.
            "--safe-mode",
        ])
        .args(extra)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("ANTHROPIC_")
            || name.starts_with("CLAUDE_CODE_")
            || name == "CLAUDECODE"
        {
            command.env_remove(&key);
        }
    }
    let mut child = command.spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("claude stdin unavailable"))?
        .write_all(prompt.as_bytes())?;
    // Read while it runs: an answer past the pipe's buffer would otherwise block the child.
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("claude stdout unavailable"))?;
    let reader = std::thread::spawn(move || {
        let mut output = String::new();
        stdout.read_to_string(&mut output).map(|_| output)
    });
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("claude stderr unavailable"))?;
    let errors = std::thread::spawn(move || {
        let mut output = String::new();
        let _ = stderr.read_to_string(&mut output);
        output
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("claude timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    if !status.success() {
        let errors = errors.join().unwrap_or_default();
        let last = errors
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        anyhow::bail!(
            "claude exited with {status}: {}",
            last.chars().take(300).collect::<String>()
        );
    }
    reader
        .join()
        .map_err(|_| anyhow::anyhow!("claude stdout reader panicked"))?
        .map_err(Into::into)
}

fn clip(text: &str, max: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// The last requests of a transcript, oldest first, each followed by the agent's last answer.
/// Injected reminders and slash-command wrappers are not what the user was working on.
fn digest(agent: Agent, jsonl: &str) -> Option<String> {
    let turns: Vec<_> = transcript::turns(agent, jsonl)
        .into_iter()
        .filter(|turn| !turn.request.starts_with('<') && !turn.request.starts_with('['))
        .collect();
    let start = turns.len().saturating_sub(MAX_TURNS);
    let digest = turns[start..]
        .iter()
        .map(|turn| match &turn.reply {
            Some(reply) => format!(
                "User: {}\n\nAssistant: {}",
                clip(&turn.request, MAX_TURN_CHARS),
                clip(reply, MAX_TURN_CHARS)
            ),
            None => format!("User: {}", clip(&turn.request, MAX_TURN_CHARS)),
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!digest.is_empty()).then_some(digest)
}

fn unquote(line: &str) -> &str {
    line.trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '“' | '”' | '「' | '」'))
        .trim()
}

/// The model's first line, as a title.
fn clean_title(output: &str) -> Option<String> {
    let line = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let title: String = unquote(line.trim_start_matches("제목:"))
        .trim_end_matches(['.', '。'])
        .trim()
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    (!title.is_empty()).then_some(title)
}

/// The model's second line, as a summary. A missing line yields none, so the pane keeps its
/// previous one rather than showing a made-up one.
fn clean_summary(output: &str) -> Option<String> {
    let line = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .nth(1)?;
    let summary = unquote(line.trim_start_matches("요약:"));
    match summary.chars().count() {
        0 => None,
        count if count <= MAX_SUMMARY_CHARS => Some(summary.to_owned()),
        _ => Some(format!(
            "{}…",
            summary
                .chars()
                .take(MAX_SUMMARY_CHARS - 1)
                .collect::<String>()
        )),
    }
}

/// Up to three distinct titles from the model's answer, one per line, without list markers.
fn parse_candidates(output: &str) -> Vec<String> {
    let mut titles: Vec<String> = Vec::new();
    for line in output.lines() {
        let line = line.trim().trim_start_matches(['-', '*', '•']).trim_start();
        let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let line = match line[digits..].strip_prefix(['.', ')']) {
            Some(rest) if digits > 0 => rest,
            _ => line,
        };
        let title = clip(unquote(line), MAX_TITLE_CHARS);
        if !title.is_empty() && !titles.contains(&title) {
            titles.push(title);
        }
        if titles.len() == CANDIDATES {
            break;
        }
    }
    titles
}

/// The title and summary request; with the last turn's tool use, it also asks where the turn
/// lost time and what it left unsolved, for 제안.
fn title_prompt(digest: &str, tool_use: Option<&str>) -> String {
    let Some(tool_use) = tool_use else {
        return format!(
            "아래는 터미널 에이전트와 나눈 작업 대화의 최근 일부다. \
             첫 줄: 이 작업을 가장 잘 나타내는 한국어 제목({MAX_TITLE_CHARS}자 이내). \
             둘째 줄: 지금 하던 일과 막힌 곳을 한 문장으로({MAX_SUMMARY_CHARS}자 이내). \
             대화에 없는 내용은 쓰지 않는다. 따옴표, 마침표, 번호, 설명은 쓰지 않는다.\n\n{digest}"
        );
    };
    format!(
        "아래는 터미널 에이전트와 나눈 작업 대화의 최근 일부다. \
         첫 줄: 이 작업을 가장 잘 나타내는 한국어 제목({MAX_TITLE_CHARS}자 이내). \
         둘째 줄: 지금 하던 일과 막힌 곳을 한 문장으로({MAX_SUMMARY_CHARS}자 이내). \
         그다음 줄부터, 마지막 턴에서 에이전트가 시간을 잃었지만 문서·규칙·설정을 바꿨다면 \
         막을 수 있었던 곳만 최대 3줄 `개선: <종류> | <한국어 200자 이내>` 형식으로 쓴다. \
         종류는 no_doc(필요한 문서가 없었음), searched_again(같은 것을 다시 찾음), \
         wrong_guess(잘못 짐작함), tool_friction(도구·설정이 불편함) 중 하나다. \
         그런 곳이 없으면 개선 줄은 쓰지 않는다. 세션이 풀지 못하고 남긴 문제가 있으면 한 줄 \
         `남은 문제: <한국어 120자 이내>`로 쓰고, 없으면 쓰지 않는다. <tool-use>는 Naru가 센 \
         마지막 턴의 도구 사용이다. 대화에 없는 내용은 쓰지 않는다. 제목과 요약에는 따옴표, \
         마침표, 번호, 설명을 쓰지 않는다.\n\n{digest}\n\n<tool-use>\n{tool_use}\n</tool-use>"
    )
}

fn modified_ms(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Daemon {
    /// Titles every agent pane whose conversation changed and then went quiet. One pass at a
    /// time; a pass already running makes this one a no-op.
    pub fn title_pass(&self) {
        if self.titles.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let targets = self.title_targets();
        if !targets.is_empty()
            && let Some(claude) = claude_path(&self.home)
        {
            let queue = Mutex::new(VecDeque::from(targets));
            std::thread::scope(|scope| {
                for _ in 0..WORKERS {
                    scope.spawn(|| {
                        while let Some(target) = queue.lock().expect("title queue").pop_front() {
                            self.title(&claude, target);
                        }
                    });
                }
            });
        }
        self.titles.running.store(false, Ordering::SeqCst);
    }

    fn title_targets(&self) -> Vec<Target> {
        let quiet_since = now_ms().saturating_sub(QUIET.as_millis() as u64);
        // Read under the lock, then look at the files without holding it.
        let sessions: Vec<_> = {
            let core = self.core();
            core.workspace
                .panes
                .iter()
                .filter(|(_, record)| record.node.is_none())
                .filter_map(|(pane, record)| {
                    let session = record.session.as_ref()?;
                    let session_id = session.session_id.clone().filter(|id| is_plain_word(id))?;
                    transcript::readable(session.agent).then(|| {
                        let reported = session.transcript_path.clone();
                        let cwd = (session.agent == Agent::Claude)
                            .then(|| record.cwd.clone())
                            .flatten();
                        (
                            *pane,
                            session.agent,
                            session_id,
                            reported,
                            record.summary.is_some(),
                            cwd,
                        )
                    })
                })
                .collect()
        };
        let mut titled = self.titles.titled_at.lock().expect("titles lock");
        sessions
            .into_iter()
            .filter_map(|(pane, agent, session_id, reported, has_summary, cwd)| {
                let path =
                    transcript::locate(agent, reported.as_deref(), Some(&session_id), &self.home)?;
                let modified = modified_ms(&path)?;
                let last = titled.get(&session_id).copied();
                // A restored pane shows its saved summary until there is new conversation.
                if last.is_none() && has_summary {
                    titled.insert(session_id, modified);
                    return None;
                }
                (last != Some(modified) && modified <= quiet_since).then_some(Target {
                    pane,
                    agent,
                    session_id,
                    path,
                    modified,
                    repo: cwd.and_then(|cwd| suggestions::repo_root(Path::new(&cwd))),
                })
            })
            .collect()
    }

    fn title(&self, claude: &Path, target: Target) {
        let mut turn = None;
        let output = transcript::read_tail(&target.path, TAIL_BYTES)
            .map_err(anyhow::Error::from)
            .and_then(|(text, _)| {
                turn = target
                    .repo
                    .as_ref()
                    .and_then(|_| suggestions::last_turn(&text));
                let tool_use = turn
                    .as_ref()
                    .map(|turn: &suggestions::LastTurn| turn.summary.as_str());
                digest(target.agent, &text)
                    .map(|digest| {
                        ask_claude(
                            claude,
                            MODEL,
                            &title_prompt(&digest, tool_use),
                            CALL_TIMEOUT,
                        )
                    })
                    .transpose()
            });
        // Marked titled even when the call failed, so a broken login costs one call per change.
        self.titles
            .titled_at
            .lock()
            .expect("titles lock")
            .insert(target.session_id.clone(), target.modified);
        let output = match output {
            Ok(Some(output)) => output,
            Ok(None) => return,
            Err(error) => {
                eprintln!("auto title for {}: {error:#}", target.session_id);
                return;
            }
        };
        if let (Some(root), Some(turn)) = (&target.repo, &turn) {
            let improvements = suggestions::parse_improvements(&output);
            let missed = suggestions::parse_missed(&output);
            suggestions::record_turn(
                root,
                &target.session_id,
                &suggestions::TurnFacts {
                    turn: &turn.turn,
                    improvements: &improvements,
                    failed: &turn.failed,
                    missed: missed.as_deref(),
                },
            );
        }
        let (title, summary) = (clean_title(&output), clean_summary(&output));
        if title.is_none() && summary.is_none() {
            return;
        }
        let mut core = self.core();
        let Some(record) = core.workspace.panes.get_mut(&target.pane) else {
            return;
        };
        // The pane may have moved on to another conversation while the call ran.
        if record
            .session
            .as_ref()
            .and_then(|s| s.session_id.as_deref())
            != Some(&target.session_id)
        {
            return;
        }
        record.auto_title = title.or(record.auto_title.take());
        record.summary = summary.or(record.summary.take());
        self.changed(&core);
    }

    /// Three title ideas for the title sheet, from the pane's conversation or, for a plain
    /// shell, what it shows.
    pub fn title_candidates(&self, pane: PaneId) -> anyhow::Result<Vec<String>> {
        let claude = claude_path(&self.home)
            .ok_or_else(|| anyhow::anyhow!("~/.local/bin/claude가 없어요"))?;
        let (session, live) = {
            let core = self.core();
            let record = core
                .workspace
                .panes
                .get(&pane)
                .ok_or_else(|| anyhow::anyhow!("no pane {pane}"))?;
            (record.session.clone(), core.live.get(&pane).cloned())
        };
        let conversation = session.and_then(|session| {
            let id = session
                .session_id
                .as_deref()
                .filter(|id| is_plain_word(id))?;
            let path = transcript::locate(
                session.agent,
                session.transcript_path.as_deref(),
                Some(id),
                &self.home,
            )?;
            let text = transcript::read_tail(&path, TAIL_BYTES).ok()?.0;
            digest(session.agent, &text)
        });
        let context = match conversation {
            Some(digest) => digest,
            None => live
                .map(|live| live.read(SCREEN_LINES))
                .ok_or_else(|| anyhow::anyhow!("no pane {pane}"))?,
        };
        anyhow::ensure!(
            !context.trim().is_empty(),
            "요약할 대화나 명령이 아직 없어요"
        );
        let prompt = format!(
            "아래는 터미널 pane의 최근 요청과 결과다. 이 작업을 나타내는 한국어 제목 후보 \
             {CANDIDATES}개를 한 줄에 하나씩 쓴다. 각 {MAX_TITLE_CHARS}자 이내, 서로 다른 관점으로. \
             따옴표, 번호, 마침표, 설명은 쓰지 않는다.\n\n<pane>\n{context}\n</pane>"
        );
        let titles = parse_candidates(&ask_claude(&claude, MODEL, &prompt, CANDIDATES_TIMEOUT)?);
        anyhow::ensure!(!titles.is_empty(), "AI 제목 후보를 받지 못했어요");
        Ok(titles)
    }
}

#[cfg(test)]
#[path = "titles_tests.rs"]
mod tests;
