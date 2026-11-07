//! First run only: the Warp fork's saved workspace, read from a copy of its database so the running
//! fork is never touched. Groups, tabs, split trees, pane folders and titles, agent sessions, agent
//! teams and node panes come over; pane ids stay the fork's pane uuids, so `~/.naru` files keyed by
//! them (asleep.json, learn.json) keep pointing at the same panes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::agent::{Agent, is_plain_word};
use crate::session::{Session, Status};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};

use crate::state::{Group, Id, Layout, Node, PaneId, PaneRecord, Tab, Workspace};

const SQLITE: &str = "/usr/bin/sqlite3";

pub fn warp_db(home: &Path) -> PathBuf {
    home.join("Library/Application Support/dev.warp.WarpOss/warp.sqlite")
}

#[derive(Deserialize)]
struct WindowRow {
    id: i64,
    active_tab_index: i64,
}

#[derive(Deserialize)]
struct GroupRow {
    id: i64,
    name: Option<String>,
    color: Option<String>,
    collapsed: i64,
    pinned: i64,
    project_path: Option<String>,
}

#[derive(Deserialize)]
struct TabRow {
    id: i64,
    window_id: i64,
    custom_title: Option<String>,
    color: Option<String>,
    tab_group_id: Option<i64>,
    pinned: i64,
    background_image: Option<String>,
}

#[derive(Deserialize)]
struct NodeRow {
    id: i64,
    tab_id: i64,
    parent: Option<i64>,
    flex: Option<f32>,
    horizontal: Option<i64>,
    kind: Option<String>,
    is_focused: Option<i64>,
    title: Option<String>,
}

#[derive(Deserialize)]
struct TerminalRow {
    id: i64,
    uuid: String,
    cwd: Option<String>,
    cli_agent: Option<String>,
    cli_agent_session_id: Option<String>,
    cli_agent_launcher: Option<String>,
    cli_agent_lifecycle: Option<String>,
}

#[derive(Deserialize)]
struct ParentRow {
    child: String,
    parent: String,
}

#[derive(Deserialize)]
struct NodePaneRow {
    pane: String,
    host: String,
    cwd: Option<String>,
    resume: Option<String>,
}

/// The fork's `CLIAgentLifecycle`: `["Active", { … }]`; only these fields matter here.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Lifecycle {
    launch_command: Option<String>,
    activity_summary: Option<String>,
    shows_done: bool,
}

