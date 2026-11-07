//! The finder's own snapshot of its last scan, at a path the host chooses.
//!
//! Stale-while-revalidate: a finder opens on whatever the snapshot holds, however old, and rescans in
//! the background when it is older than the configured TTL. Its age is the file's modification time, so
//! nothing inside the file has to be trusted to say when it was written.
//!
//! The format is the goto-repo cache's — a summary line, then a kind and a path per line — so the file
//! reads with the same parser and a person can read it too.

use std::path::Path;
use std::time::{Duration, SystemTime};

use super::source::{Item, Kind, parse_launcher_cache};

/// A snapshot read back: its items and when it was written.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// What the last scan found.
    pub items: Vec<Item>,
    /// When it was written.
    pub written: SystemTime,
}

impl Snapshot {
    /// Whether it is older than `ttl` at `now`. A snapshot from the future — a clock that moved back —
    /// is not stale: rescanning on every open would be the cost of trusting it less.
    #[must_use]
    pub fn is_stale(&self, ttl: Duration, now: SystemTime) -> bool {
        now.duration_since(self.written).is_ok_and(|age| age > ttl)
    }
}

/// Read the snapshot at `path`, or `None` when there is none.
#[must_use]
pub fn load(path: &Path) -> Option<Snapshot> {
    let written = std::fs::metadata(path).ok()?.modified().ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(Snapshot {
        items: parse_launcher_cache(&text)
            .into_iter()
            .chain(parse_files(&text))
            .collect(),
        written,
    })
}

/// Write `items` to `path`, through a rename so a reader never sees half a file.
///
/// # Errors
///
/// Whatever creating the folder, writing the temporary file or renaming it returned.
pub fn save(path: &Path, items: &[Item]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = format!("{} items\n", items.len());
    for item in items {
        let kind = match item.kind {
            Kind::Repo => "repo",
            Kind::Worktree => "  wt",
            Kind::Folder => " dir",
            Kind::Workspace => "  ws",
            Kind::File => "file",
        };
        text.push_str(kind);
        text.push('\t');
        text.push_str(&item.path.to_string_lossy());
        text.push('\n');
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
}

/// The `file` rows, which the goto-repo parser does not know.
fn parse_files(text: &str) -> impl Iterator<Item = Item> + '_ {
    text.lines().skip(1).filter_map(|line| {
        let (kind, path) = line.split_once('\t')?;
        (kind == "file").then(|| Item::new(path, Kind::File))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_reads_back_what_was_saved_in_order() {
        let dir = super::super::source::tests::scratch("cache");
        let path = dir.join("nested").join("snapshot.tsv");
        assert!(load(&path).is_none(), "no file, no snapshot");
        let items = vec![
            Item::new("/home/user/a", Kind::Repo),
            Item::new("/home/user/b", Kind::Worktree),
            Item::new("/home/user", Kind::Folder),
            Item::new("/home/user/w", Kind::Workspace),
        ];
        save(&path, &items).expect("saved");
        let snapshot = load(&path).expect("read back");
        assert_eq!(snapshot.items, items);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn staleness_is_age_past_the_ttl() {
        let written = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let snapshot = Snapshot {
            items: Vec::new(),
            written,
        };
        let ttl = Duration::from_mins(1);
        assert!(!snapshot.is_stale(ttl, written + Duration::from_mins(1)));
        assert!(snapshot.is_stale(ttl, written + Duration::from_secs(61)));
        assert!(
            !snapshot.is_stale(ttl, written - Duration::from_secs(5)),
            "a clock that moved back does not force a rescan"
        );
    }
}
