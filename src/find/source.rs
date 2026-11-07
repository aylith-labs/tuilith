//! Where a finder's items come from.
//!
//! Each source is a function over paths, so a test can hand it a directory it built and a host can call
//! one alone. [`instant`] is what is known without walking anything — the goto-repo cache, bookmarks,
//! and the two folders home and root — and is what a finder draws on its first frame. [`scan`] walks,
//! and is what a background revalidation runs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::Config;

/// What an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A git repository: a folder holding a `.git` directory.
    Repo,
    /// A git worktree: a folder holding a `.git` file.
    Worktree,
    /// A plain folder.
    Folder,
    /// A workspace, as the goto-repo cache marks one.
    Workspace,
    /// A file.
    File,
}

impl Kind {
    /// A short word for the kind column.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Repo => "repo",
            Self::Worktree => "wt",
            Self::Folder => "dir",
            Self::Workspace => "ws",
            Self::File => "file",
        }
    }

    /// Whether this is a git checkout of either shape.
    #[must_use]
    pub fn is_git(self) -> bool {
        matches!(self, Self::Repo | Self::Worktree)
    }
}

/// One thing a finder can pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Where it is. Absolute for folders; for files, relative to the root they were listed under.
    pub path: PathBuf,
    /// What it is.
    pub kind: Kind,
}

impl Item {
    /// An item of `kind` at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, kind: Kind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }

    /// Its last path component, or the whole path where there is none (`/`).
    #[must_use]
    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    /// The text that identifies it: the path as a string. History is keyed on this.
    #[must_use]
    pub fn key(&self) -> String {
        self.path.display().to_string()
    }
}

/// Which sources a finder lists. Any combination; the default is git repositories alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // Six independent switches a host maps one-to-one onto settings.
pub struct Sources {
    /// Git repositories and worktrees: from the goto-repo cache at once, and by walking the roots on a
    /// revalidation.
    pub git_repos: bool,
    /// Every folder under the roots, to the configured depth.
    pub all_folders: bool,
    /// The home folder and the folders directly in it.
    pub home: bool,
    /// The filesystem root and the folders directly in it.
    pub root: bool,
    /// The configured bookmarks.
    pub bookmarks: bool,
    /// Everything the goto-repo cache lists: repositories, worktrees, workspaces and the folders
    /// holding them.
    pub launcher_cache: bool,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            git_repos: true,
            all_folders: false,
            home: false,
            root: false,
            bookmarks: false,
            launcher_cache: false,
        }
    }
}

/// Read a goto-repo cache file.
///
/// The first line is a summary (`12 repos · 3 worktrees`) and is skipped. Every other line is a kind and
/// a path separated by a tab, the kind padded with spaces to line up in fzf: `repo`, `  wt`, ` dir`,
/// `  ws`. A line of any other shape is skipped rather than guessed at. `None` when the file is not
/// there, which is a different answer from an empty cache.
#[must_use]
pub fn read_launcher_cache(path: &Path) -> Option<Vec<Item>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(parse_launcher_cache(&text))
}

/// The items in a goto-repo cache's text. See [`read_launcher_cache`].
#[must_use]
pub fn parse_launcher_cache(text: &str) -> Vec<Item> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let (kind, path) = line.split_once('\t')?;
            let kind = match kind.trim() {
                "repo" => Kind::Repo,
                "wt" => Kind::Worktree,
                "dir" => Kind::Folder,
                "ws" => Kind::Workspace,
                _ => return None,
            };
            let path = path.trim_end();
            (!path.is_empty()).then(|| Item::new(path, kind))
        })
        .collect()
}

