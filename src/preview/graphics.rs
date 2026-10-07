//! Which graphics protocol this terminal can actually show.
//!
//! A multiplexer answers terminal queries about itself, not about the terminal hosting it, so the
//! environment is read before any query is believed. herdr (`TERM_PROGRAM=herdr`) answers DA1 without
//! sixel and accepts kitty graphics into its own pane, then forwards them only to some hosts — a kitty
//! query there says nothing about whether the picture reaches the screen, so it is never asked. tmux,
//! GNU screen and zellij are settled the same way, and a terminal that names itself in the environment
//! (kitty, ghostty, `WezTerm`, Windows Terminal) is believed without a query.
//!
//! [`decide`] is pure, so every row of that table is a test. [`probe::detect`](super::probe::detect)
//! runs it, asks the terminal only when it says so, and decides again with the answers.

use std::fmt;

crate::provenance! {
    component: "preview::graphics",
    about: "Which graphics protocol a terminal can show, decided from its environment before any query is believed",
    origin: crate::Origin::Private,
    lineage: crate::Lineage::Inspired {
        by: "yazi's adapter and emulator driver matrix",
    },
    since: "0.1",
}

/// A way of putting pixels on the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// Kitty graphics, placed with unicode placeholders.
    Kitty,
    /// DEC sixel.
    Sixel,
    /// Two pixels per cell with `▀` and colours: always works, never readable for text.
    Halfblocks,
    /// No pictures: metadata cards only.
    None,
}

impl Protocol {
    /// Every protocol, in the order a settings list offers them.
    pub const ALL: [Self; 4] = [Self::Kitty, Self::Sixel, Self::Halfblocks, Self::None];

    /// The word for it, as a flag or an environment variable spells it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Kitty => "kitty",
            Self::Sixel => "sixel",
            Self::Halfblocks => "halfblocks",
            Self::None => "none",
        }
    }

    /// The protocol a word names, ignoring case. `None` for a word that names none.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|protocol| protocol.label().eq_ignore_ascii_case(word.trim()))
    }

    /// Whether this protocol draws pictures at all.
    #[must_use]
    pub fn draws_pictures(self) -> bool {
        !matches!(self, Self::None)
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// One character cell's size in pixels — what an encoder sizes a picture by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellSize {
    /// Pixels across.
    pub width: u16,
    /// Pixels down.
    pub height: u16,
}

impl CellSize {
    /// The size assumed when the terminal reports none: a common 1:2 cell.
    pub const FALLBACK: Self = Self {
        width: 10,
        height: 20,
    };

    /// A cell size, when both sides are non-zero.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Option<Self> {
        (width > 0 && height > 0).then_some(Self { width, height })
    }

    /// The cell size the kernel reports for the controlling terminal (`TIOCGWINSZ`), when it reports
    /// pixels at all — many terminals leave those fields zero, and Windows has none.
    #[must_use]
    pub fn from_window() -> Option<Self> {
        let window = crossterm::terminal::window_size().ok()?;
        if window.columns == 0 || window.rows == 0 {
            return None;
        }
        Self::new(window.width / window.columns, window.height / window.rows)
    }
}

impl fmt::Display for CellSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}×{} px", self.width, self.height)
    }
}

/// What the environment says about the terminal, read once at start.
// Each field is one independent environment signal; folding them into an enum would lose the ones that
// appear together, such as a stale `KITTY_WINDOW_ID` inside tmux.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    /// `TERM`.
    pub term: String,
    /// `TERM_PROGRAM`.
    pub term_program: String,
    /// `TMUX` is set.
    pub tmux: bool,
    /// `STY` is set: GNU screen.
    pub screen: bool,
    /// `ZELLIJ` is set.
    pub zellij: bool,
    /// `WT_SESSION` is set: Windows Terminal.
    pub windows_terminal: bool,
    /// `KITTY_WINDOW_ID` is set.
    pub kitty_window: bool,
    /// `WEZTERM_PANE` is set.
    pub wezterm_pane: bool,
}