/// Reads the fork's database at `db` into a workspace. `naru_dir` holds `typed-titles.json`, the
/// titles the user typed, which automatic titles must not replace.
pub fn import(db: &Path, naru_dir: &Path) -> anyhow::Result<Workspace> {
    // The database is in WAL mode: opened read-only it cannot be read once the fork has quit and
    // taken its -shm file along. A copy of the files opens like any database and leaves the
    // originals as they were.
    let dir = std::env::temp_dir().join(format!("naru-import-{}", std::process::id()));
    let mut attempt = 0;
    loop {
        attempt += 1;
        let read = copy_files(db, &dir).and_then(|copy| read_copy(&copy, naru_dir));
        let _ = std::fs::remove_dir_all(&dir);
        match read {
            // A copy taken while the fork was writing can come out torn; the next one will not.
            Err(error) if attempt < 3 => {
                eprintln!("import: attempt {attempt} failed: {error:#}");
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            read => return read,
        }
    }
}

/// Copies the database and whichever of its -wal and -shm files exist into `dir`.
fn copy_files(db: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let copy = dir.join("warp.sqlite");
    std::fs::copy(db, &copy).with_context(|| format!("cannot copy {}", db.display()))?;
    for suffix in ["-wal", "-shm"] {
        let from = PathBuf::from(format!("{}{suffix}", db.display()));
        if from.exists() {
            std::fs::copy(&from, dir.join(format!("warp.sqlite{suffix}")))
                .with_context(|| format!("cannot copy {}", from.display()))?;
        }
    }
    Ok(copy)
}

fn read_copy(copy: &Path, naru_dir: &Path) -> anyhow::Result<Workspace> {
    let windows: Vec<WindowRow> =
        query(copy, "SELECT id, active_tab_index FROM windows ORDER BY id")?;
    let groups: Vec<GroupRow> = query(
        copy,
        "SELECT id, name, color, collapsed, pinned, project_path FROM tab_groups ORDER BY window_id, id",
    )?;
    let tabs: Vec<TabRow> = query(
        copy,
        "SELECT id, window_id, custom_title, color, tab_group_id, pinned, background_image \
         FROM tabs ORDER BY window_id, id",
    )?;
    let nodes: Vec<NodeRow> = query(
        copy,
        "SELECT n.id, n.tab_id, n.parent_pane_node_id AS parent, n.flex, b.horizontal, l.kind, \
         l.is_focused, l.custom_vertical_tabs_title AS title FROM pane_nodes n \
         LEFT JOIN pane_branches b ON b.pane_node_id = n.id \
         LEFT JOIN pane_leaves l ON l.pane_node_id = n.id ORDER BY n.id",
    )?;
    let terminals: Vec<TerminalRow> = query(
        copy,
        "SELECT id, lower(hex(uuid)) AS uuid, cwd, cli_agent, cli_agent_session_id, \
         cli_agent_launcher, cli_agent_lifecycle FROM terminal_panes",
    )?;
    let parents: Vec<ParentRow> = query(
        copy,
        "SELECT lower(hex(child)) AS child, lower(hex(parent)) AS parent FROM naru_agent_parents",
    )
    .unwrap_or_else(|error| {
        eprintln!("import: no agent teams: {error:#}");
        Vec::new()
    });
    let node_panes: Vec<NodePaneRow> = query(
        copy,
        "SELECT lower(hex(pane)) AS pane, host, cwd, resume FROM naru_node_panes",
    )
    .unwrap_or_else(|error| {
        eprintln!("import: no node panes: {error:#}");
        Vec::new()
    });
    let typed = typed_titles(naru_dir);
    Ok(build(
        &windows,
        &groups,
        &tabs,
        &nodes,
        &terminals,
        &parents,
        &node_panes,
        &typed,
    ))
}

/// Rows of `sql` from the copy, as `sqlite3 -json` prints them.
fn query<T: for<'de> Deserialize<'de>>(db: &Path, sql: &str) -> anyhow::Result<Vec<T>> {
    let output = Command::new(SQLITE)
        .arg("-json")
        .arg(db)
        .arg(sql)
        .output()?;
    if !output.status.success() {
        bail!("{sql}: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    if output.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&output.stdout).with_context(|| format!("reading rows of {sql}"))
}

fn typed_titles(naru_dir: &Path) -> HashSet<String> {
    let path = naru_dir.join("typed-titles.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            eprintln!("import: ignoring unreadable {}: {error}", path.display());
            HashSet::new()
        }),
        Err(_) => HashSet::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build(
    windows: &[WindowRow],
    groups: &[GroupRow],
    tabs: &[TabRow],
    nodes: &[NodeRow],
    terminals: &[TerminalRow],
    parents: &[ParentRow],
    node_panes: &[NodePaneRow],
    typed: &HashSet<String>,
) -> Workspace {
    let mut ws = Workspace::default();
    let mut group_ids: HashMap<i64, Id> = HashMap::new();
    for row in groups {
        let name = row
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .or_else(|| {
                let path = row.project_path.as_deref()?;
                Some(Path::new(path).file_name()?.to_string_lossy().into_owned())
            });
        // A group with neither name nor folder has nothing to show; its tabs stay loose.
        let Some(name) = name else { continue };
        let id = ws.add_group(name, row.project_path.clone());
        let group = ws.groups.last_mut().expect("just added");
        group.color = row.color.clone();
        group.collapsed = row.collapsed != 0;
        group.pinned = row.pinned != 0;
        group_ids.insert(row.id, id);
    }

    let terminal_of: HashMap<i64, &TerminalRow> = terminals.iter().map(|t| (t.id, t)).collect();
    let mut children: BTreeMap<Option<i64>, Vec<&NodeRow>> = BTreeMap::new();
    for node in nodes {
        children.entry(node.parent).or_default().push(node);
    }
    let pane_ids: HashMap<&str, PaneId> = terminals
        .iter()
        .filter_map(|t| Some((t.uuid.as_str(), PaneId::try_parse(&t.uuid).ok()?)))
        .collect();

    let mut active = None;
    for window in windows {
        let window_tabs: Vec<&TabRow> = tabs
            .iter()
            .filter(|tab| tab.window_id == window.id)
            .collect();
        for (index, row) in window_tabs.iter().enumerate() {
            let Some(root) = nodes
                .iter()
                .find(|n| n.tab_id == row.id && n.parent.is_none())
            else {
                continue;
            };
            let mut focused = None;
            let Some(layout) = layout_of(root, &children, &terminal_of, &pane_ids, &mut focused)
            else {
                continue;
            };
            let id = ws.add_imported_tab(Tab {
                id: 0,
                group: row
                    .tab_group_id
                    .and_then(|group| group_ids.get(&group).copied()),
                title: row
                    .custom_title
                    .clone()
                    .map(|t| t.trim().to_owned())
                    .filter(|t| !t.is_empty()),
                color: row.color.clone(),
                pinned: row.pinned != 0,
                background: row.background_image.clone(),
                focused: focused.unwrap_or_else(|| layout.panes()[0]),
                layout,
            });
            if window.id == windows[0].id && index as i64 == window.active_tab_index {
                active = Some(id);
            }
        }
    }
    if active.is_some() {
        ws.active_tab = active;
    }

    let titles: HashMap<i64, &str> = nodes
        .iter()
        .filter_map(|n| Some((n.id, n.title.as_deref().filter(|t| !t.is_empty())?)))
        .collect();
    let parent_of: HashMap<&str, &str> = parents
        .iter()
        .map(|p| (p.child.as_str(), p.parent.as_str()))
        .collect();
    let node_of: HashMap<&str, &NodePaneRow> =
        node_panes.iter().map(|n| (n.pane.as_str(), n)).collect();
    let laid_out: HashSet<PaneId> = ws.tabs.iter().flat_map(|tab| tab.layout.panes()).collect();
    for terminal in terminals {
        let Some(&pane) = pane_ids.get(terminal.uuid.as_str()) else {
            continue;
        };
        if !laid_out.contains(&pane) {
            continue;
        }
        let lifecycle = terminal
            .cli_agent_lifecycle
            .as_deref()
            .and_then(|text| serde_json::from_str::<(Value, Lifecycle)>(text).ok())
            .map(|(_, lifecycle)| lifecycle)
            .unwrap_or_default();
        let title = titles.get(&terminal.id).map(|t| (*t).to_owned());
        let typed_title = title.as_ref().is_some_and(|t| typed.contains(t));
        let node = node_of
            .get(terminal.uuid.as_str())
            .and_then(|row| node_record(row));
        let session = if node.is_some() {
            None
        } else {
            session_of(terminal, &lifecycle)
        };
        ws.panes.insert(
            pane,
            PaneRecord {
                cwd: terminal.cwd.clone(),
                title: title.clone().filter(|_| typed_title),
                auto_title: title.filter(|_| !typed_title),
                summary: lifecycle.activity_summary.clone(),
                session,
                parent: parent_of
                    .get(terminal.uuid.as_str())
                    .and_then(|parent| pane_ids.get(parent).copied()),
                node,
                ..PaneRecord::default()
            },
        );
    }
    ws.repair();
    ws
}

/// The tab's split tree with only its terminal panes; `None` when it has none.
fn layout_of(
    node: &NodeRow,
    children: &BTreeMap<Option<i64>, Vec<&NodeRow>>,
    terminal_of: &HashMap<i64, &TerminalRow>,
    pane_ids: &HashMap<&str, PaneId>,
    focused: &mut Option<PaneId>,
) -> Option<Layout> {
    if node.kind.is_some() {
        if node.kind.as_deref() != Some("terminal") {
            return None;
        }
        let pane = *pane_ids.get(terminal_of.get(&node.id)?.uuid.as_str())?;
        if node.is_focused == Some(1) {
            *focused = Some(pane);
        }
        return Some(Layout::Pane { pane });
    }
    let mut kept = Vec::new();
    let mut flex = Vec::new();
    for child in children.get(&Some(node.id)).into_iter().flatten() {
        if let Some(layout) = layout_of(child, children, terminal_of, pane_ids, focused) {
            kept.push(layout);
            flex.push(child.flex.unwrap_or(1.0));
        }
    }
    match kept.len() {
        0 => None,
        1 => kept.pop(),
        // The fork's horizontal split lays panes left to right.
        _ => Some(Layout::Split {
            row: node.horizontal == Some(1),
            children: kept,
            flex,
        }),
    }
}

/// The pane's agent, as a restart resumes it. Values are checked again when the resume command
/// is built, so an edited database cannot put a command line together.
fn session_of(terminal: &TerminalRow, lifecycle: &Lifecycle) -> Option<Session> {
    let agent: Agent = serde_json::from_value(Value::String(terminal.cli_agent.clone()?)).ok()?;
    let launcher = terminal
        .cli_agent_launcher
        .clone()
        .filter(|l| is_plain_word(l));
    let mut session = Session::new(agent, launcher, lifecycle.launch_command.clone());
    session.session_id = terminal
        .cli_agent_session_id
        .clone()
        .filter(|id| is_plain_word(id));
    session.cwd = terminal.cwd.clone();
    session.status = Status::Success;
    // Only a turn the fork still showed as done comes back done; the rest wait at their prompt.
    session.seen = !lifecycle.shows_done;
    session.read = true;
    Some(session)
}

/// A node pane, when its host and saved resume command are plain enough to run again.
fn node_record(row: &NodePaneRow) -> Option<Node> {
    if !crate::nodes::is_host(&row.host) {
        eprintln!("import: skipping node pane with host {:?}", row.host);
        return None;
    }
    let resume = row.resume.clone().filter(|resume| {
        shell_words::split(resume).is_ok_and(|words| {
            !words.is_empty()
                && words
                    .iter()
                    .all(|word| is_plain_word(word.strip_prefix("--").unwrap_or(word)))
        })
    });
    Some(Node {
        host: row.host.clone(),
        cwd: row.cwd.clone(),
        resume,
        first: None,
    })
}

/// The fork's project thumbnails, base64 PNG by group, for groups whose folder had one. The fork
/// kept them as `~/.warp-oss/project-thumbnails/<sha256 of the folder>.png`.
pub fn thumbnails(home: &Path, groups: &[Group]) -> BTreeMap<Id, String> {
    let dir = home.join(".warp-oss/project-thumbnails");
    groups
        .iter()
        .filter_map(|group| {
            let digest = Sha256::digest(group.path.as_deref()?.as_bytes());
            let name: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            let file = dir.join(format!("{name}.png"));
            match std::fs::read(&file) {
                Ok(png) => Some((group.id, BASE64.encode(png))),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    eprintln!("cannot read {}: {error}", file.display());
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