/// What is known without walking anything, in the order a finder lists it: bookmarks, the cache's
/// repositories, the rest of the cache, home, root. A path two sources give is listed once, where it
/// first appears.
#[must_use]
pub fn instant(config: &Config) -> Vec<Item> {
    let cache = if config.sources.git_repos || config.sources.launcher_cache {
        config
            .launcher_cache
            .as_deref()
            .and_then(read_launcher_cache)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut items = Vec::new();
    if config.sources.bookmarks {
        items.extend(bookmarks(config));
    }
    if config.sources.git_repos {
        items.extend(cache.iter().filter(|item| item.kind.is_git()).cloned());
    }
    if config.sources.launcher_cache {
        items.extend(cache);
    }
    if config.sources.home {
        items.extend(folder_and_children(&config.home));
    }
    if config.sources.root {
        items.extend(folder_and_children(Path::new("/")));
    }
    dedup(items, config.limit)
}

/// Everything the configured sources hold, walking the roots where a source asks for that. Slow on a
/// large tree, which is why a finder runs it off the thread that draws.
#[must_use]
pub fn scan(config: &Config) -> Vec<Item> {
    let mut items = Vec::new();
    if config.sources.bookmarks {
        items.extend(bookmarks(config));
    }
    if config.sources.git_repos || config.sources.all_folders {
        let walked = walk_folders(config);
        if config.sources.git_repos {
            // The cache's checkouts first, so a scan adds to what the first frame showed and never
            // drops a repository the walk's roots do not reach.
            items.extend(
                config
                    .launcher_cache
                    .as_deref()
                    .and_then(read_launcher_cache)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|item| item.kind.is_git()),
            );
            items.extend(walked.iter().filter(|item| item.kind.is_git()).cloned());
        }
        if config.sources.all_folders {
            items.extend(walked);
        }
    }
    if config.sources.launcher_cache {
        items.extend(
            config
                .launcher_cache
                .as_deref()
                .and_then(read_launcher_cache)
                .unwrap_or_default(),
        );
    }
    if config.sources.home {
        items.extend(folder_and_children(&config.home));
    }
    if config.sources.root {
        items.extend(folder_and_children(Path::new("/")));
    }
    dedup(items, config.limit)
}

fn bookmarks(config: &Config) -> impl Iterator<Item = Item> + '_ {
    config
        .bookmarks
        .iter()
        .map(|path| Item::new(path.clone(), kind_of(path)))
}

/// What a folder is, by its `.git`: a directory makes it a repository, a file a worktree.
#[must_use]
pub fn kind_of(folder: &Path) -> Kind {
    let git = folder.join(".git");
    if git.is_dir() {
        Kind::Repo
    } else if git.is_file() {
        Kind::Worktree
    } else {
        Kind::Folder
    }
}

/// `folder` and the folders directly inside it, by name.
#[must_use]
pub fn folder_and_children(folder: &Path) -> Vec<Item> {
    let mut children: Vec<Item> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| {
            let path = entry.path();
            let kind = kind_of(&path);
            Item::new(path, kind)
        })
        .collect();
    children.sort_by_key(|item| item.name().to_lowercase());
    let mut items = vec![Item::new(folder, kind_of(folder))];
    items.extend(children);
    items
}

/// Every folder under the roots, breadth first, down to `config.depth` levels, never entering a folder
/// whose name `config.skip` lists, nor a hidden one, nor a git checkout's own `.git`. A checkout is
/// listed and not entered: what is inside one is its files, which is the file finder's question.
fn walk_folders(config: &Config) -> Vec<Item> {
    let mut found = Vec::new();
    let mut level: Vec<PathBuf> = config.roots.clone();
    for _ in 0..config.depth {
        let mut next = Vec::new();
        for folder in &level {
            let Ok(entries) = std::fs::read_dir(folder) else {
                continue;
            };
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || config.skip.contains(&name) {
                    continue;
                }
                let path = entry.path();
                let kind = kind_of(&path);
                if !kind.is_git() {
                    next.push(path.clone());
                }
                found.push(Item::new(path, kind));
                if found.len() >= config.limit {
                    return found;
                }
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    found
}

/// The files under `root`, relative to it.
///
/// Inside a git repository the list is `git ls-files`: tracked files and untracked ones that are not
/// ignored, so `.gitignore` decides exactly as git does. Outside one, or where git is not installed, it
/// is a walk that skips hidden entries — there is no `.gitignore` for it to honour. Either way at most
/// `limit` files, sorted.
#[must_use]
pub fn files(root: &Path, limit: usize) -> Vec<Item> {
    let mut paths = git_files(root).unwrap_or_else(|| walk_files(root, limit));
    paths.sort();
    paths.truncate(limit);
    paths
        .into_iter()
        .map(|path| Item::new(path, Kind::File))
        .collect()
}

fn git_files(root: &Path) -> Option<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    // `--cached` lists a file once per merge stage while a merge is unresolved.
    let mut seen = HashSet::new();
    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
            .map(|name| PathBuf::from(String::from_utf8_lossy(name).into_owned()))
            .filter(|path| seen.insert(path.clone()))
            .collect(),
    )
}

