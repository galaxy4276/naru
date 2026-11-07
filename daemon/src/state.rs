//! The workspace a restart brings back: project groups, tabs, each tab's split tree, and what
//! every pane was running. Saved whole to `state.json` after every change.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::Session;

pub type PaneId = Uuid;
pub type Id = u64;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub groups: Vec<Group>,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<Id>,
    pub panes: BTreeMap<PaneId, PaneRecord>,
    /// Files agents showed the user with `naru open`, oldest first.
    pub views: Vec<View>,
    next_id: Id,
}

/// A file in the document lane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub id: Id,
    pub path: String,
    /// A review report for the proof panel (`naru proof`).
    #[serde(default)]
    pub proof: bool,
    /// The pane whose agent opened it.
    pub from: Option<PaneId>,
    /// The user has looked at it; until then it waits in 내 차례.
    #[serde(default)]
    pub seen: bool,
}

/// A pane whose shell runs `naru node attach`: an ssh session to another machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub host: String,
    /// The folder there; `None` is the node user's home.
    pub cwd: Option<String>,
    /// What a reconnect runs there: the agent's resume command once it reported its session.
    pub resume: Option<String>,
    /// The command `naru agent spawn --machine` asked for, run on the first attach only.
    pub first: Option<String>,
}

/// A project: the tabs working in one folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: Id,
    pub name: String,
    pub path: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub pinned: bool,
}

