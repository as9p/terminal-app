//! Finds the project a folder belongs to: its git root, branch and remote, read straight from
//! the files under `.git`. Never runs the git program, so it costs no process per prompt.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    /// `remote:<host/owner/repo>`, or `folder:<root folder path>` with no remote or no git.
    pub key: String,
    /// The folder holding `.git`, or the folder itself when there is no git.
    pub root: PathBuf,
    pub is_git: bool,
    /// None with no git or a detached HEAD.
    pub branch: Option<String>,
}

/// The project for a folder. Walks up to the first `.git`, stopping at the home folder or the
/// filesystem root, where the folder becomes a project of its own.
pub fn resolve(folder: &Path) -> Project {
    resolve_under(folder, dirs::home_dir().as_deref())
}

/// `resolve` with the home folder given, so tests need not change the real one.
fn resolve_under(folder: &Path, home: Option<&Path>) -> Project {
    let home = home.map(plain);
    for dir in folder.ancestors() {
        // Home is only looked at when it is the folder asked about, so a dotfiles repo there
        // does not swallow every folder under it.
        if home.is_some() && home == Some(plain(dir)) && dir != folder {
            break;
        }
        let dot_git = dir.join(".git");
        let git_dir = if dot_git.is_dir() {
            Some(dot_git)
        } else if dot_git.is_file() {
            linked_git_dir(dir, &dot_git)
        } else {
            None
        };
        if let Some(git_dir) = git_dir {
            return git_project(dir, &git_dir);
        }
    }
    Project {
        key: folder_key(folder),
        root: folder.to_path_buf(),
        is_git: false,
        branch: None,
    }
}

/// The folder as text without the `\\?\` that `canonicalize` puts in front of a Windows path,
/// so a resolved folder and the home folder compare in the same form. A Unix folder never
/// starts that way.
fn plain(path: &Path) -> Cow<'_, str> {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return Cow::Owned(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => Cow::Owned(rest.to_string()),
        None => text,
    }
}

/// `host/owner/repo` in lower case, with no scheme, user, port or `.git`, so the same repo
/// matches whether it was cloned over HTTPS or SSH.
pub fn normalize_remote(url: &str) -> String {
    let url = url.trim().to_lowercase().replace('\\', "/");
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let host = host.split_once(':').map_or(host, |(host, _)| host);
        (host.to_string(), path.to_string())
    } else if let Some((authority, path)) = scp_parts(&url) {
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        (host.to_string(), path.to_string())
    } else {
        (String::new(), url.clone())
    };
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    if host.is_empty() {
        path.to_string()
    } else {
        format!("{host}/{path}")
    }
}

/// `user@host:path`, the short form SSH remotes use. A Windows drive letter is not a host.
fn scp_parts(url: &str) -> Option<(&str, &str)> {
    let (authority, path) = url.split_once(':')?;
    let drive = authority.len() == 1 && authority.chars().all(|c| c.is_ascii_alphabetic());
    if authority.is_empty() || authority.contains('/') || drive {
        return None;
    }
    Some((authority, path))
}

/// The git folder a `.git` file names on its `gitdir:` line, as a linked worktree or a
/// submodule has. A relative path is from the folder holding the file.
fn linked_git_dir(dir: &Path, dot_git: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(dot_git).ok()?;
    let target = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))?
        .trim();
    if target.is_empty() {
        return None;
    }
    Some(dir.join(target))
}

fn git_project(root: &Path, git_dir: &Path) -> Project {
    // A worktree keeps its own HEAD, but shares the main repo's config through `commondir`.
    let common = match fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) if !text.trim().is_empty() => git_dir.join(text.trim()),
        _ => git_dir.to_path_buf(),
    };
    let key = fs::read_to_string(common.join("config"))
        .ok()
        .and_then(|config| origin_url(&config))
        .map(|url| format!("remote:{}", normalize_remote(&url)))
        .unwrap_or_else(|| folder_key(root));
    let branch = fs::read_to_string(git_dir.join("HEAD"))
        .ok()
        .and_then(|head| {
            head.trim()
                .strip_prefix("ref:")
                .and_then(|name| name.trim().strip_prefix("refs/heads/"))
                .map(str::to_string)
        });
    Project {
        key,
        root: root.to_path_buf(),
        is_git: true,
        branch,
    }
}

/// The `url` under `[remote "origin"]` in a git config file.
fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            let section: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            in_origin = section == "[remote\"origin\"]";
        } else if in_origin
            && let Some((name, value)) = line.split_once('=')
            && name.trim() == "url"
            && !value.trim().is_empty()
        {
            return Some(value.trim().to_string());
        }
    }
    None
}