fn walk_files(root: &Path, limit: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&relative)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let path = relative.join(&name);
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(path),
                Ok(kind) if kind.is_file() => {
                    found.push(path);
                    if found.len() >= limit {
                        return found;
                    }
                }
                _ => {}
            }
        }
    }
    found
}

fn dedup(items: Vec<Item>, limit: usize) -> Vec<Item> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.path.clone()))
        .take(limit)
        .collect()
}

#[cfg(test)]
pub(in crate::find) mod tests {
    use super::*;

    pub(in crate::find) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tuilith-find-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch dir");
        dir
    }

    #[test]
    fn the_launcher_cache_skips_its_summary_and_reads_every_padded_kind() {
        let text = "2 repos · 1 worktrees\n\
                    repo\t/home/user/src/alpha\n\
                    repo\t/home/user/src/beta\n  \
                    wt\t/home/user/src/alpha.worktrees/one\n \
                    dir\t/home/user/src\n  \
                    ws\t/home/user/work\n\
                    junk line\n\
                    pkg\t/home/user/nope\n";
        let items = parse_launcher_cache(text);
        let got: Vec<(Kind, &str)> = items
            .iter()
            .map(|item| (item.kind, item.path.to_str().expect("utf-8")))
            .collect();
        assert_eq!(
            got,
            [
                (Kind::Repo, "/home/user/src/alpha"),
                (Kind::Repo, "/home/user/src/beta"),
                (Kind::Worktree, "/home/user/src/alpha.worktrees/one"),
                (Kind::Folder, "/home/user/src"),
                (Kind::Workspace, "/home/user/work"),
            ]
        );
        assert!(
            parse_launcher_cache("repo\t/only/a/summary/line\n").is_empty(),
            "the first line is the summary even when it looks like a row"
        );
    }

    fn config_over(dir: &Path) -> Config {
        let cache = dir.join("repos.tsv");
        std::fs::write(
            &cache,
            "1 repos\nrepo\t/home/user/a\n dir\t/home/user\n  wt\t/home/user/b\n",
        )
        .expect("cache written");
        let mut config = Config::new(dir.join("home"));
        config.launcher_cache = Some(cache);
        config.roots = vec![dir.join("tree")];
        config.bookmarks = vec![
            PathBuf::from("/home/user/marked"),
            PathBuf::from("/home/user/a"),
        ];
        config
    }

    #[test]
    fn git_repos_read_only_checkouts_from_the_cache_and_the_whole_cache_reads_everything() {
        let dir = scratch("instant");
        let mut config = config_over(&dir);
        let paths = |items: Vec<Item>| -> Vec<String> { items.iter().map(Item::key).collect() };

        assert_eq!(paths(instant(&config)), ["/home/user/a", "/home/user/b"]);

        config.sources = Sources {
            git_repos: false,
            launcher_cache: true,
            ..Sources::default()
        };
        assert_eq!(
            paths(instant(&config)),
            ["/home/user/a", "/home/user", "/home/user/b"]
        );

        config.sources = Sources {
            bookmarks: true,
            ..Sources::default()
        };
        assert_eq!(
            paths(instant(&config)),
            ["/home/user/marked", "/home/user/a", "/home/user/b"],
            "a bookmark listed first, and a repo it repeats listed once"
        );

        config.sources = Sources {
            git_repos: false,
            ..Sources::default()
        };
        assert!(instant(&config).is_empty(), "no source, no items");
        std::fs::remove_dir_all(&dir).ok();
    }

    fn tree(dir: &Path) {
        for folder in [
            "tree/code/alpha/.git",
            "tree/code/alpha/src",
            "tree/code/notes",
            "tree/code/beta",
            "tree/node_modules/pkg/.git",
            "tree/.hidden/gamma/.git",
        ] {
            std::fs::create_dir_all(dir.join(folder)).expect("tree built");
        }
        std::fs::write(dir.join("tree/code/beta/.git"), "gitdir: elsewhere").expect("worktree");
    }

    #[test]
    fn a_scan_finds_checkouts_under_the_roots_and_lists_folders_only_when_asked() {
        let dir = scratch("scan");
        tree(&dir);
        let mut config = config_over(&dir);
        config.skip = vec!["node_modules".into()];
        let names = |items: Vec<Item>| -> Vec<(String, Kind)> {
            let mut named: Vec<_> = items.iter().map(|item| (item.name(), item.kind)).collect();
            named.sort_by(|left, right| left.0.cmp(&right.0));
            named
        };

        assert_eq!(
            names(scan(&config)),
            [
                ("a".into(), Kind::Repo),
                ("alpha".into(), Kind::Repo),
                ("b".into(), Kind::Worktree),
                ("beta".into(), Kind::Worktree),
            ],
            "the cache's checkouts and the walk's, never a skipped name or a hidden folder"
        );

        config.sources = Sources {
            git_repos: false,
            all_folders: true,
            ..Sources::default()
        };
        assert_eq!(
            names(scan(&config)),
            [
                ("alpha".into(), Kind::Repo),
                ("beta".into(), Kind::Worktree),
                ("code".into(), Kind::Folder),
                ("notes".into(), Kind::Folder),
            ],
            "a checkout is listed but not entered, so its src folder is not"
        );

        config.depth = 1;
        assert_eq!(names(scan(&config)), [("code".into(), Kind::Folder)]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn home_lists_itself_then_its_folders_by_name() {
        let dir = scratch("home");
        for folder in ["home/Zed", "home/apps"] {
            std::fs::create_dir_all(dir.join(folder)).expect("built");
        }
        std::fs::write(dir.join("home/file.txt"), "").expect("a file is not a folder");
        let mut config = config_over(&dir);
        config.sources = Sources {
            git_repos: false,
            home: true,
            ..Sources::default()
        };
        let names: Vec<String> = instant(&config).iter().map(Item::name).collect();
        assert_eq!(names, ["home", "apps", "Zed"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn files_outside_a_repository_are_walked_and_hidden_ones_skipped() {
        let dir = scratch("walk");
        std::fs::create_dir_all(dir.join("src/deep")).expect("built");
        std::fs::create_dir_all(dir.join(".cache")).expect("built");
        for file in [
            "README.md",
            "src/main.rs",
            "src/deep/mod.rs",
            ".env",
            ".cache/blob",
        ] {
            std::fs::write(dir.join(file), "").expect("written");
        }
        let listed: Vec<String> = files(&dir, 100).iter().map(Item::key).collect();
        assert_eq!(listed, ["README.md", "src/deep/mod.rs", "src/main.rs"]);
        assert_eq!(files(&dir, 2).len(), 2, "the limit holds");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn files_inside_a_repository_follow_gitignore() {
        let dir = scratch("git");
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .is_ok_and(|output| output.status.success())
        };
        if !git(&["init", "-q"]) {
            eprintln!("git is not installed; the walk covers this case");
            return;
        }
        std::fs::create_dir_all(dir.join("target")).expect("built");
        std::fs::write(dir.join(".gitignore"), "target/\n").expect("written");
        std::fs::write(dir.join("tracked.rs"), "").expect("written");
        std::fs::write(dir.join("untracked.rs"), "").expect("written");
        std::fs::write(dir.join("target/built.o"), "").expect("written");
        assert!(git(&["add", "tracked.rs"]));
        let listed: Vec<String> = files(&dir, 100).iter().map(Item::key).collect();
        assert_eq!(
            listed,
            [".gitignore", "tracked.rs", "untracked.rs"],
            "tracked and untracked files, and nothing git ignores"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
