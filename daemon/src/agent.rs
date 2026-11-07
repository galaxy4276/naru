//! CLI agents Naru resumes, and the command lines that bring their conversations back. Ported
//! from the Warp fork's `app/src/terminal/cli_agent.rs`.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Named as the Warp fork stores them, so its saved panes read back unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Agent {
    Claude,
    Gemini,
    Codex,
    Amp,
    Droid,
    OpenCode,
    Copilot,
    Pi,
    OhMyPi,
    Auggie,
    CursorCli,
    Kiro,
    Goose,
    Hermes,
    Vibe,
    Antigravity,
    Grok,
}

const ALL: [Agent; 17] = [
    Agent::Claude,
    Agent::Gemini,
    Agent::Codex,
    Agent::Amp,
    Agent::Droid,
    Agent::OpenCode,
    Agent::Copilot,
    Agent::Pi,
    Agent::OhMyPi,
    Agent::Auggie,
    Agent::CursorCli,
    Agent::Kiro,
    Agent::Goose,
    Agent::Hermes,
    Agent::Vibe,
    Agent::Antigravity,
    Agent::Grok,
];

impl Agent {
    fn command_prefixes(self) -> &'static [&'static str] {
        match self {
            Agent::Claude => &["claude", "cl"],
            Agent::Gemini => &["gemini"],
            Agent::Codex => &["codex"],
            Agent::Amp => &["amp"],
            Agent::Droid => &["droid"],
            Agent::OpenCode => &["opencode", "oc"],
            Agent::Copilot => &["copilot"],
            Agent::Pi => &["pi"],
            Agent::OhMyPi => &["omp"],
            Agent::Auggie => &["auggie"],
            Agent::CursorCli => &["agent"],
            Agent::Kiro => &["kiro-cli"],
            Agent::Goose => &["goose"],
            Agent::Hermes => &["hermes"],
            Agent::Vibe => &["vibe", "vibe-acp"],
            Agent::Antigravity => &["agy"],
            Agent::Grok => &["grok"],
        }
    }

    pub fn binary(self) -> &'static str {
        self.command_prefixes()[0]
    }

    /// The agent a plugin names in its events (`"agent":"claude"`).
    pub fn from_event_name(name: &str) -> Option<Agent> {
        ALL.into_iter()
            .find(|agent| agent.command_prefixes().contains(&name))
    }

    /// The agent a typed command line starts, and its first word when that word can be put back
    /// on a command line (a shell function like `cl` keeps the flags it adds).
    pub fn detect(command: &str) -> Option<(Agent, Option<String>)> {
        let words = shell_words::split(command).ok()?;
        let first = words.into_iter().find(|word| !is_env_assignment(word))?;
        let basename = first.rsplit('/').next().unwrap_or(&first);
        let agent = ALL
            .into_iter()
            .find(|agent| agent.command_prefixes().contains(&basename))?;
        Some((agent, is_plain_word(&first).then_some(first)))
    }

    /// Command that resumes `session_id`. `None` when this agent has no ID-based resume, or
    /// when `session_id` isn't a plain ID: it arrives over OSC from the PTY, so anything that
    /// could be shell syntax is rejected rather than run.
    pub fn resume_command(self, launcher: Option<&str>, session_id: &str) -> Option<String> {
        if !is_plain_word(session_id)
            || launcher.is_some_and(|value| !is_plain_word(value))
            || (self == Agent::Gemini && uuid::Uuid::parse_str(session_id).is_err())
        {
            return None;
        }
        let args = match self {
            Agent::Claude | Agent::Grok | Agent::CursorCli | Agent::Gemini => "--resume",
            Agent::Codex => "resume",
            Agent::OpenCode => "--session",
            Agent::Kiro => "chat --resume-id",
            Agent::Amp
            | Agent::Droid
            | Agent::Copilot
            | Agent::Pi
            | Agent::OhMyPi
            | Agent::Auggie
            | Agent::Goose
            | Agent::Hermes
            | Agent::Vibe
            | Agent::Antigravity => return None,
        };
        Some(format!(
            "{} {args} {session_id}",
            self.launcher_or_binary(launcher)
        ))
    }

    /// Continues the agent's latest conversation in the current folder. Only typed into the
    /// prompt, never run: it is a guess when the agent never reported its session ID.
    // ponytail: latest-session fallback can collide across panes in one directory.
    pub fn continue_command(self, launcher: Option<&str>) -> Option<String> {
        if launcher.is_some_and(|value| !is_plain_word(value)) {
            return None;
        }
        let args = match self {
            Agent::Claude | Agent::Grok | Agent::CursorCli | Agent::OpenCode => "--continue",
            Agent::Codex => "resume --last",
            Agent::Gemini => "--resume latest",
            Agent::Kiro => "chat --resume",
            Agent::Amp
            | Agent::Droid
            | Agent::Copilot
            | Agent::Pi
            | Agent::OhMyPi
            | Agent::Auggie
            | Agent::Goose
            | Agent::Hermes
            | Agent::Vibe
            | Agent::Antigravity => return None,
        };
        Some(format!("{} {args}", self.launcher_or_binary(launcher)))
    }

    /// The agent's own conversation picker, typed when there is nothing better to offer.
    pub fn session_picker_command(self, launcher: Option<&str>) -> Option<String> {
        if launcher.is_some_and(|value| !is_plain_word(value)) {
            return None;
        }
        let args = match self {
            Agent::Claude => "--resume",
            Agent::Codex => "resume",
            _ => return None,
        };
        Some(format!("{} {args}", self.launcher_or_binary(launcher)))
    }

    /// `resume_command`, plus the sandbox and approval flags a Codex launch was started with so
    /// the resumed agent keeps the same permissions.
    pub fn resume_command_for_launch(
        self,
        launcher: Option<&str>,
        session_id: &str,
        launch_command: Option<&str>,
    ) -> Option<String> {
        let mut command = self.resume_command(launcher, session_id)?;
        if self != Agent::Codex {
            return Some(command);
        }
        // Without the launch line the permissions it ran with are unknown, so nothing runs.
        let words = shell_words::split(launch_command?).ok()?;
        if words.first()?.as_str() != self.launcher_or_binary(launcher) {
            return None;
        }
        let mut args = words[1..].iter().map(String::as_str).peekable();
        while let Some(arg) = args.next() {
            match arg {
                "resume" => {
                    if args.peek().is_some_and(|value| is_plain_word(value)) {
                        args.next();
                    }
                }
                "--last" => {}
                // Runs hooks inside the pane, where they can reach its terminal.
                "--no-daemon" => command.push_str(" --no-daemon"),
                "--sandbox" | "-s" | "--ask-for-approval" | "-a" => {
                    let value = args.next()?;
                    let allowed = if matches!(arg, "--sandbox" | "-s") {
                        matches!(
                            value,
                            "read-only" | "workspace-write" | "danger-full-access"
                        )
                    } else {
                        matches!(value, "untrusted" | "on-failure" | "on-request" | "never")
                    };
                    if !allowed {
                        return None;
                    }
                    command.push_str(&format!(" {arg} {value}"));
                }
                _ => return None,
            }
        }
        Some(command)
    }

    // ponytail: assumes the launcher forwards its arguments to the agent ("$@"); a launcher
    // like `env` or `npx` would need the full command line, not just the first word.
    fn launcher_or_binary(self, launcher: Option<&str>) -> &str {
        launcher
            .filter(|launcher| is_plain_word(launcher))
            .unwrap_or(self.binary())
    }
}

