//! The `naru` helper's channel into the daemon. The helper prints each request as an OSC 777
//! `naru://agent` notification carrying the token the daemon wrote to `~/.naru/token`, and waits
//! for the reply in `~/.naru/replies/<nonce>.json`. Any program in a pane can print an OSC, but
//! only one that can read the user's files can start a program or type into an agent. A node
//! reaches the same channel through an ssh forward of `~/.naru/naru.sock`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::Daemon;
use crate::state::PaneId;
use crate::team::Kind;

pub const AGENT: &str = "naru://agent";
pub const VIEW_OPEN: &str = "naru://view-open";
/// How long the socket waits for the reply file of a node's request.
const REPLY_WAIT: Duration = Duration::from_secs(10);

#[derive(Deserialize, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Op {
    Spawn {
        command: String,
        /// On a node, `None` is the node user's home.
        cwd: Option<String>,
        #[serde(default)]
        parent: bool,
        title: Option<String>,
        /// The ssh host to run on; `None` runs on this machine.
        #[serde(default)]
        machine: Option<String>,
        /// The step of the caller's plan the child takes.
        #[serde(default)]
        plan_node: Option<String>,
    },
    Tell {
        to: String,
        kind: Kind,
        text: String,
        /// Type it in while the recipient works instead of waiting for it to stop.
        #[serde(default)]
        now: bool,
    },
    /// Answers a letter (l7) to whoever wrote it.
    Reply {
        letter: String,
        kind: Option<Kind>,
        text: String,
        #[serde(default)]
        now: bool,
    },
    /// One ask of `naru agent wait`.
    Wait {
        to: String,
    },
    List,
    Projects,
    Show {
        to: String,
    },
    Ps,
    Sleep {
        to: String,
    },
    Wake {
        to: String,
    },
    PaneOpen {
        command: String,
    },
    NodeAttach,
    Read {
        to: String,
        lines: Option<usize>,
    },
    Send {
        to: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        keys: Vec<String>,
        #[serde(default)]
        enter: bool,
    },
    LearnMark {
        title: String,
        #[serde(default)]
        hard: String,
        #[serde(default)]
        excerpt: String,
    },
    LearnDone {
        id: u32,
        note: String,
    },
    PlanSet {
        title: String,
        steps: Vec<crate::plans::NewStep>,
        #[serde(default)]
        ask: bool,
    },
    PlanShow {
        #[serde(default)]
        to: Option<String>,
    },
    PlanApprove,
    PlanDrop,
    /// A memory card the user's current words contradict.
    MemoryStale {
        card: u32,
        reason: String,
    },
}

#[derive(Deserialize, Debug)]
struct Request {
    token: String,
    /// Names the reply file; checked by [`is_nonce`] before it touches the file system.
    reply: String,
    #[serde(flatten)]
    op: Op,
}

/// Fails while another Naru (the Warp fork, say) answers on `dir/naru.sock`: it owns the token the
/// naru CLI reads, and taking both over would cut its agents off mid-task. A socket file nobody
/// answers on is left over from a Naru that has quit.
pub fn refuse_if_served(dir: &Path) -> anyhow::Result<()> {
    let path = dir.join("naru.sock");
    if UnixStream::connect(&path).is_ok() {
        anyhow::bail!(
            "another Naru is serving {}; quit it before opening this one",
            path.display()
        );
    }
    Ok(())
}

/// Writes a fresh token to `dir/token` (0600) and makes the replies folder.
pub fn publish(dir: &Path) -> anyhow::Result<String> {
    std::fs::create_dir_all(dir.join("replies"))?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(dir.join("token"))?
        .write_all(token.as_bytes())?;
    Ok(token)
}

/// A reply file name the helper chose: short and plain, so it cannot point outside `replies/`.
pub fn is_nonce(value: &str) -> bool {
    (8..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn write_reply(dir: &Path, nonce: &str, result: Result<Value, String>) {
    let body = match result {
        Ok(Value::Object(mut fields)) => {
            fields.insert("ok".into(), true.into());
            Value::Object(fields)
        }
        Ok(other) => json!({ "ok": true, "value": other }),
        Err(error) => json!({ "ok": false, "error": error }),
    };
    let dir = dir.join("replies");
    let partial = dir.join(format!("{nonce}.part"));
    let result = std::fs::write(&partial, body.to_string())
        .and_then(|()| std::fs::rename(&partial, dir.join(format!("{nonce}.json"))));
    if let Err(error) = result {
        eprintln!("cannot write naru reply {nonce}: {error}");
    }
}

/// Serves `dir/naru.sock`: each client gets the token, sends its nonce, and receives the reply
/// file's body once the request it printed through its ssh tty has been answered.
pub fn serve_socket(dir: &Path, token: String) -> anyhow::Result<()> {
    let path = dir.join("naru.sock");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let replies = dir.join("replies");
    let token: Arc<str> = token.into();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (replies, token) = (replies.clone(), token.clone());
            std::thread::spawn(move || {
                if let Err(error) = answer(stream, &replies, &token) {
                    eprintln!("naru socket client: {error}");
                }
            });
        }
    });
    Ok(())
}

