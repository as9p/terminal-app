//! The tree of groups and tabs: where a tab lands, when it moves, and what closing and
//! reopening do. Pure logic, so these only touch small temp folders that stand in for repos.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use term_host::project::Project;
use term_host::state::SavedSession;
use term_host::tree::{Dir, Effects, GroupDefaults, Op, Place, Split, Tab, Tree, TreeError};
use uuid::Uuid;

const CHAT: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);

/// A temp folder for one test, removed on drop. Its path has every link resolved, as the
/// session code resolves reported folders before they reach the tree.
struct Root(PathBuf);

impl Root {
    fn new() -> Root {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("term-host-tree-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).expect("make the test folder");
        Root(fs::canonicalize(&path).expect("resolve the test folder"))
    }

    /// A folder with no git.
    fn plain(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).expect("make a plain folder");
        path
    }

    /// A repo on branch `main` whose origin is `https://github.com/owner/<name>.git`.
    fn repo(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        let git = path.join(".git");
        fs::create_dir_all(&git).expect("make a repo");
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").expect("write HEAD");
        fs::write(
            git.join("config"),
            format!("[remote \"origin\"]\n\turl = https://github.com/owner/{name}.git\n"),
        )
        .expect("write config");
        path
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn shell(tree: &mut Tree, session: &str, cwd: &Path) -> String {
    tree.add_session(session, Place::NewTab, cwd, true)
}

fn split_of(tree: &mut Tree, session: &str, of: &str, dir: Dir, cwd: &Path) -> String {
    let place = Place::SplitOf {
        session: of.to_string(),
        dir,
    };
    tree.add_session(session, place, cwd, true)
}

fn ok(tree: &mut Tree, op: Op) -> Effects {
    tree.apply(op).expect("the op applies")
}

fn leaf(session: &str) -> Split {
    Split::Leaf {
        session: session.to_string(),
    }
}

/// The ids of each group's tabs, then the ungrouped ones under the key "".
fn layout(tree: &Tree) -> Vec<(String, Vec<String>)> {
    let ids = |tabs: &[Tab]| tabs.iter().map(|tab| tab.id.clone()).collect::<Vec<_>>();
    let mut out: Vec<_> = tree
        .groups
        .iter()
        .map(|group| (group.key.clone(), ids(&group.tabs)))
        .collect();
    out.push((String::new(), ids(&tree.ungrouped)));
    out
}

fn tab<'a>(tree: &'a Tree, id: &str) -> &'a Tab {
    tree.groups
        .iter()
        .flat_map(|group| group.tabs.iter())
        .chain(tree.ungrouped.iter())
        .find(|tab| tab.id == id)
        .expect("the tab is in the tree")
}

fn saved(uuid: &str, kind: &str, cwd: &Path, chat: Option<Uuid>) -> SavedSession {
    SavedSession {
        uuid: uuid.to_string(),
        kind: kind.to_string(),
        cwd: cwd.to_path_buf(),
        title: Some(format!("{uuid} title")),
        chat,
    }
}

#[test]
fn cd_into_repo_moves_tab_and_creates_group() {
    let root = Root::new();
    let plain = root.plain("plain");
    let app = root.repo("app");
    let mut tree = Tree::default();
    let id = shell(&mut tree, "a", &plain);
    assert_eq!(layout(&tree), [(String::new(), vec![id.clone()])]);

    assert!(tree.on_cwd("a", &app.join("src")));
    assert_eq!(tree.groups.len(), 1);
    let group = &tree.groups[0];
    assert_eq!(group.key, "remote:github.com/owner/app");
    assert_eq!(group.path, app);
    assert_eq!(group.branch.as_deref(), Some("main"));
    assert_eq!(group.tabs.len(), 1);
    assert_eq!(group.tabs[0].id, id);
    assert!(tree.ungrouped.is_empty());

    // A second report from the same repo changes nothing.
    assert!(!tree.on_cwd("a", &app));
}

