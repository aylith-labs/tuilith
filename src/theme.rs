//! Nine colour roles, as a light variant and a dark one.
//!
//! A theme is a **pair**, never a palette plus a background switch. With one palette and a toggle,
//! every colour chosen against dark is now on light — and the failure is not uniform: most roles stay
//! readable and two or three do not, which reads as a bug in those places rather than as the wrong
//! mode. Requiring both variants makes that state unrepresentable.
//!
//! The roles are semantic rather than literal. `ok`/`warn`/`bad` mean status, so a monochrome theme can
//! set all three to the foreground and let weight and symbols carry state — which is what a terminal
//! with no colour needs, and it needs no special case anywhere else.

use ratatui::style::Color;
use terminal_colorsaurus::{QueryOptions, ThemeMode};

crate::provenance! {
    component: "theme",
    about: "Nine semantic colour roles as a light and dark pair, with terminal background detection",
    origin: crate::Origin::Repo("a private repo"),
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// The nine roles every theme defines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The surface everything is drawn on.
    pub background: Color,
    /// Ordinary text.
    pub foreground: Color,
    /// Text that is present but secondary — a label, a count, a hint.
    pub dim: Color,
    /// Pane edges and rules.
    pub border: Color,
    /// The one colour that means "this, here": focus, selection, a key in a hint.
    pub accent: Color,
    /// The band behind a selected row.
    pub selection: Color,
    /// Status: succeeded.
    pub ok: Color,
    /// Status: needs attention.
    pub warn: Color,
    /// Status: failed.
    pub bad: Color,
}

/// The default dark variant.
pub const DEFAULT_DARK: Palette = Palette {
    background: Color::from_u32(0x000B_0F1A),
    foreground: Color::from_u32(0x00E6_EAF2),
    dim: Color::from_u32(0x0077_8295),
    border: Color::from_u32(0x0033_415C),
    accent: Color::from_u32(0x0033_66FF),
    selection: Color::from_u32(0x0018_233D),
    ok: Color::from_u32(0x0032_C0A0),
    warn: Color::from_u32(0x00FB_C02D),
    bad: Color::from_u32(0x00F3_565D),
};

/// The same roles against white.
///
/// The status colours are darkened rather than reused: a mint and a yellow tuned for a dark background
/// fail contrast on this one, which is exactly where it matters.
pub const DEFAULT_LIGHT: Palette = Palette {
    background: Color::from_u32(0x00FF_FFFF),
    foreground: Color::from_u32(0x001B_2333),
    dim: Color::from_u32(0x006B_7688),
    border: Color::from_u32(0x00D5_DAE3),
    accent: Color::from_u32(0x002B_4FCC),
    selection: Color::from_u32(0x00E7_EDFF),
    ok: Color::from_u32(0x0017_806A),
    warn: Color::from_u32(0x008A_6100),
    bad: Color::from_u32(0x00C0_303B),
};

/// Which variant to draw in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// Dark text on a light background.
    Light,
    /// Light text on a dark background — the fallback when the terminal says nothing.
    #[default]
    Dark,
}

impl Mode {
    /// What the terminal answers, or dark when it answers nothing.
    ///
    /// Dark is the safer guess on an unknown background: light text on an unknown background is more
    /// often readable than dark text on one.
    ///
    /// Query this **before** entering the alternate screen — the reply arrives on the same terminal,
    /// and asking after the UI is up prints the answer into the frame.
    #[must_use]
    pub fn detect() -> Self {
        match terminal_colorsaurus::theme_mode(QueryOptions::default()) {
            Ok(ThemeMode::Light) => Self::Light,
            _ => Self::Dark,
        }
    }

    /// The palette for this mode.
    #[must_use]
    pub fn palette(self) -> Palette {
        match self {
            Self::Light => DEFAULT_LIGHT,
            Self::Dark => DEFAULT_DARK,
        }
    }

    /// The other one, for a check that one variant has not leaked into the other.
    #[must_use]
    pub fn inverse(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_variants_define_every_role_differently_from_their_background() {
        for palette in [DEFAULT_DARK, DEFAULT_LIGHT] {
            for role in [
                palette.foreground,
                palette.dim,
                palette.border,
                palette.accent,
                palette.selection,
                palette.ok,
                palette.warn,
                palette.bad,
            ] {
                assert_ne!(
                    role, palette.background,
                    "a role invisible against its own background"
                );
            }
        }
    }

    #[test]
    fn the_two_variants_share_no_colour_so_a_leak_between_them_is_detectable() {
        // What makes the "did this frame draw in the wrong mode" test possible at all: if a role held
        // the same value in both, a leak in that role could not be seen.
        let dark = [
            DEFAULT_DARK.background,
            DEFAULT_DARK.foreground,
            DEFAULT_DARK.dim,
            DEFAULT_DARK.border,
            DEFAULT_DARK.accent,
            DEFAULT_DARK.selection,
            DEFAULT_DARK.ok,
            DEFAULT_DARK.warn,
            DEFAULT_DARK.bad,
        ];
        let light = [
            DEFAULT_LIGHT.background,
            DEFAULT_LIGHT.foreground,
            DEFAULT_LIGHT.dim,
            DEFAULT_LIGHT.border,
            DEFAULT_LIGHT.accent,
            DEFAULT_LIGHT.selection,
            DEFAULT_LIGHT.ok,
            DEFAULT_LIGHT.warn,
            DEFAULT_LIGHT.bad,
        ];
        for (role, colour) in dark.iter().enumerate() {
            assert_ne!(
                *colour, light[role],
                "role {role} is the same in both variants"
            );
        }
    }

    #[test]
    fn dark_is_what_an_unanswering_terminal_gets() {
        assert_eq!(Mode::default(), Mode::Dark);
        assert_eq!(Mode::Dark.palette(), DEFAULT_DARK);
        assert_eq!(Mode::Dark.inverse(), Mode::Light);
    }
}