/// A session card in the sidebar; its panes are the small cards inside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: Id,
    pub group: Option<Id>,
    pub title: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    pub background: Option<String>,
    pub layout: Layout,
    pub focused: PaneId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Layout {
    Pane {
        pane: PaneId,
    },
    /// `row` lays the children side by side; otherwise they stack top to bottom.
    Split {
        row: bool,
        children: Vec<Layout>,
        flex: Vec<f32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Right,
    Down,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneRecord {
    pub cwd: Option<String>,
    /// A title the user typed; automatic titles never replace it.
    pub title: Option<String>,
    /// The title haiku gave the pane's conversation.
    pub auto_title: Option<String>,
    /// What the pane's agent is doing, in one line from haiku.
    pub summary: Option<String>,
    pub session: Option<Session>,
    /// The command line the shell is running now.
    #[serde(skip_deserializing)]
    pub running: Option<String>,
    /// The program's own window title (OSC 0/2).
    #[serde(skip_deserializing)]
    pub program_title: Option<String>,
    /// The pane whose agent started this one with `naru agent spawn --parent here`.
    pub parent: Option<PaneId>,
    pub node: Option<Node>,
    /// Columns and rows the screen last gave the pane. A restored shell starts at this size, so
    /// zsh's partial-line mark is not left behind when the screen replays its first prompt.
    pub size: Option<(u16, u16)>,
}

/// macOS shows `/tmp`, `/var` and `/etc` also under `/private`; a shell may report either.
fn unprivate(path: &Path) -> PathBuf {
    match path.strip_prefix("/private") {
        Ok(rest)
            if ["tmp", "var", "etc"]
                .iter()
                .any(|top| rest.starts_with(top)) =>
        {
            Path::new("/").join(rest)
        }
        _ => path.to_path_buf(),
    }
}

impl Layout {
    pub fn panes(&self) -> Vec<PaneId> {
        match self {
            Layout::Pane { pane } => vec![*pane],
            Layout::Split { children, .. } => children.iter().flat_map(Layout::panes).collect(),
        }
    }

    fn contains(&self, id: PaneId) -> bool {
        match self {
            Layout::Pane { pane } => *pane == id,
            Layout::Split { children, .. } => children.iter().any(|child| child.contains(id)),
        }
    }

    fn split(&mut self, at: PaneId, new: PaneId, row: bool) -> bool {
        match self {
            Layout::Pane { pane } if *pane == at => {
                *self = Layout::Split {
                    row,
                    children: vec![Layout::Pane { pane: at }, Layout::Pane { pane: new }],
                    flex: vec![1.0, 1.0],
                };
                true
            }
            Layout::Pane { .. } => false,
            Layout::Split {
                row: split_row,
                children,
                flex,
            } => {
                let Some(index) = children.iter().position(|child| child.contains(at)) else {
                    return false;
                };
                if *split_row == row && matches!(children[index], Layout::Pane { .. }) {
                    children.insert(index + 1, Layout::Pane { pane: new });
                    flex.insert(index + 1, flex[index]);
                    true
                } else {
                    children[index].split(at, new, row)
                }
            }
        }
    }

    /// Removes the pane; `false` when nothing is left of this node.
    fn remove(&mut self, id: PaneId) -> bool {
        match self {
            Layout::Pane { pane } => *pane != id,
            Layout::Split { children, flex, .. } => {
                let mut index = 0;
                while index < children.len() {
                    if children[index].remove(id) {
                        index += 1;
                    } else {
                        children.remove(index);
                        flex.remove(index);
                    }
                }
                match children.len() {
                    0 => false,
                    1 => {
                        *self = children.remove(0);
                        true
                    }
                    _ => true,
                }
            }
        }
    }
}

impl Workspace {
    pub fn load(path: &Path) -> anyhow::Result<Workspace> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("{} is not a Naru workspace", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Workspace::default()),
            Err(error) => Err(error.into()),
        }
    }

    /// Replaces the file whole, so a crash mid-write leaves the previous workspace.
    // ponytail: no fsync; survives the daemon dying, not the machine losing power mid-write.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let temp = path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        fs::rename(&temp, path)?;
        Ok(())
    }

    fn next_id(&mut self) -> Id {
        self.next_id += 1;
        self.next_id
    }

    /// The group whose folder holds `cwd`, the deepest one when folders nest.
    pub fn group_for(&self, cwd: &str) -> Option<Id> {
        let cwd = unprivate(Path::new(cwd));
        self.groups
            .iter()
            .filter_map(|group| {
                let path = unprivate(Path::new(group.path.as_deref()?));
                cwd.starts_with(&path)
                    .then_some((path.components().count(), group.id))
            })
            .max()
            .map(|(_, id)| id)
    }

    pub fn add_group(&mut self, name: String, path: Option<String>) -> Id {
        let id = self.next_id();
        self.groups.push(Group {
            id,
            name,
            path,
            color: None,
            collapsed: false,
            pinned: false,
        });
        id
    }

    pub fn rename_group(&mut self, group: Id, name: &str) -> anyhow::Result<()> {
        let name = name.trim();
        anyhow::ensure!(!name.is_empty(), "project name is empty");
        let found = self
            .groups
            .iter_mut()
            .find(|g| g.id == group)
            .ok_or_else(|| anyhow::anyhow!("no project {group}"))?;
        found.name = name.to_string();
        Ok(())
    }

    /// Appends a tab read from elsewhere, giving it this workspace's next id.
    pub fn add_imported_tab(&mut self, mut tab: Tab) -> Id {
        tab.id = self.next_id();
        let id = tab.id;
        self.tabs.push(tab);
        id
    }

    /// Where a new tab of `group` goes: after the group's last tab.
    pub fn end_of_group(&self, group: Option<Id>) -> usize {
        self.tabs
            .iter()
            .rposition(|tab| tab.group == group)
            .map_or(self.tabs.len(), |index| index + 1)
    }

    /// A new tab with one pane, placed after the last tab of its group, and made active.
    pub fn add_tab(&mut self, group: Option<Id>, cwd: Option<String>) -> (Id, PaneId) {
        let (id, pane) = self.insert_tab(self.end_of_group(group), group, cwd);
        self.active_tab = Some(id);
        (id, pane)
    }

    /// A new tab with one pane at `at`, leaving the active tab as it was.
    pub fn insert_tab(
        &mut self,
        at: usize,
        group: Option<Id>,
        cwd: Option<String>,
    ) -> (Id, PaneId) {
        let id = self.next_id();
        let pane = Uuid::new_v4();
        self.panes.insert(
            pane,
            PaneRecord {
                cwd,
                ..PaneRecord::default()
            },
        );
        self.tabs.insert(
            at,
            Tab {
                id,
                group,
                title: None,
                color: None,
                pinned: false,
                background: None,
                layout: Layout::Pane { pane },
                focused: pane,
            },
        );
        (id, pane)
    }

    /// Shows `path` in the document lane; a file shown again comes to the front, unseen.
    pub fn add_view(&mut self, path: String, proof: bool, from: Option<PaneId>) -> &mut View {
        self.views.retain(|view| view.path != path);
        let id = self.next_id();
        self.views.push(View {
            id,
            path,
            proof,
            from,
            seen: false,
        });
        self.views.last_mut().expect("just pushed")
    }

    /// A new pane beside `at`, starting in its folder; focused when `focus`.
    pub fn split(
        &mut self,
        at: PaneId,
        direction: Direction,
        focus: bool,
    ) -> anyhow::Result<PaneId> {
        let cwd = self.panes.get(&at).and_then(|record| record.cwd.clone());
        let pane = Uuid::new_v4();
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.layout.contains(at)) else {
            bail!("no pane {at}");
        };
        tab.layout.split(at, pane, direction == Direction::Right);
        if focus {
            tab.focused = pane;
            self.active_tab = Some(tab.id);
        }
        self.panes.insert(
            pane,
            PaneRecord {
                cwd,
                ..PaneRecord::default()
            },
        );
        Ok(pane)
    }

    /// Drops the pane, and its tab once the tab has no pane left.
    pub fn remove_pane(&mut self, pane: PaneId) {
        self.panes.remove(&pane);
        let Some(index) = self.tabs.iter().position(|tab| tab.layout.contains(pane)) else {
            return;
        };
        let tab = &mut self.tabs[index];
        if tab.layout.remove(pane) {
            if tab.focused == pane {
                tab.focused = tab.layout.panes()[0];
            }
            return;
        }
        let removed = self.tabs.remove(index);
        if self.active_tab == Some(removed.id) {
            self.active_tab = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|tab| tab.id);
        }
    }

    /// Panes in the order a restart resumes them: the focused card first (its focused pane
    /// leading), then the rest of its project, then every other card as listed.
    pub fn resume_order(&self) -> Vec<PaneId> {
        let active = self.tabs.iter().find(|tab| Some(tab.id) == self.active_tab);
        let rank = |tab: &Tab| match active {
            Some(active) if tab.id == active.id => 0,
            Some(active) if tab.group == active.group => 1,
            _ => 2,
        };
        let mut tabs: Vec<&Tab> = self.tabs.iter().collect();
        tabs.sort_by_key(|tab| rank(tab));
        tabs.into_iter()
            .flat_map(|tab| {
                let mut panes = tab.layout.panes();
                panes.sort_by_key(|pane| *pane != tab.focused);
                panes
            })
            .collect()
    }

    pub fn tab_of(&self, pane: PaneId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.layout.contains(pane))
    }

    /// The pane the user is looking at.
    pub fn focused(&self) -> Option<PaneId> {
        let active = self.active_tab?;
        self.tabs
            .iter()
            .find(|tab| tab.id == active)
            .map(|tab| tab.focused)
    }

    /// Whether `pane` was started, directly or not, by `ancestor`.
    pub fn descends_from(&self, mut pane: PaneId, ancestor: PaneId) -> bool {
        let mut hops = 0;
        while let Some(parent) = self.panes.get(&pane).and_then(|record| record.parent) {
            if parent == ancestor {
                return true;
            }
            hops += 1;
            // A loop written by hand would otherwise never end.
            if hops > self.panes.len() {
                return false;
            }
            pane = parent;
        }
        false
    }

    pub fn focus(&mut self, pane: PaneId) -> anyhow::Result<()> {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.layout.contains(pane)) else {
            bail!("no pane {pane}");
        };
        tab.focused = pane;
        self.active_tab = Some(tab.id);
        Ok(())
    }

    /// Panes a layout names but the workspace lost, or the other way round, from a file written
    /// by hand or by an older version.
    pub fn repair(&mut self) {
        let laid_out: Vec<PaneId> = self
            .tabs
            .iter()
            .flat_map(|tab| tab.layout.panes())
            .collect();
        self.panes.retain(|id, _| laid_out.contains(id));
        for id in laid_out {
            self.panes.entry(id).or_default();
        }
        let ids: Vec<PaneId> = self.panes.keys().copied().collect();
        for record in self.panes.values_mut() {
            record.parent = record.parent.filter(|parent| ids.contains(parent));
        }
        for tab in &mut self.tabs {
            if !tab.layout.contains(tab.focused) {
                tab.focused = tab.layout.panes()[0];
            }
        }
        let groups: Vec<Id> = self.groups.iter().map(|group| group.id).collect();
        for tab in &mut self.tabs {
            tab.group = tab.group.filter(|group| groups.contains(group));
        }
        if !self
            .active_tab
            .is_some_and(|active| self.tabs.iter().any(|tab| tab.id == active))
        {
            self.active_tab = self.tabs.first().map(|tab| tab.id);
        }
        let highest = self
            .tabs
            .iter()
            .map(|tab| tab.id)
            .chain(groups)
            .chain(self.views.iter().map(|view| view.id))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(highest);
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