#[test]
fn cd_out_to_plain_folder_stays() {
    let root = Root::new();
    let plain = root.plain("plain");
    let app = root.repo("app");
    let mut tree = Tree::default();
    let id = shell(&mut tree, "a", &app);
    let before = layout(&tree);
    assert_eq!(
        before[0],
        ("remote:github.com/owner/app".to_string(), vec![id])
    );

    assert!(!tree.on_cwd("a", &plain));
    assert_eq!(layout(&tree), before);
}

#[test]
fn pinned_tab_never_moves() {
    let root = Root::new();
    let plain = root.plain("plain");
    let app = root.repo("app");
    let mut tree = Tree::default();
    let id = shell(&mut tree, "a", &plain);
    ok(
        &mut tree,
        Op::PinTab {
            tab: id.clone(),
            pinned: true,
        },
    );

    assert!(!tree.on_cwd("a", &app));
    assert!(tree.groups.is_empty());
    assert_eq!(layout(&tree), [(String::new(), vec![id.clone()])]);

    ok(
        &mut tree,
        Op::PinTab {
            tab: id.clone(),
            pinned: false,
        },
    );
    assert!(tree.on_cwd("a", &app));
    assert_eq!(tree.groups[0].tabs[0].id, id);
}

#[test]
fn hand_move_pins() {
    let root = Root::new();
    let app = root.repo("app");
    let web = root.repo("web");
    let mut tree = Tree::default();
    let first = shell(&mut tree, "a", &app);
    let second = shell(&mut tree, "b", &web);
    assert!(!tab(&tree, &first).pinned);

    ok(
        &mut tree,
        Op::MoveTab {
            tab: first.clone(),
            group: Some("remote:github.com/owner/web".to_string()),
            index: 0,
        },
    );
    assert!(tab(&tree, &first).pinned);
    assert_eq!(
        layout(&tree),
        [
            (
                "remote:github.com/owner/web".to_string(),
                vec![first.clone(), second]
            ),
            (String::new(), vec![]),
        ]
    );

    // Back in its old repo, it stays where it was put.
    assert!(!tree.on_cwd("a", &app));
    assert_eq!(tree.groups.len(), 1);
}

#[test]
fn new_tab_joins_current_group_when_folder_is_no_group() {
    let root = Root::new();
    let plain = root.plain("plain");
    let app = root.repo("app");
    let web = root.repo("web");
    let mut tree = Tree::default();
    let first = shell(&mut tree, "a", &app);
    let other = shell(&mut tree, "b", &web);
    ok(&mut tree, Op::Used { tab: other.clone() });
    ok(&mut tree, Op::Used { tab: first.clone() });

    let new = shell(&mut tree, "c", &plain);
    assert_eq!(
        layout(&tree),
        [
            ("remote:github.com/owner/app".to_string(), vec![first, new]),
            ("remote:github.com/owner/web".to_string(), vec![other]),
            (String::new(), vec![]),
        ]
    );
}

#[test]
fn new_tab_with_no_group_is_ungrouped() {
    let root = Root::new();
    let plain = root.plain("plain");
    let app = root.repo("app");
    let mut tree = Tree::default();
    let first = shell(&mut tree, "a", &plain);
    assert!(tree.groups.is_empty());
    assert_eq!(layout(&tree), [(String::new(), vec![first.clone()])]);

    // A group exists, but no tab in it was used, so a plain folder still lands ungrouped.
    let grouped = shell(&mut tree, "b", &app);
    let second = shell(&mut tree, "c", &plain);
    assert_eq!(
        layout(&tree),
        [
            ("remote:github.com/owner/app".to_string(), vec![grouped]),
            (String::new(), vec![first.clone(), second.clone()]),
        ]
    );

    // The last used tab is ungrouped, so the next one is too.
    ok(&mut tree, Op::Used { tab: second });
    let third = shell(&mut tree, "d", &plain);
    assert_eq!(tree.ungrouped.last().map(|tab| tab.id.clone()), Some(third));
}

