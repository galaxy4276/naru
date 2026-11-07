//! Panes that run agents on another machine over ssh. The pane's own shell runs
//! `naru node attach`, which asks the daemon for the ssh line and runs it again whenever the
//! connection drops; the agent's resume goes along so a reconnect lands in the same conversation.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::core::Daemon;
use crate::state::PaneId;

pub const ATTACH: &str = "naru node attach";

/// The token-less request a node's prompt hook sends when its shell prompt shows again.
pub fn is_prompt_signal(body: &str) -> bool {
    serde_json::from_str::<Value>(body).is_ok_and(|value| value == json!({ "op": "node_prompt" }))
}

/// An ssh destination that cannot be read as an option or carry shell syntax.
pub fn is_host(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'@'))
}

/// `cd` into a folder on the node, where `~` is that machine's home.
fn cd(cwd: &str) -> String {
    match cwd.strip_prefix('~') {
        Some("") => "cd".into(),
        Some(rest) if rest.starts_with('/') => format!("cd ~/{}", shell_words::quote(&rest[1..])),
        _ => format!("cd {}", shell_words::quote(cwd)),
    }
}

/// The connection settings of `ssh -G` output, without the user's forwards and control master:
/// those fail with `ExitOnForwardFailure` or take the user's own tunnel ports.
pub fn ssh_options_from_config(config: &str) -> Vec<String> {
    const KEEP: &[&str] = &[
        "hostname",
        "user",
        "port",
        "identityfile",
        "identitiesonly",
        "certificatefile",
        "proxyjump",
        "proxycommand",
        "userknownhostsfile",
        "hostkeyalias",
        "stricthostkeychecking",
    ];
    config
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(key, value)| KEEP.contains(key) && *value != "none")
        .map(|(key, value)| format!("{key}={value}"))
        .collect()
}

fn resolved_ssh_options(host: &str) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("ssh")
        .args(["-G", host])
        .output()
        .map_err(|error| format!("cannot run ssh: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ssh -G {host} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(ssh_options_from_config(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// The ssh line a node pane runs: a login shell there with the environment the agents' Warp
/// plugins and the naru helper look for, and Naru's socket forwarded for the helper's replies.
pub fn ssh_command(
    host: &str,
    cwd: Option<&str>,
    command: &str,
    local_socket: &Path,
    options: &[String],
) -> Result<String, String> {
    if !is_host(host) {
        return Err(format!("not an ssh host: {host}"));
    }
    let socket = format!(
        "/tmp/naru-{}.sock",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    );
    let env = format!(
        "NARU_SOCK={socket} NARU_PANE=1 NARU_NODE={host} TERM_PROGRAM=WarpTerminal \
         WARP_CLIENT_VERSION=local WARP_CLI_AGENT_PROTOCOL_VERSION=1"
    );
    // The agents and their launchers are set up only in an interactive shell there.
    let shell = if command.trim().is_empty() {
        r#""$SHELL" -li"#.to_string()
    } else {
        let run = format!(r#"{}; exec "$SHELL" -li"#, command.trim());
        format!(r#""$SHELL" -lic {}"#, shell_words::quote(&run))
    };
    let remote = match cwd {
        Some(cwd) => format!("{} && exec env {env} {shell}", cd(cwd)),
        None => format!("exec env {env} {shell}"),
    };
    let forward = format!("{socket}:{}", local_socket.display());
    let options: String = options
        .iter()
        .map(|option| format!(" -o {}", shell_words::quote(option)))
        .collect();
    Ok(format!(
        "command ssh -F /dev/null{options} -t -o ExitOnForwardFailure=yes \
         -o StreamLocalBindUnlink=yes -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -R {} \
         {host} {}",
        shell_words::quote(&forward),
        shell_words::quote(&remote)
    ))
}

impl Daemon {
    /// The ssh line for `naru node attach`: the spawned command the first time, the agent's
    /// resume after that.
    pub fn op_node_attach(self: &Arc<Self>, pane: PaneId) -> Result<Value, String> {
        let not_node = "this pane is not a naru node pane; open one with naru node connect";
        let (host, cwd, command) = {
            let mut core = self.core();
            let node = core
                .workspace
                .panes
                .get_mut(&pane)
                .and_then(|record| record.node.as_mut())
                .ok_or(not_node)?;
            let command = node
                .first
                .take()
                .or_else(|| node.resume.clone())
                .unwrap_or_default();
            let found = (node.host.clone(), node.cwd.clone(), command);
            self.changed(&core);
            found
        };
        // ssh -G runs without the lock: it may wait on the network for a ProxyCommand host.
        let options = resolved_ssh_options(&host)?;
        let socket = self.naru_dir.join("naru.sock");
        let line = ssh_command(&host, cwd.as_deref(), &command, &socket, &options)?;
        Ok(json!({ "line": line, "host": host }))
    }

    /// The node's prompt showed again: the agent it ran has ended, which Naru cannot see through
    /// ssh, so a restart must not resume it.
    pub fn node_prompt_returned(&self, pane: PaneId) {
        let mut core = self.core();
        let Some(record) = core.workspace.panes.get_mut(&pane) else {
            return;
        };
        let Some(node) = record.node.as_mut() else {
            return;
        };
        let changed = node.resume.take().is_some() | record.session.take().is_some();
        if changed {
            self.changed(&core);
        }
    }
}

#[cfg(test)]
#[path = "nodes_tests.rs"]
mod tests;
