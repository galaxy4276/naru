//! What the PR and dev server panels show: open pull requests of the projects' repositories, and
//! the ports that programs started in Naru panes listen on.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use serde::Serialize;
use serde_json::Value;

use crate::state::{PaneId, Workspace};

/// One repository's open pull requests.
#[derive(Debug, Serialize)]
pub struct RepoPrs {
    pub repo: String,
    pub name: String,
    pub prs: Vec<Pr>,
    /// Why `gh` gave nothing (not on GitHub, signed out, offline).
    pub error: Option<String>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Pr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub branch: String,
    pub draft: bool,
    pub checks: Checks,
    /// Panes whose checkout is on this PR's branch.
    pub panes: Vec<PaneId>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Checks {
    None,
    Pass,
    Running,
    Fail,
}

/// A port a program started from a pane listens on.
#[derive(Debug, PartialEq, Serialize)]
pub struct Server {
    pub port: u16,
    pub pid: u32,
    pub command: String,
    /// Seconds since the program started.
    pub age: u64,
    pub pane: PaneId,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// Repositories of the projects and open panes, with the branch each pane is on.
fn repos(ws: &Workspace) -> BTreeMap<PathBuf, Vec<(String, PaneId)>> {
    let mut repos: BTreeMap<PathBuf, Vec<(String, PaneId)>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for path in ws.groups.iter().filter_map(|group| group.path.as_deref()) {
        if let Some(root) = git(Path::new(path), &["rev-parse", "--show-toplevel"]) {
            repos.entry(PathBuf::from(root)).or_default();
        }
    }
    for (pane, record) in &ws.panes {
        let Some(cwd) = record.cwd.as_deref().filter(|_| record.node.is_none()) else {
            continue;
        };
        let cwd = Path::new(cwd);
        let Some(root) = git(cwd, &["rev-parse", "--show-toplevel"]) else {
            continue;
        };
        let entry = repos.entry(PathBuf::from(&root)).or_default();
        if let Some(branch) = git(cwd, &["branch", "--show-current"])
            && seen.insert(*pane)
        {
            entry.push((branch, *pane));
        }
    }
    repos
}

/// Asks `gh` for each repository at once; one slow or failing repository holds up no other.
pub fn open_prs(ws: &Workspace, gh: Option<&Path>) -> Vec<RepoPrs> {
    let repos = repos(ws);
    let Some(gh) = gh else {
        return vec![RepoPrs {
            repo: String::new(),
            name: String::new(),
            prs: Vec::new(),
            error: Some("gh가 없어요 (brew install gh)".into()),
        }];
    };
    let jobs: Vec<_> = repos
        .into_iter()
        .map(|(root, panes)| {
            let gh = gh.to_path_buf();
            thread::spawn(move || {
                let name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let out = Command::new(&gh)
                    .current_dir(&root)
                    .args([
                        "pr",
                        "list",
                        "--state",
                        "open",
                        "--limit",
                        "30",
                        "--json",
                        "number,title,url,headRefName,isDraft,statusCheckRollup",
                    ])
                    .output();
                let (prs, error) = match out {
                    Ok(out) if out.status.success() => {
                        match parse_prs(&String::from_utf8_lossy(&out.stdout), &panes) {
                            Some(prs) => (prs, None),
                            None => (Vec::new(), Some("gh 응답을 읽지 못했어요".into())),
                        }
                    }
                    Ok(out) => {
                        let err = String::from_utf8_lossy(&out.stderr);
                        let line = err
                            .lines()
                            .find(|line| !line.trim().is_empty())
                            .unwrap_or("gh가 실패했어요");
                        (Vec::new(), Some(line.chars().take(200).collect()))
                    }
                    Err(error) => (Vec::new(), Some(error.to_string())),
                };
                RepoPrs {
                    repo: root.to_string_lossy().into_owned(),
                    name,
                    prs,
                    error,
                }
            })
        })
        .collect();
    jobs.into_iter().filter_map(|job| job.join().ok()).collect()
}

/// `gh pr list --json …` output, with the panes on each PR's branch.
pub fn parse_prs(json: &str, panes: &[(String, PaneId)]) -> Option<Vec<Pr>> {
    let list: Vec<Value> = serde_json::from_str(json).ok()?;
    let mut prs: Vec<Pr> = list
        .iter()
        .filter_map(|pr| {
            let branch = pr["headRefName"].as_str()?.to_owned();
            Some(Pr {
                number: pr["number"].as_u64()?,
                title: pr["title"].as_str()?.to_owned(),
                url: pr["url"].as_str()?.to_owned(),
                draft: pr["isDraft"].as_bool().unwrap_or(false),
                checks: checks(&pr["statusCheckRollup"]),
                panes: panes
                    .iter()
                    .filter(|(b, _)| *b == branch)
                    .map(|(_, pane)| *pane)
                    .collect(),
                branch,
            })
        })
        .collect();
    // PRs a session works on come first; the rest keep gh's newest-first order.
    prs.sort_by_key(|pr| pr.panes.is_empty());
    Some(prs)
}

/// A failure anywhere wins, then anything still running; skipped and neutral runs count as passed.
fn checks(rollup: &Value) -> Checks {
    let Some(items) = rollup.as_array().filter(|items| !items.is_empty()) else {
        return Checks::None;
    };
    let mut running = false;
    for item in items {
        // A CheckRun has status + conclusion, a StatusContext has state.
        let verdict = item["conclusion"]
            .as_str()
            .filter(|c| !c.is_empty())
            .or(item["state"].as_str());
        match verdict {
            Some(
                "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
                | "STARTUP_FAILURE",
            ) => {
                return Checks::Fail;
            }
            Some("SUCCESS" | "SKIPPED" | "NEUTRAL" | "STALE") => {}
            _ => running = true,
        }
    }
    if running {
        Checks::Running
    } else {
        Checks::Pass
    }
}

/// TCP ports `pid` listens on, read from its own file table. Neither lsof (it hung for days in
/// the kernel on the user's Mac) nor netstat (its internet table comes back empty unless an
/// interactive shell started it) can be trusted here (2026-11-06).
fn listening_ports(pid: u32) -> Vec<u16> {
    use libproc::libproc::bsd_info::BSDInfo;
    use libproc::libproc::file_info::{ListFDs, ProcFDType, pidfdinfo};
    use libproc::libproc::net_info::{SocketFDInfo, SocketInfoKind};
    use libproc::libproc::proc_pid::{listpidinfo, pidinfo};
    // From <sys/proc_info.h>.
    const TSI_S_LISTEN: i32 = 1;
    let pid = pid as i32;
    let Ok(info) = pidinfo::<BSDInfo>(pid, 0) else {
        return Vec::new();
    };
    let Ok(fds) = listpidinfo::<ListFDs>(pid, info.pbi_nfiles as usize) else {
        return Vec::new();
    };
    let mut ports = Vec::new();
    for fd in fds {
        if !matches!(fd.proc_fdtype.into(), ProcFDType::Socket) {
            continue;
        }
        let Ok(socket) = pidfdinfo::<SocketFDInfo>(pid, fd.proc_fd) else {
            continue;
        };
        if !matches!(socket.psi.soi_kind.into(), SocketInfoKind::Tcp) {
            continue;
        }
        // SAFETY: soi_kind says the union holds TCP info.
        let tcp = unsafe { socket.psi.soi_proto.pri_tcp };
        // The port is in network byte order in the low 16 bits.
        let port = u16::from_be(tcp.tcpsi_ini.insi_lport as u16);
        if tcp.tcpsi_state == TSI_S_LISTEN && !ports.contains(&port) {
            ports.push(port);
        }
    }
    ports
}

/// One `ps -axo pid=,ppid=,etime=,command=` row.
#[derive(Debug, PartialEq)]
pub struct Proc {
    pub ppid: u32,
    pub age: u64,
    pub command: String,
}

pub fn parse_ps(text: &str) -> HashMap<u32, Proc> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let ppid = parts.next()?.parse().ok()?;
            let age = etime(parts.next()?)?;
            let command = parts.collect::<Vec<_>>().join(" ");
            Some((pid, Proc { ppid, age, command }))
        })
        .collect()
}