#[test]
fn open_folder_makes_group_for_non_git() {
    let root = Root::new();
    let notes = root.plain("notes");
    let elsewhere = root.plain("elsewhere");
    let mut tree = Tree::default();
    let effects = ok(
        &mut tree,
        Op::OpenFolder {
            path: notes.clone(),
        },
    );
    assert!(effects.kill.is_empty());
    assert_eq!(effects.spawn.len(), 1);
    let spawn = &effects.spawn[0];
    assert_eq!(spawn.kind, "shell");
    assert_eq!(spawn.cwd, notes);
    assert_eq!(spawn.chat, None);
    assert_eq!(tree.groups.len(), 1);
    let group = &tree.groups[0];
    assert_eq!(group.key, "folder:notes");
    assert_eq!(group.path, notes);
    assert_eq!(group.branch, None);

    // The shell the effect asks for lands in the new group.
    let id = shell(&mut tree, "s", &notes);
    assert_eq!(tree.groups[0].tabs[0].id, id);

    // A tab elsewhere that moves into the folder joins the group too.
    let other = shell(&mut tree, "t", &elsewhere);
    assert!(tree.ungrouped.iter().any(|tab| tab.id == other));
    assert!(tree.on_cwd("t", &notes));
    assert_eq!(
        layout(&tree)[0],
        ("folder:notes".to_string(), vec![id, other])
    );
}

#[test]
fn last_tab_out_removes_group_and_keeps_defaults() {
    let root = Root::new();
    let app = root.repo("app");
    let key = "remote:github.com/owner/app".to_string();
    let mut tree = Tree::default();
    let id = shell(&mut tree, "a", &app);
    ok(
        &mut tree,
        Op::RenameGroup {
            group: key.clone(),
            name: Some("App".to_string()),
        },
    );
    ok(
        &mut tree,
        Op::CollapseGroup {
            group: key.clone(),
            collapsed: true,
        },
    );

    ok(
        &mut tree,
        Op::MoveTab {
            tab: id.clone(),
            group: None,
            index: 0,
        },
    );
    assert!(tree.groups.is_empty());
    assert_eq!(layout(&tree), [(String::new(), vec![id])]);
    assert_eq!(
        tree.defaults.get(&key),
        Some(&GroupDefaults {
            name: Some("App".to_string()),
            collapsed: true,
        })
    );
}

#[test]
fn group_returns_with_its_name() {
    let root = Root::new();
    let app = root.repo("app");
    let key = "remote:github.com/owner/app".to_string();
    let mut tree = Tree::default();
    let id = shell(&mut tree, "a", &app);
    ok(
        &mut tree,
        Op::RenameGroup {
            group: key.clone(),
            name: Some("App".to_string()),
        },
    );
    ok(&mut tree, Op::CloseTab { tab: id });
    assert!(tree.groups.is_empty());

    shell(&mut tree, "b", &app);
    assert_eq!(tree.groups.len(), 1);
    assert_eq!(tree.groups[0].key, key);
    assert_eq!(tree.groups[0].name.as_deref(), Some("App"));
    assert!(!tree.groups[0].collapsed);
}