impl Env {
    /// Read from this process's environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self::from_lookup(&|name| std::env::var(name).ok())
    }

    /// Read through `lookup`, which answers a variable's value or `None` when it is unset.
    #[must_use]
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Self {
        let set = |name: &str| lookup(name).is_some_and(|value| !value.is_empty());
        let text = |name: &str| lookup(name).unwrap_or_default();
        Self {
            term: text("TERM"),
            term_program: text("TERM_PROGRAM"),
            tmux: set("TMUX"),
            screen: set("STY"),
            zellij: set("ZELLIJ"),
            windows_terminal: set("WT_SESSION"),
            kitty_window: set("KITTY_WINDOW_ID"),
            wezterm_pane: set("WEZTERM_PANE"),
        }
    }

    /// Inside a multiplexer, whose answers to a query describe itself rather than the terminal.
    #[must_use]
    pub fn multiplexed(&self) -> bool {
        self.term_program.eq_ignore_ascii_case("herdr") || self.tmux || self.screen || self.zellij
    }
}

/// The terminal's answers to a kitty graphics query and to DA1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    /// The kitty query (`a=q`) was acknowledged with `OK`.
    pub kitty: bool,
    /// DA1 listed attribute `4`.
    pub sixel: bool,
}

/// The chosen protocol and why, so a diagnostics line can show both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// What to draw with.
    pub protocol: Protocol,
    /// Why, in a phrase.
    pub reason: &'static str,
    /// The environment settled nothing and no probe has run yet: query the terminal, then decide again.
    pub needs_probe: bool,
}

const fn decided(protocol: Protocol, reason: &'static str) -> Decision {
    Decision {
        protocol,
        reason,
        needs_probe: false,
    }
}

