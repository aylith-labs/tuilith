//! The goto-repo usage history, and the orders a finder can sort by.
//!
//! The history is an append-only file, one line per visit, `<unix-epoch>\t<path>`. From it come a visit
//! count and a last-used time per path, which are what the recent and most-used orders read. It is the
//! file goto-repo and its ranking script append to, so a pick made in one tool ranks in all of them.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The order a finder lists its items in before anything is typed.
///
/// Once a query is typed the match score decides, and this order breaks ties — the same rule fzf
/// follows with `--tiebreak=index`, which is what the shell finders sharing this history run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    /// The order the sources gave.
    #[default]
    Relevance,
    /// By name, ignoring case.
    Name,
    /// Most recently picked first.
    Recent,
    /// Most often picked first.
    MostUsed,
}

impl SortMode {
    /// Every mode, in the order [`Self::cycle`] walks them.
    pub const ALL: [Self; 4] = [Self::Relevance, Self::Name, Self::Recent, Self::MostUsed];

    /// The next mode: relevance, name, recent, most used, and round again.
    #[must_use]
    pub fn cycle(self) -> Self {
        match self {
            Self::Relevance => Self::Name,
            Self::Name => Self::Recent,
            Self::Recent => Self::MostUsed,
            Self::MostUsed => Self::Relevance,
        }
    }

    /// A short word for the mode, for a header or a menu.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Relevance => "relevance",
            Self::Name => "name",
            Self::Recent => "recent",
            Self::MostUsed => "most used",
        }
    }

    /// The mode a goto-repo sort file names.
    ///
    /// That file holds one digit: 0 alphabetical, 1 recent, 2 most used, 3 frecency. Frecency is read
    /// from zoxide, which this does not consult, so it comes back as most used — the nearest order this
    /// can compute. Anything else is not a sort file and answers `None`.
    #[must_use]
    pub fn from_shared(text: &str) -> Option<Self> {
        match text.trim() {
            "0" => Some(Self::Name),
            "1" => Some(Self::Recent),
            "2" | "3" => Some(Self::MostUsed),
            _ => None,
        }
    }
}

/// The goto-repo cache directory under `home`: it holds `repos.tsv` and the sort file.
#[must_use]
pub fn goto_repo_cache_dir(home: &Path) -> PathBuf {
    home.join(".cache").join("goto-repo")
}

/// The goto-repo history file under `home`.
#[must_use]
pub fn goto_repo_history_path(home: &Path) -> PathBuf {
    home.join(".config").join("goto-repo").join("history")
}

/// The order a goto-repo sort file asks for, if one is there and reads as one.
#[must_use]
pub fn read_shared_sort(path: &Path) -> Option<SortMode> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| SortMode::from_shared(&text))
}

/// Visit counts and last-used times per path, read from a history file.
#[derive(Debug, Clone, Default)]
pub struct History {
    stats: HashMap<String, (u32, i64)>,
    path: Option<PathBuf>,
}

impl History {
    /// Read a history file. A missing or unreadable one is an empty history, and the first
    /// [`Self::record_use`] creates it.
    #[must_use]
    pub fn load(path: PathBuf) -> Self {
        let mut stats: HashMap<String, (u32, i64)> = HashMap::new();
        if let Ok(contents) = std::fs::read_to_string(&path) {
            for line in contents.lines() {
                let Some((epoch, entry)) = line.split_once('\t') else {
                    continue;
                };
                let epoch: i64 = epoch.trim().parse().unwrap_or(0);
                let stat = stats.entry(entry.to_string()).or_insert((0, 0));
                stat.0 += 1;
                stat.1 = stat.1.max(epoch);
            }
        }
        Self {
            stats,
            path: Some(path),
        }
    }

    /// A history held only in memory: nothing is read, and a use is never written anywhere.
    #[must_use]
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// How many times `path` was picked.
    #[must_use]
    pub fn count(&self, path: &str) -> u32 {
        self.stats.get(path).map_or(0, |stat| stat.0)
    }

    /// When `path` was last picked, in seconds since the epoch, or 0 when it never was.
    #[must_use]
    pub fn last_used(&self, path: &str) -> i64 {
        self.stats.get(path).map_or(0, |stat| stat.1)
    }

    /// Count a pick of `path`, and append it to the file this history was read from.
    ///
    /// The write is best effort: a history that cannot be written still ranks this session's picks.
    pub fn record_use(&mut self, path: &str) {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
            .unwrap_or(0);
        let stat = self.stats.entry(path.to_string()).or_insert((0, 0));
        stat.0 += 1;
        stat.1 = epoch;
        let Some(file) = &self.path else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut handle) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
        {
            let _ = writeln!(handle, "{epoch}\t{path}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_history_counts_visits_and_keeps_the_latest_time() {
        let dir = std::env::temp_dir().join(format!("tuilith-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let file = dir.join("history");
        std::fs::write(&file, "100\t/a\n200\t/a\n150\t/b\nnot a line\n").expect("written");
        let history = History::load(file);
        assert_eq!(history.count("/a"), 2);
        assert_eq!(history.last_used("/a"), 200);
        assert_eq!(history.count("/b"), 1);
        assert_eq!(history.count("/missing"), 0);
        assert_eq!(history.last_used("/missing"), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_recorded_use_is_appended_where_another_tool_reads_it() {
        let dir = std::env::temp_dir().join(format!("tuilith-record-{}", std::process::id()));
        let file = dir.join("nested").join("history");
        let mut history = History::load(file.clone());
        history.record_use("/x");
        assert_eq!(history.count("/x"), 1);
        let written = std::fs::read_to_string(&file).expect("the file was created");
        assert!(written.ends_with("\t/x\n"), "{written:?}");
        std::fs::remove_dir_all(&dir).ok();

        let mut memory = History::in_memory();
        memory.record_use("/y");
        assert_eq!(memory.count("/y"), 1, "an in-memory history still ranks");
    }

    #[test]
    fn sort_modes_cycle_and_read_the_shared_sort_file() {
        assert_eq!(SortMode::Relevance.cycle(), SortMode::Name);
        assert_eq!(SortMode::MostUsed.cycle(), SortMode::Relevance);
        assert_eq!(SortMode::from_shared("0\n"), Some(SortMode::Name));
        assert_eq!(SortMode::from_shared("1"), Some(SortMode::Recent));
        assert_eq!(SortMode::from_shared("2"), Some(SortMode::MostUsed));
        assert_eq!(SortMode::from_shared("3"), Some(SortMode::MostUsed));
        assert_eq!(SortMode::from_shared("9"), None);
        assert_eq!(SortMode::from_shared(""), None);
    }
}
