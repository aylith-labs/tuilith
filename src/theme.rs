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

impl Palette {
    /// Every role's colour, so a check can iterate the palette without naming nine fields.
    ///
    /// Order matches the struct's, but nothing should depend on it: this exists for membership tests —
    /// "is this colour one of ours" — not for indexing.
    #[must_use]
    pub fn roles(self) -> [Color; 9] {
        [
            self.background,
            self.foreground,
            self.dim,
            self.border,
            self.accent,
            self.selection,
            self.ok,
            self.warn,
            self.bad,
        ]
    }

    /// Which role a colour is, when it is one of this palette's.
    ///
    /// For a report about a rendered frame: a failure that says `dim on selection` is one somebody can
    /// act on, where the same failure quoting two hex values is one they have to decode first. A colour
    /// the palette does not contain has no name here, which is itself worth reporting.
    #[must_use]
    pub fn role_of(self, colour: Color) -> Option<&'static str> {
        const NAMES: [&str; 9] = [
            "background",
            "foreground",
            "dim",
            "border",
            "accent",
            "selection",
            "ok",
            "warn",
            "bad",
        ];
        self.roles()
            .iter()
            .position(|role| *role == colour)
            .map(|at| NAMES[at])
    }
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
    /// What the background signals answer, or dark when none of them does.
    ///
    /// Dark is the safer guess on an unknown background: light text on an unknown background is more
    /// often readable than dark text on one.
    ///
    /// Query this **before** entering the alternate screen — the OSC reply arrives on the same
    /// terminal, and asking after the UI is up prints the answer into the frame.
    ///
    /// This discards *which* signal answered. Use [`crate::background::read`] where that matters, which
    /// is anywhere the answer is shown to a person: a mode nobody observed is a guess, and a guess that
    /// cannot be told apart from an observation is one nobody will ever question.
    ///
    /// Needs the `background` feature, which is what supplies the signals. Without it there is nothing
    /// to detect from, and a `detect` that always answered `Dark` would be a guess wearing the name of
    /// an observation — the exact confusion the paragraph above exists to prevent.
    #[must_use]
    #[cfg(feature = "background")]
    pub fn detect() -> Self {
        crate::background::read().mode
    }

    /// The word for it, for a diagnostics line or a toggle's label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
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
            // Named rather than taken from `roles()`, which includes the background: skipping it by
            // value would make the assertion below a tautology that can never fail.
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
    fn no_colour_appears_in_both_variants_at_all_so_a_leak_between_them_is_detectable() {
        // What makes the "did this frame draw in the wrong mode" check possible: a colour shared by the
        // two palettes is one that check can never see.
        //
        // Set-wise rather than role-by-role, and the difference is the point. Comparing `dark.dim` only
        // with `light.dim` passes when `light.dim` equals `dark.border` — a real collision, and one that
        // would silently blind the leak check for both of those roles while this test stayed green.
        for dark in DEFAULT_DARK.roles() {
            for light in DEFAULT_LIGHT.roles() {
                assert_ne!(
                    dark, light,
                    "a colour in both palettes cannot be attributed to either"
                );
            }
        }
    }

    #[test]
    fn dark_is_what_an_unanswering_terminal_gets() {
        assert_eq!(Mode::default(), Mode::Dark);
        assert_eq!(Mode::Dark.palette(), DEFAULT_DARK);
        assert_eq!(Mode::Dark.inverse(), Mode::Light);
    }
}