/// `[[dd-]hh:]mm:ss` in seconds.
fn etime(text: &str) -> Option<u64> {
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, text),
    };
    let mut seconds = 0;
    for part in clock.split(':') {
        seconds = seconds * 60 + part.parse::<u64>().ok()?;
    }
    Some(days * 86_400 + seconds)
}

/// The pane whose shell `pid` descends from.
fn pane_of(pid: u32, procs: &HashMap<u32, Proc>, shells: &[(PaneId, u32)]) -> Option<PaneId> {
    let mut at = procs.get(&pid)?.ppid;
    // ps parents always point up; the bound only guards a table read mid-change.
    for _ in 0..64 {
        if let Some((pane, _)) = shells.iter().find(|(_, shell)| *shell == at) {
            return Some(*pane);
        }
        at = procs.get(&at)?.ppid;
    }
    None
}

/// Listening sockets whose process descends from a pane's shell.
pub fn match_servers(
    listening: &[(u32, u16)],
    procs: &HashMap<u32, Proc>,
    shells: &[(PaneId, u32)],
) -> Vec<Server> {
    let mut out: Vec<Server> = listening
        .iter()
        .filter_map(|&(pid, port)| {
            let proc = procs.get(&pid)?;
            Some(Server {
                port,
                pid,
                command: proc.command.clone(),
                age: proc.age,
                pane: pane_of(pid, procs, shells)?,
            })
        })
        .collect();
    out.sort_by_key(|server| server.port);
    out
}

pub fn servers(shells: &[(PaneId, u32)]) -> anyhow::Result<Vec<Server>> {
    let ps = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,etime=,command="])
        .output()?;
    anyhow::ensure!(ps.status.success(), "ps failed");
    let procs = parse_ps(&String::from_utf8_lossy(&ps.stdout));
    let listening: Vec<(u32, u16)> = procs
        .keys()
        .filter(|pid| pane_of(**pid, &procs, shells).is_some())
        .flat_map(|pid| {
            listening_ports(*pid)
                .into_iter()
                .map(move |port| (*pid, port))
        })
        .collect();
    Ok(match_servers(&listening, &procs, shells))
}

#[cfg(test)]
#[path = "devtools_tests.rs"]
mod tests;
