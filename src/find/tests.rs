use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::source::tests::scratch;
use super::*;
use crate::theme::DEFAULT_DARK;

fn repos(paths: &[&str]) -> Vec<Item> {
    paths
        .iter()
        .map(|path| Item::new(*path, Kind::Repo))
        .collect()
}

/// A config that reads and writes nothing of the machine's.
fn quiet() -> Config {
    let mut config = Config::new(PathBuf::from("/home/user"));
    config.launcher_cache = None;
    config.history = None;
    config.roots = Vec::new();
    config.revalidate = false;
    config
}

fn listed(finder: &Finder) -> Vec<String> {
    finder
        .hits()
        .iter()
        .map(|hit| finder.item(hit).key())
        .collect()
}

fn painted(finder: &mut Finder, width: u16, height: u16) -> (Buffer, Geometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a terminal");
    let mut geometry = Geometry::default();
    terminal
        .draw(|frame| {
            geometry = draw(frame, frame.area(), finder, &DEFAULT_DARK, "search");
        })
        .expect("drawn");
    (terminal.backend().buffer().clone(), geometry)
}

fn row_text(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn the_first_frame_draws_from_the_goto_repo_cache_and_a_missing_one_draws_a_scan() {
    let dir = scratch("first-frame");
    let cache = dir.join("repos.tsv");
    std::fs::write(&cache, "2 repos\nrepo\t/home/user/a\nrepo\t/home/user/b\n").expect("written");
    let mut config = quiet();
    config.launcher_cache = Some(cache.clone());
    config.revalidate = true;
    let mut finder = Finder::open(config.clone());
    assert_eq!(
        listed(&finder),
        ["/home/user/a", "/home/user/b"],
        "listed before any scan has answered"
    );
    assert!(
        finder.loading(),
        "and a scan is running, with no snapshot to call fresh"
    );
    let (buffer, _) = painted(&mut finder, 40, 4);
    assert!(
        row_text(&buffer, 0).ends_with("scanning · 2"),
        "{:?}",
        row_text(&buffer, 0)
    );

    config.launcher_cache = Some(dir.join("absent.tsv"));
    let mut empty = Finder::open(config);
    assert_eq!(empty.total(), 0, "no cache, nothing known yet");
    assert!(empty.loading());
    let deadline = Instant::now() + Duration::from_secs(10);
    while empty.loading() && Instant::now() < deadline {
        empty.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!empty.loading(), "the scan answered within ten seconds");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_fresh_snapshot_is_not_rescanned_and_a_stale_one_is() {
    let dir = scratch("snapshot");
    let snapshot = dir.join("finder.tsv");
    cache::save(&snapshot, &repos(&["/home/user/kept"])).expect("saved");
    let mut config = quiet();
    config.snapshot = Some(snapshot.clone());
    config.revalidate = true;
    config.ttl = Duration::from_hours(1);

    let fresh = Finder::open(config.clone());
    assert_eq!(listed(&fresh), ["/home/user/kept"]);
    assert!(!fresh.loading(), "inside its TTL, nothing is scanned");

    config.ttl = Duration::ZERO;
    std::thread::sleep(Duration::from_millis(20));
    let mut stale = Finder::open(config);
    assert_eq!(
        listed(&stale),
        ["/home/user/kept"],
        "a stale snapshot still draws"
    );
    assert!(stale.loading(), "and is revalidated");
    let deadline = Instant::now() + Duration::from_secs(10);
    while stale.loading() && Instant::now() < deadline {
        stale.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        listed(&stale).is_empty(),
        "the scan found nothing, and nothing was under the pointer"
    );
    assert!(
        cache::load(&snapshot).expect("rewritten").items.is_empty(),
        "and the scan rewrote the snapshot"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_scan_never_moves_the_row_under_the_pointer() {
    let mut finder = Finder::with_items(repos(&["/r/c", "/r/d", "/r/e"]), quiet());
    let (_, geometry) = painted(&mut finder, 30, 6);
    let (rect, offset) = geometry.rows[1];
    finder.hover(Some(offset));
    assert_eq!(finder.item_at(offset).expect("a row").key(), "/r/d");

    finder.revalidated(repos(&["/r/a", "/r/b", "/r/c", "/r/d", "/r/e"]));
    let (buffer, geometry) = painted(&mut finder, 30, 6);
    assert_eq!(geometry.rows[1], (rect, offset));
    assert_eq!(
        finder.item_at(offset).expect("a row").key(),
        "/r/d",
        "the hovered row still names what it named"
    );
    assert_eq!(
        finder.pending(),
        Some(Pending {
            added: 2,
            removed: 0
        })
    );
    assert!(
        row_text(&buffer, 0).ends_with("2 new"),
        "{:?}",
        row_text(&buffer, 0)
    );
    let pending = geometry.pending.expect("pressable");
    assert_eq!(pending.width, 5);

    finder.show_pending();
    assert_eq!(finder.pending(), None);
    assert_eq!(listed(&finder), ["/r/a", "/r/b", "/r/c", "/r/d", "/r/e"]);
}

#[test]
fn a_scan_lands_at_once_on_a_list_at_the_top_with_nothing_under_the_pointer() {
    let mut finder = Finder::with_items(repos(&["/r/c", "/r/d"]), quiet());
    finder.move_by(1);
    finder.revalidated(repos(&["/r/a", "/r/c", "/r/d"]));
    assert_eq!(finder.pending(), None);
    assert_eq!(listed(&finder), ["/r/a", "/r/c", "/r/d"]);
    assert_eq!(
        finder.selected().expect("a cursor").key(),
        "/r/d",
        "the cursor stays on its item"
    );

    finder.hover(Some(0));
    finder.revalidated(repos(&["/r/a", "/r/b", "/r/c", "/r/d"]));
    assert!(finder.pending().is_some(), "held while the pointer rests");
    finder.hover(None);
    assert_eq!(finder.pending(), None, "applied when it leaves");
    assert_eq!(finder.total(), 4);
}

#[test]
fn a_scan_is_held_while_the_list_is_scrolled_even_with_no_pointer() {
    let items: Vec<Item> = (0..20)
        .map(|index| Item::new(format!("/r/{index:02}"), Kind::Repo))
        .collect();
    let mut finder = Finder::with_items(items.clone(), quiet());
    finder.move_by(15);
    let _ = painted(&mut finder, 30, 6);
    assert!(finder.scroll() > 0);
    let mut more = vec![Item::new("/r/new", Kind::Repo)];
    more.extend(items);
    finder.revalidated(more);
    assert_eq!(finder.pending().map(|pending| pending.added), Some(1));
    finder.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert_eq!(finder.pending(), None, "ctrl+r shows them");
}

#[test]
fn each_display_mode_draws_what_it_names() {
    let items = vec![Item::new("/home/user/code/polygit", Kind::Repo)];
    let mut config = quiet();

    config.display = Display::Name;
    let mut finder = Finder::with_items(items.clone(), config.clone());
    let (buffer, _) = painted(&mut finder, 40, 3);
    assert_eq!(row_text(&buffer, 1), "polygit");

    config.display = Display::Path;
    let mut finder = Finder::with_items(items.clone(), config.clone());
    let (buffer, _) = painted(&mut finder, 40, 3);
    assert_eq!(row_text(&buffer, 1), "~/code/polygit", "home as ~");

    config.tilde = false;
    let mut finder = Finder::with_items(items.clone(), config.clone());
    let (buffer, _) = painted(&mut finder, 40, 3);
    assert_eq!(row_text(&buffer, 1), "/home/user/code/polygit");
    config.tilde = true;

    config.display = Display::NameAndPath;
    let mut finder = Finder::with_items(items.clone(), config.clone());
    let (buffer, _) = painted(&mut finder, 40, 3);
    let row = row_text(&buffer, 1);
    assert!(row.starts_with("polygit "), "{row:?}");
    assert!(row.ends_with("~/code/polygit"), "{row:?}");
    assert_eq!(
        row.chars().count(),
        38,
        "the path ends at the content's right edge, clear of the bar's two columns"
    );

    config.display = Display::TwoRows;
    let mut finder = Finder::with_items(items, config);
    let (buffer, geometry) = painted(&mut finder, 40, 5);
    assert_eq!(row_text(&buffer, 1), "polygit");
    assert_eq!(row_text(&buffer, 2), "  ~/code/polygit");
    assert_eq!(
        geometry.rows[0].0.height, 2,
        "one item is two rows to hit-test"
    );
    assert_eq!(geometry.item_at(5, 2), Some(0));
}

#[test]
fn a_narrow_row_cuts_the_path_from_the_left() {
    let mut config = quiet();
    config.display = Display::NameAndPath;
    let mut finder = Finder::with_items(
        vec![Item::new(
            "/home/user/a/very/long/path/to/polygit",
            Kind::Repo,
        )],
        config,
    );
    let (buffer, _) = painted(&mut finder, 30, 3);
    let row = row_text(&buffer, 1);
    assert!(row.starts_with("polygit  "), "{row:?}");
    assert!(
        row.ends_with("/to/polygit"),
        "the end of the path survives: {row:?}"
    );
    assert!(row.contains('…'), "{row:?}");
}

#[test]
fn a_query_highlights_the_name_and_the_path_where_it_matched() {
    let mut config = quiet();
    config.display = Display::NameAndPath;
    let mut finder = Finder::with_items(
        repos(&["/home/user/zed/polygit", "/home/user/other"]),
        config,
    );
    for character in "pgit".chars() {
        finder.push(character);
    }
    assert_eq!(listed(&finder), ["/home/user/zed/polygit"]);
    let (buffer, _) = painted(&mut finder, 40, 3);
    let accent: String = (0..40)
        .filter(|x| buffer[(*x, 1)].fg == DEFAULT_DARK.accent)
        .map(|x| buffer[(x, 1)].symbol().to_string())
        .collect();
    assert_eq!(accent, "pgitpgit", "once in the name and once in the path");
}

#[test]
fn the_order_before_a_query_follows_the_sort_and_a_query_breaks_ties_by_it() {
    let dir = scratch("sort");
    let history = dir.join("history");
    std::fs::write(&history, "10\t/r/beta\n20\t/r/beta\n30\t/r/alpha\n").expect("written");
    let mut config = quiet();
    config.history = Some(history);
    let mut finder = Finder::with_items(repos(&["/r/gamma", "/r/beta", "/r/alpha"]), config);

    assert_eq!(
        listed(&finder),
        ["/r/gamma", "/r/beta", "/r/alpha"],
        "relevance keeps source order"
    );
    finder.set_sort(SortMode::Name);
    assert_eq!(listed(&finder), ["/r/alpha", "/r/beta", "/r/gamma"]);
    finder.set_sort(SortMode::Recent);
    assert_eq!(listed(&finder), ["/r/alpha", "/r/beta", "/r/gamma"]);
    finder.set_sort(SortMode::MostUsed);
    assert_eq!(listed(&finder), ["/r/beta", "/r/alpha", "/r/gamma"]);

    // `alpha` scores highest (its `a` starts a word); `beta` and `gamma` score alike, and keep the
    // most-used order between them rather than the order the sources gave.
    finder.push('a');
    assert_eq!(listed(&finder), ["/r/alpha", "/r/beta", "/r/gamma"]);
    finder.set_sort(SortMode::Relevance);
    assert_eq!(listed(&finder), ["/r/alpha", "/r/gamma", "/r/beta"]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn keys_type_move_pick_and_count_the_pick() {
    let dir = scratch("keys");
    let history = dir.join("history");
    let mut config = quiet();
    config.history = Some(history.clone());
    let mut finder = Finder::with_items(repos(&["/r/one", "/r/two"]), config);
    let press = |finder: &mut Finder, code| finder.on_key(KeyEvent::new(code, KeyModifiers::NONE));

    assert_eq!(press(&mut finder, KeyCode::Down), Outcome::Open);
    assert_eq!(finder.selected().expect("a cursor").key(), "/r/two");
    press(&mut finder, KeyCode::Char('o'));
    press(&mut finder, KeyCode::Char('n'));
    assert_eq!(finder.query(), "on");
    assert_eq!(listed(&finder), ["/r/one"]);
    assert_eq!(
        press(&mut finder, KeyCode::Enter),
        Outcome::Picked(Item::new("/r/one", Kind::Repo))
    );
    assert_eq!(finder.uses(&Item::new("/r/one", Kind::Repo)), 1);
    assert!(
        std::fs::read_to_string(&history)
            .expect("appended")
            .ends_with("\t/r/one\n")
    );
    assert_eq!(
        finder.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
        Outcome::Open
    );
    assert_eq!(finder.query(), "on", "an alt key is the host's");
    assert_eq!(press(&mut finder, KeyCode::Esc), Outcome::Cancelled);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_file_finder_lists_under_its_root_and_counts_the_full_path() {
    let dir = scratch("files");
    std::fs::create_dir_all(dir.join("src")).expect("built");
    std::fs::write(dir.join("src/lib.rs"), "").expect("written");
    std::fs::write(dir.join("README.md"), "").expect("written");
    let history = dir.join("history");
    let mut config = quiet();
    config.history = Some(history.clone());
    config.display = Display::Path;
    let mut finder = Finder::files(dir.clone(), config);
    assert_eq!(finder.files_root(), Some(dir.as_path()));
    assert_eq!(listed(&finder), ["README.md", "src/lib.rs"]);
    finder.push('l');
    finder.push('i');
    finder.push('b');
    let picked = finder.pick().expect("a pick");
    assert_eq!(picked, Item::new("src/lib.rs", Kind::File));
    let written = std::fs::read_to_string(&history).expect("appended");
    assert!(
        written.ends_with(&format!("\t{}\n", dir.join("src/lib.rs").display())),
        "{written:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn kind_and_count_columns_appear_only_when_asked() {
    let dir = scratch("columns");
    let history = dir.join("history");
    std::fs::write(&history, "1\t/r/one\n2\t/r/one\n").expect("written");
    let mut config = quiet();
    config.history = Some(history);
    config.display = Display::Name;
    let mut finder = Finder::with_items(repos(&["/r/one"]), config.clone());
    let (buffer, _) = painted(&mut finder, 30, 3);
    assert_eq!(row_text(&buffer, 1), "one");

    config.show_kind = true;
    config.show_count = true;
    let mut finder = Finder::with_items(repos(&["/r/one"]), config);
    let (buffer, _) = painted(&mut finder, 30, 3);
    assert_eq!(row_text(&buffer, 1), "repo   2 one");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_zero_rect_draws_nothing_and_says_so() {
    let mut finder = Finder::with_items(repos(&["/r/one"]), quiet());
    let mut terminal = Terminal::new(TestBackend::new(10, 3)).expect("a terminal");
    terminal
        .draw(|frame| {
            let geometry = draw(
                frame,
                Rect::new(0, 0, 0, 0),
                &mut finder,
                &DEFAULT_DARK,
                "s",
            );
            assert_eq!(geometry, Geometry::default());
        })
        .expect("drawn");
}