#[test]
fn new_group_sorted_by_folder() {
    let root = Root::new();
    let a = root.repo("a");
    let b = root.repo("b");
    let c = root.repo("c");
    let d = root.repo("d");
    let mut tree = Tree::default();
    shell(&mut tree, "1", &b);
    shell(&mut tree, "2", &d);
    shell(&mut tree, "3", &a);
    let paths = |tree: &Tree| {
        tree.groups
            .iter()
            .map(|g| g.path.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(paths(&tree), [a.clone(), b.clone(), d.clone()]);

    // The order Alex drags them into stays, and a new group goes before the first greater one.
    ok(
        &mut tree,
        Op::MoveGroup {
            group: "remote:github.com/owner/a".to_string(),
            index: 2,
        },
    );
    assert_eq!(paths(&tree), [b.clone(), d.clone(), a.clone()]);
    shell(&mut tree, "4", &c);
    assert_eq!(paths(&tree), [b, c, d, a]);
}

#[test]
fn close_tab_kills_every_pane() {
    let root = Root::new();
    let app = root.repo("app");
    let mut tree = Tree::default();
    let keep = shell(&mut tree, "x", &app);
    let id = shell(&mut tree, "a", &app);
    assert_eq!(split_of(&mut tree, "b", "a", Dir::Right, &app), id);
    assert_eq!(split_of(&mut tree, "c", "b", Dir::Down, &app), id);
    assert_eq!(tab(&tree, &id).split.leaves(), ["a", "b", "c"]);

    // The panes go in with the close itself, so no reopen can find the entry without them.
    let panes = vec![
        saved("a", "shell", &app, None),
        saved("c", "shell", &app, None),
    ];
    let effects = tree.close_tab(&id, panes.clone()).expect("close");
    assert_eq!(effects.kill, ["a", "b", "c"]);
    assert!(effects.spawn.is_empty());
    assert_eq!(layout(&tree)[0].1, [keep]);
    assert_eq!(tree.closed.len(), 1);
    let closed = &tree.closed[0];
    assert_eq!(closed.tab.id, id);
    assert_eq!(closed.group.as_deref(), Some("remote:github.com/owner/app"));
    assert_eq!(closed.index, 1);
    assert_eq!(closed.panes, panes);
}

#[test]
fn close_with_nothing_to_bring_back_leaves_no_entry() {
    let root = Root::new();
    let plain = root.plain("plain");
    let mut tree = Tree::default();
    let kept = shell(&mut tree, "k", &plain);
    let empty = shell(&mut tree, "e", &plain);
    let other = shell(&mut tree, "o", &plain);
    tree.close_tab(&kept, vec![saved("k", "shell", &plain, None)])
        .expect("close");
    // Neither a close with no panes nor one whose panes match no leaf can be reopened.
    ok(&mut tree, Op::CloseTab { tab: empty });
    tree.close_tab(&other, vec![saved("z", "shell", &plain, None)])
        .expect("close");
    assert_eq!(tree.closed.len(), 1);

    let effects = ok(&mut tree, Op::ReopenTab);
    assert_eq!(effects.spawn[0].uuid, "k");
    assert_eq!(layout(&tree)[0].1, [kept]);
}

#[test]
fn resolved_project_needs_no_disk() {
    let mut tree = Tree::default();
    let gone = Path::new("/no/such/folder");
    let repo = Project {
        key: "remote:example.com/owner/repo".to_string(),
        root: gone.to_path_buf(),
        is_git: true,
        branch: Some("main".to_string()),
    };
    let plain = Project {
        key: "folder:else".to_string(),
        root: gone.join("else"),
        is_git: false,
        branch: None,
    };
    tree.add_session_in("a", Place::NewTab, &plain, true);
    assert!(tree.groups.is_empty());
    assert!(tree.on_cwd_in("a", &repo));
    assert_eq!(tree.groups[0].key, repo.key);
    assert_eq!(tree.groups[0].branch.as_deref(), Some("main"));
    let effects = tree
        .apply_in(
            Op::OpenFolder {
                path: gone.join("else"),
            },
            Some(plain.clone()),
        )
        .expect("open");
    assert_eq!(effects.spawn[0].cwd, gone.join("else"));
    assert_eq!(tree.groups.len(), 2);
}

#[test]
fn clean_exit_of_last_pane_closes_tab_without_reopen_entry() {
    let root = Root::new();
    let app = root.repo("app");
    let mut tree = Tree::default();
    let id = shell(&mut tree, "a", &app);
    split_of(&mut tree, "b", "a", Dir::Right, &app);

    assert!(tree.on_exit("b", true));
    assert_eq!(tab(&tree, &id).split, leaf("a"));

    // A failed program stays on screen so its error can be read.
    assert!(!tree.on_exit("a", false));
    assert_eq!(tab(&tree, &id).split, leaf("a"));

    assert!(tree.on_exit("a", true));
    assert!(tree.groups.is_empty());
    assert!(tree.ungrouped.is_empty());
    assert!(tree.closed.is_empty());
    assert!(!tree.on_exit("a", true));
}

#[test]
fn reopen_restores_place_and_resumes_claude() {
    let root = Root::new();
    let app = root.repo("app");
    let key = "remote:github.com/owner/app".to_string();
    let mut tree = Tree::default();
    let first = shell(&mut tree, "x", &app);
    let second = tree.add_session("c", Place::NewTab, &app, false);
    split_of(&mut tree, "s", "c", Dir::Down, &app);
    ok(
        &mut tree,
        Op::RenameGroup {
            group: key.clone(),
            name: Some("App".to_string()),
        },
    );

    // The server fills in each closed tab's panes from its sessions, in the order of `kill`.
    let panes = vec![
        saved("c", "claude", &app, Some(CHAT)),
        saved("s", "shell", &app.join("src"), None),
    ];
    tree.close_tab(&second, panes).expect("close");
    tree.close_tab(&first, vec![saved("x", "shell", &app, None)])
        .expect("close");
    assert!(tree.groups.is_empty());

    let effects = ok(&mut tree, Op::ReopenTab);
    assert!(effects.kill.is_empty());
    assert_eq!(effects.spawn.len(), 1);
    assert_eq!(effects.spawn[0].uuid, "x");
    assert_eq!(effects.spawn[0].kind, "shell");
    assert_eq!(effects.spawn[0].cwd, app);
    assert_eq!(effects.spawn[0].title, None);
    assert_eq!(tree.groups.len(), 1);
    assert_eq!(tree.groups[0].path, app);
    assert_eq!(tree.groups[0].name.as_deref(), Some("App"));

    let effects = ok(&mut tree, Op::ReopenTab);
    assert_eq!(effects.spawn.len(), 2);
    let claude = &effects.spawn[0];
    assert_eq!(claude.uuid, "c");
    assert_eq!(claude.kind, "claude");
    assert_eq!(claude.chat, Some(CHAT));
    let fresh = &effects.spawn[1];
    assert_eq!(fresh.uuid, "s");
    assert_eq!(fresh.kind, "shell");
    assert_eq!(fresh.cwd, app.join("src"));
    assert_eq!(fresh.title, None);
    assert_eq!(fresh.chat, None);

    assert_eq!(layout(&tree)[0], (key, vec![first, second.clone()]));
    assert_eq!(tab(&tree, &second).split.leaves(), ["c", "s"]);
    assert!(tree.closed.is_empty());
    assert_eq!(tree.apply(Op::ReopenTab), Err(TreeError::BadOp));
}

#[test]
fn closed_list_caps_at_20() {
    let root = Root::new();
    let plain = root.plain("plain");
    let mut tree = Tree::default();
    let ids: Vec<String> = (0..25)
        .map(|n| shell(&mut tree, &format!("s{n}"), &plain))
        .collect();
    for (n, id) in ids.iter().enumerate() {
        let pane = saved(&format!("s{n}"), "shell", &plain, None);
        tree.close_tab(id, vec![pane]).expect("close");
    }
    assert_eq!(tree.closed.len(), 20);
    assert_eq!(tree.closed[0].tab.id, ids[5]);
    assert_eq!(tree.closed[19].tab.id, ids[24]);
}

#[test]
fn join_then_split_out_round_trips() {
    let root = Root::new();
    let app = root.repo("app");
    let mut tree = Tree::default();
    let a = shell(&mut tree, "a", &app);
    let b = shell(&mut tree, "b", &app);
    let c = shell(&mut tree, "c", &app);

    ok(
        &mut tree,
        Op::JoinTabs {
            tab: b.clone(),
            into: a.clone(),
            dir: Dir::Right,
        },
    );
    assert_eq!(layout(&tree)[0].1, [a.clone(), c.clone()]);
    assert_eq!(
        tab(&tree, &a).split,
        Split::Row {
            children: vec![leaf("a"), leaf("b")],
            sizes: vec![0.5, 0.5],
        }
    );

    ok(
        &mut tree,
        Op::SplitOut {
            session: "b".to_string(),
        },
    );
    let ids = &layout(&tree)[0].1;
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], a);
    assert_eq!(ids[2], c);
    assert_eq!(tab(&tree, &a).split, leaf("a"));
    assert_eq!(tab(&tree, &ids[1]).split, leaf("b"));

    // A pane that is already alone in its tab cannot be split out.
    assert_eq!(
        tree.apply(Op::SplitOut {
            session: "a".to_string()
        }),
        Err(TreeError::BadOp)
    );
}

