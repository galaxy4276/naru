//! A terminal pane: its PTY, the program in it, and the recent output a screen replays when it
//! attaches.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::broadcast;

use crate::grid::{Draft, Screen};
use crate::osc::{Osc, OscCollector};

/// Output kept for a screen that attaches later.
const REPLAY_LIMIT: usize = 2 * 1024 * 1024;
/// Input messages waiting for a program that is not reading.
const INPUT_QUEUE: usize = 1024;

pub enum PaneEvent {
    Osc(Osc),
    Exited,
}

pub struct Pane {
    input: SyncSender<Vec<u8>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    output: Mutex<Output>,
    screen: Mutex<Screen>,
}

struct Output {
    replay: VecDeque<u8>,
    live: broadcast::Sender<Arc<[u8]>>,
}

impl Pane {
    pub fn spawn(
        command: CommandBuilder,
        size: PtySize,
        on_event: impl Fn(PaneEvent) + Send + 'static,
    ) -> anyhow::Result<Arc<Pane>> {
        let pair = native_pty_system().openpty(size)?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let (input, queued) = sync_channel::<Vec<u8>>(INPUT_QUEUE);
        // One writer thread keeps input in order and never blocks a caller on a full PTY.
        std::thread::spawn(move || {
            for data in queued {
                if writer
                    .write_all(&data)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    return;
                }
            }
        });
        let pane = Arc::new(Pane {
            input,
            master: Mutex::new(pair.master),
            child: Mutex::new(child),
            output: Mutex::new(Output {
                replay: VecDeque::new(),
                live: broadcast::channel(1024).0,
            }),
            screen: Mutex::new(Screen::new(size.cols, size.rows)),
        });
        let weak = Arc::downgrade(&pane);
        std::thread::spawn(move || {
            let mut osc = OscCollector::default();
            let mut buf = vec![0; 64 * 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let bytes = &buf[..n];
                for found in osc.feed(bytes) {
                    on_event(PaneEvent::Osc(found));
                }
                let Some(pane) = weak.upgrade() else { return };
                pane.screen.lock().expect("screen lock").feed(bytes);
                pane.output.lock().expect("output lock").push(bytes);
            }
            if let Some(pane) = weak.upgrade() {
                // Reaped so the shell does not linger as a zombie.
                let _ = pane.child.lock().expect("child lock").wait();
            }
            on_event(PaneEvent::Exited);
        });
        Ok(pane)
    }

    /// The output so far and a receiver for every later chunk, with nothing lost or repeated
    /// between them.
    pub fn attach(&self) -> (Vec<u8>, broadcast::Receiver<Arc<[u8]>>) {
        let output = self.output.lock().expect("output lock");
        (
            output.replay.iter().copied().collect(),
            output.live.subscribe(),
        )
    }

    pub fn write(&self, data: Vec<u8>) -> anyhow::Result<()> {
        self.input.try_send(data).map_err(|error| match error {
            TrySendError::Full(_) => anyhow!("the program is not reading its input"),
            TrySendError::Disconnected(_) => anyhow!("the pane has ended"),
        })
    }

    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.lock().expect("master lock").resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.screen.lock().expect("screen lock").resize(cols, rows);
        Ok(())
    }

    /// The last `lines` lines the pane shows.
    pub fn read(&self, lines: usize) -> String {
        self.screen.lock().expect("screen lock").last_lines(lines)
    }

    pub fn draft(&self) -> Draft {
        self.screen.lock().expect("screen lock").draft()
    }

    /// Text as the program wants it pasted: bracketed when it asked for that.
    pub fn paste(&self, text: &str) -> anyhow::Result<()> {
        let data = if self.screen.lock().expect("screen lock").bracketed_paste() {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text.to_owned()
        };
        self.write(data.into_bytes())
    }

    pub fn kill(&self) {
        let _ = self.child.lock().expect("child lock").kill();
    }

    /// The shell's process id.
    pub fn pid(&self) -> Option<u32> {
        self.child.lock().expect("child lock").process_id()
    }
}

impl Output {
    fn push(&mut self, bytes: &[u8]) {
        self.replay.extend(bytes);
        let excess = self.replay.len().saturating_sub(REPLAY_LIMIT);
        // ponytail: the cut can land inside an escape sequence; the screen redraws on the
        // program's next full repaint. Replay a parsed screen grid if that shows.
        self.replay.drain(..excess);
        let _ = self.live.send(bytes.into());
    }
}

/// Environment an agent session leaks into anything it starts. A pane that inherits it runs
/// `claude` as a child session that keeps no transcript, so `--resume` later finds nothing.
fn is_agent_session_env(key: &str) -> bool {
    matches!(key, "CLAUDECODE" | "CLAUDE_PID" | "CLAUDE_EFFORT")
        || ["CLAUDE_CODE_", "CLAUDE_PLUGIN_", "CODEX_COMPANION_"]
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

/// The user's login shell, with the environment the agents' Warp plugins look for.
pub fn shell_command(cwd: &Path, zsh_dir: &Path) -> CommandBuilder {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut command = CommandBuilder::new(&shell);
    command.arg("-l");
    command.cwd(cwd);
    for (key, _) in std::env::vars_os() {
        if is_agent_session_env(&key.to_string_lossy()) {
            command.env_remove(&key);
        }
    }
    for (key, value) in [
        ("TERM", "xterm-256color"),
        ("TERM_PROGRAM", "WarpTerminal"),
        ("COLORTERM", "truecolor"),
        ("WARP_CLIENT_VERSION", "local"),
        ("WARP_CLI_AGENT_PROTOCOL_VERSION", "1"),
        ("WARP_IS_LOCAL_SHELL_SESSION", "1"),
        ("NARU_PANE", "1"),
    ] {
        command.env(key, value);
    }
    if Path::new(&shell)
        .file_name()
        .is_some_and(|name| name == "zsh")
    {
        if let Some(user) = std::env::var_os("ZDOTDIR").or_else(|| std::env::var_os("HOME")) {
            command.env("NARU_USER_ZDOTDIR", user);
        }
        command.env("ZDOTDIR", zsh_dir);
    }
    command
}

#[cfg(test)]
#[path = "pane_tests.rs"]
mod tests;