/// `folder:<path>`, the folder's whole path, so two folders that share a name stay apart.
pub(crate) fn folder_key(folder: &Path) -> String {
    format!("folder:{}", plain(folder))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A temp folder removed on drop, with every link in its path resolved.
    struct Root(PathBuf);

    impl Root {
        fn new() -> Root {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("term-host-project-{}-{n}", std::process::id()));
            fs::create_dir_all(&path).expect("make the test folder");
            Root(fs::canonicalize(&path).expect("resolve the test folder"))
        }

        fn dir(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);
            fs::create_dir_all(&path).expect("make a folder");
            path
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().expect("a parent")).expect("make a folder");
            fs::write(path, text).expect("write a file");
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const ORIGIN: &str = "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = https://example.com/other.git\n[remote \"origin\"]\n\turl = git@github.com:Owner/App.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n";

    #[test]
    fn finds_root_from_a_subfolder() {
        let root = Root::new();
        root.write("app/.git/HEAD", "ref: refs/heads/main\n");
        root.write("app/.git/config", ORIGIN);
        let deep = root.dir("app/src/deep");

        let project = resolve(&deep);
        assert_eq!(project.root, root.0.join("app"));
        assert!(project.is_git);
        assert_eq!(project.key, "remote:github.com/owner/app");
        assert_eq!(project.branch.as_deref(), Some("main"));
    }

    #[test]
    fn worktree_git_file_uses_main_repo_remote() {
        let root = Root::new();
        root.write("main/.git/HEAD", "ref: refs/heads/main\n");
        root.write("main/.git/config", ORIGIN);
        root.write("main/.git/worktrees/wt/HEAD", "ref: refs/heads/feature/x\n");
        root.write("main/.git/worktrees/wt/commondir", "../..\n");
        let gitdir = root.0.join("main/.git/worktrees/wt");
        root.write("wt/.git", &format!("gitdir: {}\n", gitdir.display()));
        let sub = root.dir("wt/src");

        let project = resolve(&sub);
        assert_eq!(project.root, root.0.join("wt"));
        assert!(project.is_git);
        assert_eq!(project.key, "remote:github.com/owner/app");
        assert_eq!(project.branch.as_deref(), Some("feature/x"));

        // A submodule's `.git` file points with a relative path to its own git folder.
        root.write("super/.git/HEAD", "ref: refs/heads/main\n");
        root.write("super/.git/modules/lib/HEAD", "ref: refs/heads/trunk\n");
        root.write(
            "super/.git/modules/lib/config",
            "[remote \"origin\"]\n\turl = https://gitlab.com/Team/Lib\n",
        );
        root.write("super/lib/.git", "gitdir: ../.git/modules/lib\n");
        let project = resolve(&root.0.join("super/lib"));
        assert_eq!(project.root, root.0.join("super/lib"));
        assert_eq!(project.key, "remote:gitlab.com/team/lib");
        assert_eq!(project.branch.as_deref(), Some("trunk"));
    }

    #[test]
    fn no_remote_uses_folder_name() {
        let root = Root::new();
        root.write("Solo/.git/HEAD", "ref: refs/heads/main\n");
        root.write("Solo/.git/config", "[core]\n\tbare = false\n");
        let project = resolve(&root.0.join("Solo"));
        assert!(project.is_git);
        assert_eq!(project.root, root.0.join("Solo"));
        assert_eq!(project.key, folder_key(&root.0.join("Solo")));

        // No config file at all is the same.
        root.write("bare/.git/HEAD", "ref: refs/heads/main\n");
        let bare = root.0.join("bare");
        assert_eq!(resolve(&bare).key, format!("folder:{}", bare.display()));
    }

    #[test]
    fn no_git_never_climbs_to_root() {
        let root = Root::new();
        let deep = root.dir("plain/inner");
        let project = resolve(&deep);
        assert_eq!(project.root, deep);
        assert!(!project.is_git);
        assert_eq!(project.key, format!("folder:{}", deep.display()));
        assert_eq!(project.branch, None);
    }

    #[test]
    fn stops_at_home() {
        let root = Root::new();
        // A home folder that is itself a repo, as a dotfiles repo would make it.
        root.write("home/.git/HEAD", "ref: refs/heads/main\n");
        let home = root.0.join("home");
        let deep = root.dir("home/notes/today");

        let project = resolve_under(&deep, Some(&home));
        assert_eq!(project.root, deep);
        assert!(!project.is_git);

        // Home itself is still a repo when that is the folder asked about.
        assert!(resolve_under(&home, Some(&home)).is_git);
    }

    /// `canonicalize` puts `\\?\` in front of a Windows folder, and the home folder has none.
    #[test]
    fn home_matches_without_verbatim_prefix() {
        assert_eq!(
            plain(Path::new(r"\\?\C:\Users\x")),
            plain(Path::new(r"C:\Users\x"))
        );
        assert_eq!(
            plain(Path::new(r"\\?\UNC\server\share\x")),
            plain(Path::new(r"\\server\share\x"))
        );
        assert_ne!(
            plain(Path::new(r"\\?\C:\Users\x")),
            plain(Path::new(r"C:\Users\y"))
        );
    }

    #[test]
    fn normalize_remote_forms() {
        for url in [
            "git@github.com:Owner/Repo.git",
            "https://github.com/owner/repo",
            "ssh://git@github.com/owner/repo.git",
            "https://user@GitHub.com/owner/repo.git/",
            "ssh://git@github.com:22/owner/repo",
            "git://github.com/owner/repo.git",
            "  https://github.com/owner/repo.git\n",
        ] {
            assert_eq!(normalize_remote(url), "github.com/owner/repo", "{url}");
        }
        // Local paths keep forward slashes whatever the separator.
        assert_eq!(normalize_remote(r"C:\Repos\App.git"), "c:/repos/app");
        assert_eq!(normalize_remote("/srv/git/App.git"), "srv/git/app");
    }

    #[test]
    fn branch_from_head() {
        let root = Root::new();
        root.write("app/.git/HEAD", "ref: refs/heads/topic/one\n");
        assert_eq!(
            resolve(&root.0.join("app")).branch.as_deref(),
            Some("topic/one")
        );
    }

    #[test]
    fn detached_head_has_no_branch() {
        let root = Root::new();
        root.write(
            "app/.git/HEAD",
            "0123456789abcdef0123456789abcdef01234567\n",
        );
        let project = resolve(&root.0.join("app"));
        assert!(project.is_git);
        assert_eq!(project.branch, None);
    }
}