#[test]
fn tree_round_trips_json() {
    let root = Root::new();
    let app = root.repo("app");
    let plain = root.plain("plain");
    let mut tree = Tree::default();
    let a = shell(&mut tree, "a", &app);
    split_of(&mut tree, "b", "a", Dir::Down, &app);
    let gone = shell(&mut tree, "c", &plain);
    shell(&mut tree, "d", &plain);
    ok(
        &mut tree,
        Op::RenameTab {
            tab: a.clone(),
            name: Some("mine".to_string()),
        },
    );
    ok(&mut tree, Op::Used { tab: a });
    tree.close_tab(&gone, vec![saved("c", "shell", &plain, None)])
        .expect("close");
    tree.defaults.insert(
        "folder:old".to_string(),
        GroupDefaults {
            name: Some("Old".to_string()),
            collapsed: true,
        },
    );

    let text = serde_json::to_string(&tree).expect("serialize");
    assert!(text.contains(r#""type":"col""#), "{text}");
    assert!(text.contains(r#""type":"leaf""#), "{text}");
    assert!(text.contains(r#""last_used""#), "{text}");
    let back: Tree = serde_json::from_str(&text).expect("parse");
    assert_eq!(back, tree);

    // Ops and places use the same snake_case wire names.
    let op: Op =
        serde_json::from_str(r#"{"type":"join_tabs","tab":"t1","into":"t2","dir":"down"}"#)
            .expect("parse an op");
    assert_eq!(
        op,
        Op::JoinTabs {
            tab: "t1".to_string(),
            into: "t2".to_string(),
            dir: Dir::Down,
        }
    );
    let place: Place = serde_json::from_str(r#"{"type":"split_of","session":"a","dir":"right"}"#)
        .expect("parse a place");
    assert_eq!(
        place,
        Place::SplitOf {
            session: "a".to_string(),
            dir: Dir::Right,
        }
    );
    assert_eq!(Place::default(), Place::NewTab);
    let empty: Tree = serde_json::from_str("{}").expect("parse an empty tree");
    assert_eq!(empty, Tree::default());
}

#[test]
fn unknown_session_leaf_is_dropped() {
    let root = Root::new();
    let app = root.repo("app");
    let web = root.repo("web");
    let mut tree = Tree::default();
    let kept = shell(&mut tree, "a", &app);
    split_of(&mut tree, "b", "a", Dir::Right, &app);
    shell(&mut tree, "c", &web);
    ok(
        &mut tree,
        Op::RenameGroup {
            group: "remote:github.com/owner/web".to_string(),
            name: Some("Web".to_string()),
        },
    );

    assert!(tree.retain_sessions(|session| session == "a"));
    assert_eq!(
        layout(&tree),
        [
            (
                "remote:github.com/owner/app".to_string(),
                vec![kept.clone()]
            ),
            (String::new(), vec![]),
        ]
    );
    assert_eq!(tab(&tree, &kept).split, leaf("a"));
    assert_eq!(
        tree.defaults
            .get("remote:github.com/owner/web")
            .and_then(|d| d.name.clone()),
        Some("Web".to_string())
    );
    assert!(!tree.retain_sessions(|session| session == "a"));
}

#[test]
fn rename_survives_title_and_cwd_events() {
    let root = Root::new();
    let plain = root.plain("plain");
    let app = root.repo("app");
    let mut tree = Tree::default();
    let id = shell(&mut tree, "a", &plain);
    ok(
        &mut tree,
        Op::RenameTab {
            tab: id.clone(),
            name: Some("mine".to_string()),
        },
    );
    split_of(&mut tree, "b", "a", Dir::Right, &plain);

    assert!(tree.on_cwd("a", &app));
    assert!(!tree.on_cwd("a", &plain));
    assert!(tree.on_exit("b", true));
    ok(&mut tree, Op::Used { tab: id.clone() });
    assert_eq!(tab(&tree, &id).name.as_deref(), Some("mine"));

    ok(
        &mut tree,
        Op::RenameTab {
            tab: id.clone(),
            name: None,
        },
    );
    assert_eq!(tab(&tree, &id).name, None);
}

#[test]
fn ops_name_what_is_missing() {
    let root = Root::new();
    let app = root.repo("app");
    let mut tree = Tree::default();
    let id = shell(&mut tree, "a", &app);
    split_of(&mut tree, "b", "a", Dir::Right, &app);

    let no_tab = tree.apply(Op::CloseTab {
        tab: "nope".to_string(),
    });
    assert_eq!(no_tab, Err(TreeError::NoTab));
    assert_eq!(TreeError::NoTab.code(), "no_tab");
    let no_group = tree.apply(Op::MoveTab {
        tab: id.clone(),
        group: Some("folder:nope".to_string()),
        index: 0,
    });
    assert_eq!(no_group, Err(TreeError::NoGroup));
    assert_eq!(TreeError::NoGroup.code(), "no_group");
    let join_self = tree.apply(Op::JoinTabs {
        tab: id.clone(),
        into: id.clone(),
        dir: Dir::Right,
    });
    assert_eq!(join_self, Err(TreeError::BadOp));
    assert_eq!(TreeError::BadOp.code(), "bad_op");

    // New sizes must keep the same panes, with one size per child.
    let lost_pane = tree.apply(Op::SetSizes {
        tab: id.clone(),
        split: leaf("a"),
    });
    assert_eq!(lost_pane, Err(TreeError::BadOp));
    let short = tree.apply(Op::SetSizes {
        tab: id.clone(),
        split: Split::Row {
            children: vec![leaf("a"), leaf("b")],
            sizes: vec![1.0],
        },
    });
    assert_eq!(short, Err(TreeError::BadOp));
    let sized = Split::Col {
        children: vec![leaf("b"), leaf("a")],
        sizes: vec![0.25, 0.75],
    };
    ok(
        &mut tree,
        Op::SetSizes {
            tab: id.clone(),
            split: sized.clone(),
        },
    );
    assert_eq!(tab(&tree, &id).split, sized);
}

#[test]
fn set_sizes_rejects_single_child_node() {
    let root = Root::new();
    let plain = root.plain("plain");
    let mut tree = Tree::default();
    let id = shell(&mut tree, "a", &plain);
    split_of(&mut tree, "b", "a", Dir::Right, &plain);
    let before = tree.clone();
    let lonely = tree.apply(Op::SetSizes {
        tab: id,
        split: Split::Row {
            children: vec![
                Split::Col {
                    children: vec![leaf("a")],
                    sizes: vec![1.0],
                },
                leaf("b"),
            ],
            sizes: vec![0.5, 0.5],
        },
    });
    assert_eq!(lonely, Err(TreeError::BadOp));
    assert_eq!(tree, before);
}