fn answer(mut stream: UnixStream, replies: &Path, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(REPLY_WAIT * 3))?;
    writeln!(stream, "{token}")?;
    let mut nonce = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut nonce)?;
    let nonce = nonce.trim();
    if !is_nonce(nonce) {
        return Ok(());
    }
    let path = replies.join(format!("{nonce}.json"));
    let deadline = Instant::now() + REPLY_WAIT;
    while Instant::now() < deadline {
        if let Ok(body) = std::fs::read(&path) {
            let _ = std::fs::remove_file(&path);
            return stream.write_all(&body);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// The canonical path of the regular file a `naru open` names, and whether it was sent as a
/// review report. Nothing it names is run.
pub fn parse_view(body: &str) -> Option<(PathBuf, bool)> {
    #[derive(Deserialize)]
    struct View {
        path: String,
        #[serde(default)]
        kind: Option<String>,
    }
    let view: View = serde_json::from_str(body).ok()?;
    let path = PathBuf::from(view.path);
    if !path.is_absolute() {
        return None;
    }
    let path = path.canonicalize().ok()?;
    path.is_file()
        .then_some((path, view.kind.as_deref() == Some("proof")))
}

/// A review report the proof panel can lay out: it has a `## Findings` section.
pub fn has_findings(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .is_ok_and(|text| text.lines().any(|line| line.trim_end() == "## Findings"))
}

impl Daemon {
    pub fn naru_request(self: &Arc<Self>, from: PaneId, body: &str) {
        if crate::nodes::is_prompt_signal(body) {
            self.node_prompt_returned(from);
            return;
        }
        let request: Request = match serde_json::from_str(body) {
            Ok(request) => request,
            Err(error) => {
                eprintln!("ignoring a malformed naru request: {error}");
                return;
            }
        };
        if request.token != self.naru_token || !is_nonce(&request.reply) {
            eprintln!("refused a naru request without the current token");
            return;
        }
        let result = match request.op {
            Op::Spawn {
                command,
                cwd,
                parent,
                title,
                machine,
                plan_node,
            } => self.op_spawn(
                from,
                crate::team::Spawn {
                    command,
                    cwd,
                    parent,
                    title,
                    machine,
                    plan_node,
                },
            ),
            Op::Tell {
                to,
                kind,
                text,
                now,
            } => self.op_tell(from, &to, kind, &text, now),
            Op::Reply {
                letter,
                kind,
                text,
                now,
            } => self.op_reply(from, &letter, kind, &text, now),
            Op::Wait { to } => self.op_wait(from, &to),
            Op::List => Ok(self.op_list(from)),
            Op::Projects => Ok(self.op_projects(from)),
            Op::Show { to } => self.op_show(from, &to),
            Op::Ps => Ok(self.op_ps(from)),
            Op::Sleep { to } => self.op_sleep(from, &to),
            Op::Wake { to } => self.op_wake(from, &to),
            Op::PaneOpen { command } => self.op_pane_open(from, &command),
            Op::NodeAttach => self.op_node_attach(from),
            Op::Read { to, lines } => self.op_read(from, &to, lines),
            Op::Send {
                to,
                text,
                keys,
                enter,
            } => self.op_send(from, &to, &text, &keys, enter),
            Op::LearnMark {
                title,
                hard,
                excerpt,
            } => self.op_learn_mark(from, &title, &hard, &excerpt),
            Op::LearnDone { id, note } => self.op_learn_done(id, &note),
            Op::PlanSet { title, steps, ask } => self.op_plan_set(from, &title, steps, ask),
            Op::PlanShow { to } => self.op_plan_show(from, to.as_deref()),
            Op::PlanApprove => self.op_plan_approve(from),
            Op::PlanDrop => self.op_plan_drop(from),
            Op::MemoryStale { card, reason } => self.op_memory_stale(from, card, &reason),
        };
        write_reply(&self.naru_dir, &request.reply, result);
    }
}

#[cfg(test)]
#[path = "naru_tests.rs"]
mod tests;
