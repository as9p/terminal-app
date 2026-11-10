//! The tree the sidebar shows: groups of tabs, each tab a split of panes, and the rules for
//! where a tab lands and when it moves. Pure: the only outside call is `project::resolve`, made
//! only by the convenience calls (`add_session`, `on_cwd`, `on_first_edit`, `apply`). The `_in`
//! forms take the project already resolved, so a caller can look at the disk before it takes a
//! lock. There is no clock, and the caller carries out the kills and spawns `apply` asks for.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::project::{self, Project};
use crate::state::SavedSession;

/// How many closed tabs `ReopenTab` can bring back.
const KEEP_CLOSED: usize = 20;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tree {
    pub groups: Vec<Group>,
    pub ungrouped: Vec<Tab>,
    /// Oldest first, so `ReopenTab` takes the last.
    pub closed: Vec<ClosedTab>,
    /// What each removed group had, by key, put back when the group comes back.
    pub defaults: BTreeMap<String, GroupDefaults>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    /// The project key from `project::resolve`.
    pub key: String,
    /// The git root, or the folder that was opened when it has no git.
    pub path: PathBuf,
    /// A name Alex set. Nothing automatic writes it.
    pub name: Option<String>,
    pub branch: Option<String>,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub tabs: Vec<Tab>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: String,
    /// A name Alex set. Nothing automatic writes it.
    pub name: Option<String>,
    /// Set by a move by hand. A pinned tab never moves by itself.
    #[serde(default)]
    pub pinned: bool,
    pub split: Split,
    /// The order of use, not a time: the tab named by the newest `Used` op has the highest
    /// number. 0 for a tab never used.
    #[serde(default)]
    pub last_used: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Split {
    /// One pane, named by its session uuid.
    Leaf { session: String },
    /// Side by side, left to right. `sizes` holds each child's share of the width.
    Row {
        children: Vec<Split>,
        sizes: Vec<f32>,
    },
    /// Stacked, top to bottom. `sizes` holds each child's share of the height.
    Col {
        children: Vec<Split>,
        sizes: Vec<f32>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClosedTab {
    pub tab: Tab,
    /// The key of the group it was in, or None when it was ungrouped.
    pub group: Option<String>,
    pub index: usize,
    /// Filled in by the server from its sessions after `CloseTab`, since the tree only knows
    /// session uuids. A leaf with no pane here does not come back.
    #[serde(default)]
    pub panes: Vec<SavedSession>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupDefaults {
    pub name: Option<String>,
    pub collapsed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    Right,
    Down,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Op {
    OpenFolder {
        path: PathBuf,
    },
    CloseTab {
        tab: String,
    },
    ReopenTab,
    MoveTab {
        tab: String,
        group: Option<String>,
        index: usize,
    },
    MoveGroup {
        group: String,
        index: usize,
    },
    RenameTab {
        tab: String,
        name: Option<String>,
    },
    RenameGroup {
        group: String,
        name: Option<String>,
    },
    PinTab {
        tab: String,
        pinned: bool,
    },
    CollapseGroup {
        group: String,
        collapsed: bool,
    },
    JoinTabs {
        tab: String,
        into: String,
        dir: Dir,
    },
    SplitOut {
        session: String,
    },
    SetSizes {
        tab: String,
        split: Split,
    },
    Used {
        tab: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Place {
    #[default]
    NewTab,
    SplitOf {
        session: String,
        dir: Dir,
    },
}

/// What the caller must do after `apply`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// Session uuids to end, left to right, top to bottom.
    pub kill: Vec<String>,
    /// Sessions to start. An empty `uuid` (from `OpenFolder`) asks the caller to pick one and
    /// then call `add_session` with `Place::NewTab` and the spawn's folder.
    pub spawn: Vec<SavedSession>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeError {
    NoTab,
    NoGroup,
    BadOp,
}

impl TreeError {
    /// The error code the protocol sends.
    pub fn code(self) -> &'static str {
        match self {
            TreeError::NoTab => "no_tab",
            TreeError::NoGroup => "no_group",
            TreeError::BadOp => "bad_op",
        }
    }
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            TreeError::NoTab => "no such tab",
            TreeError::NoGroup => "no such group",
            TreeError::BadOp => "the tree cannot do that",
        };
        f.write_str(text)
    }
}

impl std::error::Error for TreeError {}

/// Where a tab sits: the group's index, or None for ungrouped, and its index there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Spot {
    group: Option<usize>,
    index: usize,
}

impl Tree {
    /// Puts a new session in the tree and returns the id of its tab. `kind_is_shell` is not
    /// used by any rule yet: a Claude session lands by the same folder rules as a shell.
    pub fn add_session(
        &mut self,
        session: &str,
        place: Place,
        cwd: &Path,
        kind_is_shell: bool,
    ) -> String {
        self.add_session_in(session, place, &project::resolve(cwd), kind_is_shell)
    }

    /// `add_session` with the project of its folder already resolved.
    pub fn add_session_in(
        &mut self,
        session: &str,
        place: Place,
        project: &Project,
        _kind_is_shell: bool,
    ) -> String {
        if let Some(spot) = self.find_session(session) {
            return self.tab(spot).id.clone();
        }
        // A split of a session that is gone becomes a tab of its own.
        if let Place::SplitOf { session: of, dir } = &place
            && let Some(spot) = self.find_session(of)
        {
            let tab = self.tab_mut(spot);
            tab.split.split_leaf(of, session, *dir);
            return tab.id.clone();
        }
        let group = match self.group_for(project) {
            Some(index) => Some(index),
            None if project.is_git => Some(self.create_group(project)),
            None => self.last_used_group(),
        };
        let id = self.next_id();
        self.tabs_mut(group).push(Tab {
            id: id.clone(),
            name: None,
            pinned: false,
            split: leaf(session),
            last_used: 0,
        });
        id
    }

    /// A shell reported its folder. An unpinned tab whose first pane moved into a git repo,
    /// or into a folder that is a group, moves to the end of that group, creating it for a
    /// repo. The group's branch is refreshed whenever one of its tabs reports a folder in it.
    /// True when the tree changed.
    pub fn on_cwd(&mut self, session: &str, cwd: &Path) -> bool {
        self.on_cwd_in(session, &project::resolve(cwd))
    }

    /// `on_cwd` with the project of the reported folder already resolved.
    pub fn on_cwd_in(&mut self, session: &str, project: &Project) -> bool {
        let Some(spot) = self.find_session(session) else {
            return false;
        };
        let target = self.group_for(project);
        let tab = self.tab(spot);
        let moves = !tab.pinned
            && tab.split.first() == session
            && match target {
                Some(target) => spot.group != Some(target),
                None => project.is_git,
            };
        let mut changed = false;
        if moves {
            let tab = self.tabs_mut(spot.group).remove(spot.index);
            let index = match target {
                Some(index) => index,
                None => self.create_group(project),
            };
            self.groups[index].tabs.push(tab);
            self.tidy();
            changed = true;
        }
        if project.is_git {
            let now = self.find_session(session).and_then(|spot| spot.group);
            if let Some(index) = now.filter(|&index| self.group_for(project) == Some(index)) {
                let group = &mut self.groups[index];
                if group.branch != project.branch {
                    group.branch.clone_from(&project.branch);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Claude edited a file for the first time in this session. An ungrouped, unpinned tab
    /// moves to the end of the file's git repo group, creating it. True when the tree changed.
    pub fn on_first_edit(&mut self, session: &str, file: &Path) -> bool {
        self.on_first_edit_in(session, &project::resolve(file.parent().unwrap_or(file)))
    }

    /// `on_first_edit` with the project of the edited file already resolved.
    pub fn on_first_edit_in(&mut self, session: &str, project: &Project) -> bool {
        let Some(spot) = self.find_session(session) else {
            return false;
        };
        if spot.group.is_some() || self.tab(spot).pinned || !project.is_git {
            return false;
        }
        let tab = self.ungrouped.remove(spot.index);
        let index = match self.group_for(project) {
            Some(index) => index,
            None => self.create_group(project),
        };
        self.groups[index].tabs.push(tab);
        true
    }

    /// Closes a tab and keeps it for `ReopenTab` with `panes`, what its sessions need to come
    /// back, read by the caller beforehand. A tab none of whose leaves has a pane there is not
    /// kept, so `ReopenTab` never takes an entry that brings nothing back.
    pub fn close_tab(&mut self, tab: &str, panes: Vec<SavedSession>) -> Result<Effects, TreeError> {
        let spot = self.find_tab(tab).ok_or(TreeError::NoTab)?;
        let group = spot.group.map(|index| self.groups[index].key.clone());
        let tab = self.tabs_mut(spot.group).remove(spot.index);
        let kill = tab.split.leaves();
        let panes: Vec<SavedSession> = panes
            .into_iter()
            .filter(|pane| kill.contains(&pane.uuid))
            .collect();
        if !panes.is_empty() {
            self.closed.push(ClosedTab {
                tab,
                group,
                index: spot.index,
                panes,
            });
            let extra = self.closed.len().saturating_sub(KEEP_CLOSED);
            self.closed.drain(..extra);
        }
        self.tidy();
        Ok(Effects {
            kill,
            spawn: Vec::new(),
        })
    }

    /// The session uuids in a tab, left to right, top to bottom.
    pub fn tab_sessions(&self, tab: &str) -> Option<Vec<String>> {
        let spot = self.find_tab(tab)?;
        Some(self.tab(spot).split.leaves())
    }

    /// A session's program ended. A clean exit takes its pane out, and closes the tab when it
    /// was the last pane, with no entry for `ReopenTab`. A failed program stays on screen so
    /// its error can be read. True when the tree changed.
    pub fn on_exit(&mut self, session: &str, clean: bool) -> bool {
        if !clean {
            return false;
        }
        let Some(spot) = self.find_session(session) else {
            return false;
        };
        self.drop_pane(spot, session);
        self.tidy();
        true
    }

    pub fn apply(&mut self, op: Op) -> Result<Effects, TreeError> {
        let project = self.folder_for(&op).map(|folder| project::resolve(&folder));
        self.apply_in(op, project)
    }

    /// The folder whose project `apply_in` wants for this op: the folder to open, or where the
    /// first pane of the tab `ReopenTab` brings back was. None for every other op.
    pub fn folder_for(&self, op: &Op) -> Option<PathBuf> {
        match op {
            Op::OpenFolder { path } => Some(path.clone()),
            Op::ReopenTab => {
                let closed = self.closed.last()?;
                let leaves = closed.tab.split.leaves();
                leaves
                    .iter()
                    .find_map(|session| closed.panes.iter().find(|pane| &pane.uuid == session))
                    .map(|pane| pane.cwd.clone())
            }
            _ => None,
        }
    }

    /// `apply` with the project of `folder_for(&op)` already resolved. Touches no disk. A
    /// missing or stale project (the closed list changed since `folder_for`) falls back to a
    /// group for the folder itself.
    pub fn apply_in(&mut self, op: Op, project: Option<Project>) -> Result<Effects, TreeError> {
        let mut effects = Effects::default();
        match op {
            Op::OpenFolder { path } => {
                let project = project.unwrap_or_else(|| folder_only(&path));
                if self.group_for(&project).is_none() {
                    self.create_group(&project);
                }
                effects.spawn.push(SavedSession {
                    uuid: String::new(),
                    kind: "shell".to_string(),
                    cwd: path,
                    title: None,
                    chat: None,
                });
            }
            // With no panes nothing can come back, so no entry is kept. The server closes
            // through `close_tab` with the panes it read from its sessions.
            Op::CloseTab { tab } => return self.close_tab(&tab, Vec::new()),
            Op::ReopenTab => {
                let ClosedTab {
                    mut tab,
                    group,
                    index,
                    panes,
                } = self.closed.pop().ok_or(TreeError::BadOp)?;
                let pane = |session: &str| panes.iter().find(|pane| pane.uuid == session);
                tab.split = tab
                    .split
                    .retain(&|session| pane(session).is_some())
                    .ok_or(TreeError::BadOp)?;
                for session in tab.split.leaves() {
                    let Some(mut saved) = pane(&session).cloned() else {
                        continue;
                    };
                    // A Claude pane resumes its chat; a shell starts fresh in its folder.
                    if saved.kind == "shell" {
                        saved.title = None;
                        saved.chat = None;
                    }
                    effects.spawn.push(saved);
                }
                let group = match group {
                    Some(key) => Some(match self.group_index(&key) {
                        Some(index) => index,
                        None => {
                            let cwd = effects.spawn.first().map(|saved| saved.cwd.clone());
                            let cwd = cwd.as_deref().unwrap_or(Path::new(""));
                            self.recreate_group(&key, cwd, project)
                        }
                    }),
                    None => None,
                };
                let tabs = self.tabs_mut(group);
                tabs.insert(index.min(tabs.len()), tab);
            }
            Op::MoveTab { tab, group, index } => {
                let spot = self.find_tab(&tab).ok_or(TreeError::NoTab)?;
                let dest = match &group {
                    Some(key) => Some(self.group_index(key).ok_or(TreeError::NoGroup)?),
                    None => None,
                };
                let mut tab = self.tabs_mut(spot.group).remove(spot.index);
                tab.pinned = true;
                let tabs = self.tabs_mut(dest);
                tabs.insert(index.min(tabs.len()), tab);
                self.tidy();
            }
            Op::MoveGroup { group, index } => {
                let from = self.group_index(&group).ok_or(TreeError::NoGroup)?;
                let group = self.groups.remove(from);
                let index = index.min(self.groups.len());
                self.groups.insert(index, group);
            }
            Op::RenameTab { tab, name } => {
                let spot = self.find_tab(&tab).ok_or(TreeError::NoTab)?;
                self.tab_mut(spot).name = name;
            }
            Op::RenameGroup { group, name } => {
                let index = self.group_index(&group).ok_or(TreeError::NoGroup)?;
                self.groups[index].name = name;
            }
            Op::PinTab { tab, pinned } => {
                let spot = self.find_tab(&tab).ok_or(TreeError::NoTab)?;
                self.tab_mut(spot).pinned = pinned;
            }
            Op::CollapseGroup { group, collapsed } => {
                let index = self.group_index(&group).ok_or(TreeError::NoGroup)?;
                self.groups[index].collapsed = collapsed;
            }
            Op::JoinTabs { tab, into, dir } => {
                if tab == into {
                    return Err(TreeError::BadOp);
                }
                let from = self.find_tab(&tab).ok_or(TreeError::NoTab)?;
                self.find_tab(&into).ok_or(TreeError::NoTab)?;
                let moved = self.tabs_mut(from.group).remove(from.index);
                let spot = self.find_tab(&into).ok_or(TreeError::NoTab)?;
                let target = self.tab_mut(spot);
                let split = std::mem::replace(&mut target.split, leaf(""));
                target.split = node(dir, vec![split, moved.split], vec![0.5, 0.5]);
                self.tidy();
            }
            Op::SplitOut { session } => {
                let spot = self.find_session(&session).ok_or(TreeError::NoTab)?;
                if self.tab(spot).split.leaves().len() < 2 {
                    return Err(TreeError::BadOp);
                }
                self.drop_pane(spot, &session);
                let id = self.next_id();
                self.tabs_mut(spot.group).insert(
                    spot.index + 1,
                    Tab {
                        id,
                        name: None,
                        pinned: false,
                        split: leaf(&session),
                        last_used: 0,
                    },
                );
            }
            Op::SetSizes { tab, split } => {
                let spot = self.find_tab(&tab).ok_or(TreeError::NoTab)?;
                let mut had = self.tab(spot).split.leaves();
                let mut has = split.leaves();
                had.sort();
                has.sort();
                if had != has || !split.is_valid() {
                    return Err(TreeError::BadOp);
                }
                self.tab_mut(spot).split = split;
            }
            Op::Used { tab } => {
                let spot = self.find_tab(&tab).ok_or(TreeError::NoTab)?;
                let newest = self.all_tabs().map(|tab| tab.last_used).max().unwrap_or(0);
                self.tab_mut(spot).last_used = newest + 1;
            }
        }
        Ok(effects)
    }

    /// Drops every leaf whose session `keep` turns down, then every tab left with no leaf and
    /// every group left with no tab, as on load for sessions that did not come back. Closed
    /// tabs are left alone, since their sessions are gone by design. True when anything went.
    pub fn retain_sessions(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let mut changed = false;
        let lists = self
            .groups
            .iter_mut()
            .map(|group| &mut group.tabs)
            .chain(std::iter::once(&mut self.ungrouped));
        for tabs in lists {
            tabs.retain_mut(|tab| {
                if tab.split.leaves().iter().all(|session| keep(session)) {
                    return true;
                }
                changed = true;
                let split = std::mem::replace(&mut tab.split, leaf(""));
                match split.retain(&keep) {
                    Some(split) => {
                        tab.split = split;
                        true
                    }
                    None => false,
                }
            });
        }
        self.tidy() || changed
    }

    fn tab(&self, spot: Spot) -> &Tab {
        match spot.group {
            Some(group) => &self.groups[group].tabs[spot.index],
            None => &self.ungrouped[spot.index],
        }
    }

    fn tab_mut(&mut self, spot: Spot) -> &mut Tab {
        &mut self.tabs_mut(spot.group)[spot.index]
    }

    fn tabs_mut(&mut self, group: Option<usize>) -> &mut Vec<Tab> {
        match group {
            Some(group) => &mut self.groups[group].tabs,
            None => &mut self.ungrouped,
        }
    }

    fn all_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.groups
            .iter()
            .flat_map(|group| group.tabs.iter())
            .chain(self.ungrouped.iter())
    }

    fn find(&self, wanted: impl Fn(&Tab) -> bool) -> Option<Spot> {
        for (group_index, group) in self.groups.iter().enumerate() {
            if let Some(index) = group.tabs.iter().position(&wanted) {
                return Some(Spot {
                    group: Some(group_index),
                    index,
                });
            }
        }
        let index = self.ungrouped.iter().position(&wanted)?;
        Some(Spot { group: None, index })
    }

    fn find_tab(&self, id: &str) -> Option<Spot> {
        self.find(|tab| tab.id == id)
    }

    fn find_session(&self, session: &str) -> Option<Spot> {
        self.find(|tab| tab.split.contains(session))
    }

    /// Gives each folder group saved when folder keys held only the name (`folder:<name>`)
    /// the key of its stored path. A closed tab that names such a key gets the key of the
    /// folder with that name above its first pane's folder, else of that folder, as a reopen
    /// used to pick. Defaults kept under such a key name no folder, so they are dropped.
    pub fn upgrade_folder_keys(&mut self) {
        for group in &mut self.groups {
            if is_name_key(&group.key) {
                group.key = project::folder_key(&group.path);
            }
        }
        for closed in &mut self.closed {
            let Some(name) = closed
                .group
                .as_deref()
                .filter(|key| is_name_key(key))
                .and_then(|key| key.strip_prefix("folder:"))
            else {
                continue;
            };
            let first = closed
                .tab
                .split
                .leaves()
                .into_iter()
                .find_map(|session| closed.panes.iter().find(|pane| pane.uuid == session));
            let Some(cwd) = first.map(|pane| pane.cwd.as_path()) else {
                continue;
            };
            let root = cwd
                .ancestors()
                .find(|folder| folder.file_name().is_some_and(|found| found == name))
                .unwrap_or(cwd);
            closed.group = Some(project::folder_key(root));
        }
        self.defaults.retain(|key, _| !is_name_key(key));
    }

    fn group_index(&self, key: &str) -> Option<usize> {
        self.groups.iter().position(|group| group.key == key)
    }

    /// The group a project belongs in. A remote key matches wherever the repo is cloned; a
    /// folder key holds the folder's whole path, so it only matches the same folder.
    fn group_for(&self, project: &Project) -> Option<usize> {
        let by_remote = project.key.starts_with("remote:");
        self.groups
            .iter()
            .position(|group| group.key == project.key && (by_remote || group.path == project.root))
    }

    /// A new empty group for the project, placed before the first group whose folder sorts
    /// after it, with any defaults it had before. Returns its index.
    fn create_group(&mut self, project: &Project) -> usize {
        let defaults = self.defaults.get(&project.key).cloned().unwrap_or_default();
        let index = self
            .groups
            .iter()
            .position(|group| group.path > project.root)
            .unwrap_or(self.groups.len());
        self.groups.insert(
            index,
            Group {
                key: project.key.clone(),
                path: project.root.clone(),
                name: defaults.name,
                branch: project.branch.clone(),
                collapsed: defaults.collapsed,
                tabs: Vec::new(),
            },
        );
        index
    }

    /// Brings back a group for a reopened tab, from the folder its first pane was last in.
    /// When that folder resolves to another project (the shell had left for a folder that is
    /// no group), a folder group comes back at the folder its key names, and a remote one at
    /// the folder itself. `found` is that folder's project, resolved by the caller; None or
    /// another project's means the folder is used as is.
    fn recreate_group(&mut self, key: &str, cwd: &Path, found: Option<Project>) -> usize {
        let project = if let Some(found) = found.filter(|found| found.key == key) {
            found
        } else {
            let root = key.strip_prefix("folder:").map_or(cwd, Path::new);
            Project {
                key: key.to_string(),
                root: root.to_path_buf(),
                is_git: false,
                branch: None,
            }
        };
        self.create_group(&project)
    }

    /// The group of the tab named by the newest `Used` op, or None when that tab is
    /// ungrouped or no tab was used.
    fn last_used_group(&self) -> Option<usize> {
        let newest = self
            .all_tabs()
            .filter(|tab| tab.last_used > 0)
            .max_by_key(|tab| tab.last_used)?;
        self.find_tab(&newest.id)?.group
    }

    /// `t<n>`, one past the highest in use, counting closed tabs so a reopened tab keeps a
    /// unique id.
    fn next_id(&self) -> String {
        let highest = self
            .all_tabs()
            .chain(self.closed.iter().map(|closed| &closed.tab))
            .filter_map(|tab| tab.id.strip_prefix('t')?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        format!("t{}", highest + 1)
    }

    /// Takes a pane out of its tab, and the tab out of the tree when it was the last pane.
    /// The caller tidies away a group this empties.
    fn drop_pane(&mut self, spot: Spot, session: &str) {
        let tab = self.tab_mut(spot);
        let split = std::mem::replace(&mut tab.split, leaf(""));
        match split.retain(&|leaf| leaf != session) {
            Some(split) => tab.split = split,
            None => {
                self.tabs_mut(spot.group).remove(spot.index);
            }
        }
    }

    /// Removes every group with no tabs, keeping its name and collapsed state in `defaults`.
    /// True when one went.
    fn tidy(&mut self) -> bool {
        let before = self.groups.len();
        let defaults = &mut self.defaults;
        self.groups.retain(|group| {
            if !group.tabs.is_empty() {
                return true;
            }
            defaults.insert(
                group.key.clone(),
                GroupDefaults {
                    name: group.name.clone(),
                    collapsed: group.collapsed,
                },
            );
            false
        });
        self.groups.len() != before
    }
}

impl Split {
    /// Session uuids left to right, top to bottom. The first is the pane whose folder is the
    /// tab's folder.
    pub fn leaves(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.push_leaves(&mut out);
        out
    }

    fn push_leaves(&self, out: &mut Vec<String>) {
        match self {
            Split::Leaf { session } => out.push(session.clone()),
            Split::Row { children, .. } | Split::Col { children, .. } => {
                for child in children {
                    child.push_leaves(out);
                }
            }
        }
    }

    fn first(&self) -> &str {
        match self {
            Split::Leaf { session } => session,
            Split::Row { children, .. } | Split::Col { children, .. } => {
                children.first().map_or("", Split::first)
            }
        }
    }

    fn contains(&self, session: &str) -> bool {
        match self {
            Split::Leaf { session: own } => own == session,
            Split::Row { children, .. } | Split::Col { children, .. } => {
                children.iter().any(|child| child.contains(session))
            }
        }
    }

    /// The split with only the leaves `keep` accepts, or None when none are left. A node left
    /// with one child becomes that child, and the shares of what is left are scaled back up to
    /// fill the space.
    fn retain(self, keep: &dyn Fn(&str) -> bool) -> Option<Split> {
        let (row, children, sizes) = match self {
            Split::Leaf { session } => return keep(&session).then_some(Split::Leaf { session }),
            Split::Row { children, sizes } => (true, children, sizes),
            Split::Col { children, sizes } => (false, children, sizes),
        };
        let sizes = shares(children.len(), sizes);
        let before = children.len();
        let mut kept = Vec::new();
        let mut kept_sizes = Vec::new();
        for (child, size) in children.into_iter().zip(sizes) {
            if let Some(child) = child.retain(keep) {
                kept.push(child);
                kept_sizes.push(size);
            }
        }
        match kept.len() {
            0 => None,
            1 => kept.pop(),
            count => {
                if count != before {
                    let total: f32 = kept_sizes.iter().sum();
                    for size in &mut kept_sizes {
                        *size /= total;
                    }
                }
                let dir = if row { Dir::Right } else { Dir::Down };
                Some(node(dir, kept, kept_sizes))
            }
        }
    }

    /// Puts a new leaf beside the leaf for `target`, splitting its space in half. Inside a
    /// node that already runs that way the new leaf joins it; otherwise the target leaf
    /// becomes a two-pane node. False when there is no such leaf.
    fn split_leaf(&mut self, target: &str, new: &str, dir: Dir) -> bool {
        let same = matches!(
            (&*self, dir),
            (Split::Row { .. }, Dir::Right) | (Split::Col { .. }, Dir::Down)
        );
        match self {
            Split::Leaf { session } => {
                if session != target {
                    return false;
                }
                let old = leaf(session);
                *self = node(dir, vec![old, leaf(new)], vec![0.5, 0.5]);
                true
            }
            Split::Row { children, sizes } | Split::Col { children, sizes } => {
                let at = children.iter().position(
                    |child| matches!(child, Split::Leaf { session } if session == target),
                );
                if let (true, Some(at)) = (same, at) {
                    *sizes = shares(children.len(), std::mem::take(sizes));
                    let half = sizes[at] / 2.0;
                    sizes[at] = half;
                    sizes.insert(at + 1, half);
                    children.insert(at + 1, leaf(new));
                    return true;
                }
                children
                    .iter_mut()
                    .any(|child| child.split_leaf(target, new, dir))
            }
        }
    }

    /// Every node has at least two children, one positive share per child. A node may hold
    /// one running the same way, since `retain` can leave one when it lifts a lone child.
    fn is_valid(&self) -> bool {
        match self {
            Split::Leaf { .. } => true,
            Split::Row { children, sizes } | Split::Col { children, sizes } => {
                children.len() >= 2
                    && children.len() == sizes.len()
                    && sizes.iter().all(|size| size.is_finite() && *size > 0.0)
                    && children.iter().all(Split::is_valid)
            }
        }
    }
}

/// True for a folder key saved when they held only the folder's name: `folder:` and no
/// absolute path after it.
fn is_name_key(key: &str) -> bool {
    key.strip_prefix("folder:")
        .is_some_and(|rest| !Path::new(rest).is_absolute())
}

/// A group for the folder itself, for when no resolved project was handed in.
fn folder_only(folder: &Path) -> Project {
    Project {
        key: project::folder_key(folder),
        root: folder.to_path_buf(),
        is_git: false,
        branch: None,
    }
}

fn leaf(session: &str) -> Split {
    Split::Leaf {
        session: session.to_string(),
    }
}

fn node(dir: Dir, children: Vec<Split>, sizes: Vec<f32>) -> Split {
    match dir {
        Dir::Right => Split::Row { children, sizes },
        Dir::Down => Split::Col { children, sizes },
    }
}

/// The sizes as given when there is one usable share per child, else equal shares, so a
/// hand-edited state file cannot make the split code index past the end.
fn shares(count: usize, sizes: Vec<f32>) -> Vec<f32> {
    let usable = sizes.len() == count && sizes.iter().all(|size| size.is_finite() && *size > 0.0);
    if usable {
        sizes
    } else {
        vec![1.0 / count.max(1) as f32; count]
    }
}