/// Decide from an explicit choice, the environment, and — only when the environment leaves it open —
/// the terminal's answers.
///
/// `choice` is the caller's override (a flag, a setting) and wins over everything. `is_terminal` is
/// whether output reaches a terminal at all.
#[must_use]
pub fn decide(
    choice: Option<Protocol>,
    env: &Env,
    is_terminal: bool,
    probe: Option<Probe>,
) -> Decision {
    if let Some(protocol) = choice {
        return decided(protocol, "chosen by the caller");
    }
    if !is_terminal {
        return decided(Protocol::None, "output is not a terminal");
    }
    if env.term_program.eq_ignore_ascii_case("herdr") {
        return decided(
            Protocol::Halfblocks,
            "inside herdr: the pane answers graphics queries itself and forwards kitty to some hosts only",
        );
    }
    if env.tmux || env.screen || env.zellij {
        return decided(
            Protocol::Halfblocks,
            "inside a multiplexer, whose answers describe itself rather than the terminal",
        );
    }
    let program = env.term_program.to_ascii_lowercase();
    if env.kitty_window
        || env.wezterm_pane
        || env.term == "xterm-kitty"
        || env.term == "xterm-ghostty"
        || program == "wezterm"
        || program == "ghostty"
    {
        return decided(
            Protocol::Kitty,
            "a terminal that implements kitty graphics, named by its environment",
        );
    }
    if env.windows_terminal {
        return decided(Protocol::Sixel, "Windows Terminal renders sixel");
    }
    match probe {
        Some(Probe { kitty: true, .. }) => {
            decided(Protocol::Kitty, "the terminal acknowledged a kitty query")
        }
        Some(Probe { sixel: true, .. }) => {
            decided(Protocol::Sixel, "the terminal's DA1 lists sixel")
        }
        Some(_) => decided(
            Protocol::Halfblocks,
            "the terminal claims neither kitty nor sixel",
        ),
        None => Decision {
            protocol: Protocol::Halfblocks,
            reason: "not yet probed",
            needs_probe: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lying() -> Option<Probe> {
        Some(Probe {
            kitty: true,
            sixel: true,
        })
    }

    #[test]
    fn a_choice_wins_over_everything() {
        let env = Env {
            term_program: "herdr".into(),
            ..Env::default()
        };
        let decision = decide(Some(Protocol::Sixel), &env, true, lying());
        assert_eq!(decision.protocol, Protocol::Sixel);
        assert!(!decision.needs_probe);
    }

    #[test]
    fn inside_herdr_a_lying_terminal_still_gets_halfblocks() {
        let env = Env {
            term_program: "herdr".into(),
            term: "xterm-256color".into(),
            ..Env::default()
        };
        let decision = decide(None, &env, true, lying());
        assert_eq!(decision.protocol, Protocol::Halfblocks);
        assert!(!decision.needs_probe, "herdr's answers are never asked for");
        assert!(env.multiplexed());
    }

    #[test]
    fn inside_tmux_a_stale_kitty_window_id_is_ignored() {
        let env = Env {
            tmux: true,
            kitty_window: true,
            ..Env::default()
        };
        assert_eq!(
            decide(None, &env, true, lying()).protocol,
            Protocol::Halfblocks
        );
    }

    #[test]
    fn screen_and_zellij_are_settled_without_asking() {
        for env in [
            Env {
                screen: true,
                ..Env::default()
            },
            Env {
                zellij: true,
                ..Env::default()
            },
        ] {
            let decision = decide(None, &env, true, None);
            assert_eq!(decision.protocol, Protocol::Halfblocks);
            assert!(!decision.needs_probe);
        }
    }

    #[test]
    fn windows_terminal_is_sixel_and_wezterm_is_kitty() {
        let windows = Env {
            windows_terminal: true,
            ..Env::default()
        };
        assert_eq!(decide(None, &windows, true, None).protocol, Protocol::Sixel);
        let wezterm = Env {
            term_program: "WezTerm".into(),
            ..Env::default()
        };
        assert_eq!(decide(None, &wezterm, true, None).protocol, Protocol::Kitty);
        let ghostty = Env {
            term: "xterm-ghostty".into(),
            ..Env::default()
        };
        assert_eq!(decide(None, &ghostty, true, None).protocol, Protocol::Kitty);
    }

    #[test]
    fn an_unknown_terminal_is_probed_then_believed() {
        let env = Env {
            term: "xterm-256color".into(),
            ..Env::default()
        };
        assert!(decide(None, &env, true, None).needs_probe);
        let sixel = Some(Probe {
            kitty: false,
            sixel: true,
        });
        assert_eq!(decide(None, &env, true, sixel).protocol, Protocol::Sixel);
        assert_eq!(decide(None, &env, true, lying()).protocol, Protocol::Kitty);
        let nothing = Some(Probe::default());
        assert_eq!(
            decide(None, &env, true, nothing).protocol,
            Protocol::Halfblocks
        );
    }

    #[test]
    fn not_a_terminal_shows_no_pictures() {
        assert_eq!(
            decide(None, &Env::default(), false, None).protocol,
            Protocol::None
        );
    }

    #[test]
    fn the_environment_is_read_by_name_and_an_empty_value_is_unset() {
        let env = Env::from_lookup(&|name| match name {
            "TERM_PROGRAM" => Some("herdr".to_owned()),
            "TMUX" => Some(String::new()),
            "WT_SESSION" => Some("0f".to_owned()),
            _ => None,
        });
        assert_eq!(env.term_program, "herdr");
        assert!(!env.tmux, "an empty TMUX is not a tmux session");
        assert!(env.windows_terminal);
        assert!(env.multiplexed());
    }

    #[test]
    fn a_label_parses_back_to_its_protocol() {
        for protocol in Protocol::ALL {
            assert_eq!(Protocol::parse(protocol.label()), Some(protocol));
        }
        assert_eq!(Protocol::parse(" Kitty "), Some(Protocol::Kitty));
        assert_eq!(Protocol::parse("iterm2"), None);
    }

    #[test]
    fn a_cell_size_needs_both_sides() {
        assert_eq!(CellSize::new(0, 20), None);
        assert_eq!(
            CellSize::new(9, 18),
            Some(CellSize {
                width: 9,
                height: 18
            })
        );
    }
}