fn is_env_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Whether `value` can be put on a command line as-is: no spaces, quotes, or shell syntax.
pub fn is_plain_word(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// Bytes of a transcript read while looking for its first user line.
const CLEAR_SCAN_LIMIT: u64 = 2 * 1024 * 1024;

/// Whether Claude's conversation `id` began with `/clear`, i.e. the pane's conversation was
/// cleared into it. Claude sends no event when `/clear` swaps the session ID.
pub fn claude_cleared_into(claude_projects: &Path, id: &str) -> bool {
    if uuid::Uuid::parse_str(id).is_err() {
        return false;
    }
    let Ok(projects) = std::fs::read_dir(claude_projects) else {
        return false;
    };
    projects
        .flatten()
        .map(|project| project.path().join(format!("{id}.jsonl")))
        .find(|path| path.is_file())
        .is_some_and(|path| starts_with_clear(&path))
}

fn starts_with_clear(transcript: &Path) -> bool {
    let Ok(file) = std::fs::File::open(transcript) else {
        return false;
    };
    for line in BufReader::new(file.take(CLEAR_SCAN_LIMIT)).lines() {
        let Ok(line) = line else { return false };
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        // Claude Code writes the `<local-command-caveat>` note as a meta user line before `/clear`.
        if entry.get("type").and_then(|kind| kind.as_str()) != Some("user")
            || entry.get("isMeta").and_then(|meta| meta.as_bool()) == Some(true)
        {
            continue;
        }
        return entry
            .pointer("/message/content")
            .and_then(|content| content.as_str())
            .is_some_and(|content| {
                content
                    .trim_start()
                    .starts_with("<command-name>/clear</command-name>")
            });
    }
    false
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
