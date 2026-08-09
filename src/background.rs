//! Whether the terminal is dark or light, and **which signal said so**.
//!
//! Asking the terminal directly is the right first question and it is the one that most often gets no
//! answer: OSC 11 goes unimplemented by a large minority of terminals, and the ones that follow the
//! desktop's light/dark setting are exactly the ones that tend not to reply. So there are four signals,
//! tried in order, and the answer carries which of them produced it.
//!
//! That last part is the whole reason this module exists rather than a one-line call. A resolver that
//! returns a bare mode cannot distinguish *the terminal said dark* from *nothing answered, so dark* —
//! and the second is a guess. Where the guess is wrong the UI is simply the wrong polarity, with
//! nothing anywhere to suggest a cause, so nobody looks. [`Reading::source`] is what makes it
//! inspectable, and an application that prints it in a diagnostics command turns a silent wrong guess
//! into a sentence someone can read.
//!
//! Because the probes run in a fixed order, the source also says which signals *declined*: everything
//! ahead of it in the list that was asked.
//!
//! ```no_run
//! let reading = tuilith::background::read();
//! println!("{}, according to {}", reading.mode.label(), reading.source.label());
//! ```
//!
//! # What these signals actually answer
//!
//! Only [`Source::Osc`] describes the terminal. The other three describe the *desktop*, which is a
//! different question with a usually-matching answer: a reader running a deliberately dark terminal on a
//! light desktop is told light, confidently and wrongly, and nothing here can detect it.
//! [`Source::describes_the_terminal`] is how a caller tells the two apart, and it is why a desktop answer
//! is worth reporting rather than hiding behind the word "detected".
//!
//! Two scope limits worth knowing. The Windows setting is read only under WSL — a native Windows build
//! falls through to the fallback even though the same registry value is readable there. And
//! `COLORFGBG` is fixed for a process's lifetime, so it can answer at startup but can never report a
//! change.
//!
//! # When to call which
//!
//! **[`read`] before the alternate screen is entered.** The OSC probe writes an escape sequence and reads
//! the reply from the same terminal, so asking once the UI owns the screen prints the answer into the
//! frame. It blocks: up to [`PROBE_TIMEOUT`] for the terminal, and the same again per desktop probe.
//!
//! [`read_while_running`] omits the OSC query so it can be called from a live event loop. It is not free
//! — on WSL each call spawns a process, tens of milliseconds when the interop path is healthy — so it
//! belongs on a worker rather than on the thread that draws.

#[cfg(feature = "os-appearance")]
use std::process::{Command, Output};
use std::time::Duration;

use crate::theme::Mode;

crate::provenance! {
    component: "background",
    about: "Terminal dark/light resolution over four ordered signals, reporting which one answered",
    origin: crate::Origin::Repo("polygit"),
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// How long any one signal may take before it is treated as no answer.
///
/// Per probe, not per call: [`read`] asks up to four, and [`windows_appearance`] tries two paths, so the
/// worst case for a whole call is several times this. It is applied to the terminal query too — that one
/// defaults to a full second, which would otherwise be the largest term in a budget written to keep the
/// first frame prompt.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// Which signal answered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// The terminal itself, over OSC 11. The only signal that describes the actual background.
    Osc,
    /// The `COLORFGBG` environment variable, which a few terminals set. Fixed once the process starts.
    ColorFgBg,
    /// The Windows "apps use light theme" setting, read under WSL. Terminals that follow the desktop
    /// theme track this value, and are largely the ones that do not answer OSC 11.
    WindowsRegistry,
    /// The macOS appearance setting.
    MacOsDefaults,
    /// Nothing answered, so the mode is the fallback guess rather than an observation.
    Nobody,
}

impl Source {
    /// A short phrase naming it, for a diagnostics line.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Osc => "the terminal, over OSC 11",
            Self::ColorFgBg => "the COLORFGBG variable",
            Self::WindowsRegistry => "the Windows appearance setting",
            Self::MacOsDefaults => "the macOS appearance setting",
            Self::Nobody => "nothing — this is the fallback",
        }
    }

    /// Whether anything answered at all, as opposed to the mode being the fallback guess.
    #[must_use]
    pub fn answered(self) -> bool {
        self != Self::Nobody
    }

    /// Whether the answer came from the terminal itself rather than from the desktop around it.
    ///
    /// The distinction that matters, and the one a caller is most likely to flatten. A desktop setting is
    /// a good proxy for the terminal's background and it is not the same fact: a dark terminal on a light
    /// desktop reports light here, and no signal in this module can notice.
    #[must_use]
    pub fn describes_the_terminal(self) -> bool {
        self == Self::Osc
    }
}

/// A mode, and what produced it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reading {
    /// The mode to draw in. Always usable, whether or not anything answered.
    pub mode: Mode,
    /// Which signal produced it, or [`Source::Nobody`] when it is the fallback.
    pub source: Source,
}

/// A signal, and the probe that reads it.
///
/// A function pointer rather than a closure so the order can be expressed as data — see
/// [`first_answer`], which is what makes the *order* testable rather than only each parse.
type Probe = (Source, fn() -> Option<Mode>);

/// Every signal, most authoritative first.
const AT_STARTUP: &[Probe] = &[
    (Source::Osc, osc),
    (Source::ColorFgBg, colorfgbg),
    (Source::WindowsRegistry, windows_appearance),
    (Source::MacOsDefaults, macos_appearance),
];

/// The signals that can be read while the terminal is in use, in the order that can observe a change.
///
/// `COLORFGBG` moves to the back rather than keeping its startup position. It cannot change within a
/// process, so consulting it first would answer every poll with the same value and the desktop probes
/// below it would never run — which is precisely the change this list exists to notice.
const WHILE_RUNNING: &[Probe] = &[
    (Source::WindowsRegistry, windows_appearance),
    (Source::MacOsDefaults, macos_appearance),
    (Source::ColorFgBg, colorfgbg),
];

/// The first signal that answers, or the dark fallback.
///
/// Dark is the safer guess on an unknown background: light text on an unknown background is more often
/// readable than dark text on one.
fn first_answer(probes: &[Probe]) -> Reading {
    for (source, probe) in probes {
        if let Some(mode) = probe() {
            return Reading {
                mode,
                source: *source,
            };
        }
    }
    Reading {
        mode: Mode::Dark,
        source: Source::Nobody,
    }
}

/// Ask every signal, most authoritative first.
///
/// **Call this before entering the alternate screen.** The OSC probe reads its reply from the terminal,
/// so asking afterwards prints the answer into the frame.
#[must_use]
pub fn read() -> Reading {
    first_answer(AT_STARTUP)
}

/// Ask only the signals that do not touch the terminal, for a running event loop.
///
/// Skips OSC 11, so it cannot see a background the terminal reports only that way — such a terminal
/// reads as [`Source::Nobody`] here even though [`read`] had an answer for it. This is how an
/// application follows a desktop light/dark switch without restarting; see the module docs for what it
/// costs, which is not nothing.
#[must_use]
pub fn read_while_running() -> Reading {
    first_answer(WHILE_RUNNING)
}

/// The terminal's own answer, over OSC 11.
fn osc() -> Option<Mode> {
    // Assigned rather than built in one expression because `QueryOptions` is `#[non_exhaustive]`.
    let mut options = terminal_colorsaurus::QueryOptions::default();
    options.timeout = PROBE_TIMEOUT;
    match terminal_colorsaurus::theme_mode(options) {
        Ok(terminal_colorsaurus::ThemeMode::Light) => Some(Mode::Light),
        Ok(terminal_colorsaurus::ThemeMode::Dark) => Some(Mode::Dark),
        Err(_) => None,
    }
}

/// `COLORFGBG`, when the terminal sets it.
fn colorfgbg() -> Option<Mode> {
    colorfgbg_mode(&std::env::var("COLORFGBG").ok()?)
}

/// Parse a `COLORFGBG` value — `"15;0"`, or `"15;default;0"`.
///
/// The last segment is the background's colour index within the sixteen ANSI colours: 0–6 and 8 are the
/// dark half, 7 and 9–15 the light half. Anything above 15 is refused rather than guessed — those are
/// 256-colour indices whose lightness the sixteen-colour rule says nothing about, and half of them are
/// dark, so treating them as light would guess wrong in the direction that matters.
pub(crate) fn colorfgbg_mode(raw: &str) -> Option<Mode> {
    let background: u8 = raw.rsplit(';').next()?.trim().parse().ok()?;
    if background > 15 {
        return None;
    }
    Some(if background <= 6 || background == 8 {
        Mode::Dark
    } else {
        Mode::Light
    })
}

/// Where `reg.exe` is, under a WSL distribution.
///
/// Absolute paths only. Resolving a bare `reg.exe` through `$PATH` would execute whatever a
/// user-writable directory earlier in the path happens to hold, at startup, in every application that
/// links this crate.
#[cfg(feature = "os-appearance")]
const REG_PATHS: &[&str] = &[
    "/mnt/c/Windows/System32/reg.exe",
    "/c/Windows/System32/reg.exe",
    "/windows/c/Windows/System32/reg.exe",
];

/// Under WSL, the Windows "apps use light theme" setting.
#[cfg(feature = "os-appearance")]
fn windows_appearance() -> Option<Mode> {
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

    if std::env::var_os("WSL_DISTRO_NAME").is_none() && std::env::var_os("WSL_INTEROP").is_none() {
        return None;
    }
    for program in REG_PATHS {
        if !std::path::Path::new(program).exists() {
            continue;
        }
        let mut command = Command::new(program);
        command.args(["query", KEY, "/v", "AppsUseLightTheme"]);
        // A failed `reg.exe` prints an error message that still mentions the value name, and the parser
        // takes the last token of any line that does — so the success check is what stops an error being
        // read as an answer.
        if let Some(output) =
            output_within(command, PROBE_TIMEOUT).filter(|done| done.status.success())
        {
            return reg_output_mode(&String::from_utf8_lossy(&output.stdout));
        }
    }
    None
}

#[cfg(not(feature = "os-appearance"))]
fn windows_appearance() -> Option<Mode> {
    None
}

/// Parse `reg.exe query` output: `AppsUseLightTheme    REG_DWORD    0x1` means light.
#[cfg(feature = "os-appearance")]
pub(crate) fn reg_output_mode(output: &str) -> Option<Mode> {
    let line = output
        .lines()
        .find(|line| line.contains("AppsUseLightTheme") && line.contains("REG_DWORD"))?;
    match line.split_whitespace().last()? {
        "0x0" => Some(Mode::Dark),
        "0x1" => Some(Mode::Light),
        _ => None,
    }
}

/// On macOS, the appearance setting.
#[cfg(feature = "os-appearance")]
fn macos_appearance() -> Option<Mode> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut command = Command::new("defaults");
    command.args(["read", "-g", "AppleInterfaceStyle"]);
    // The exit status is deliberately ignored here, unlike the Windows probe. `defaults` exits non-zero
    // with empty output when the key is absent, and an absent key *is* light mode — so requiring success
    // would make light unreachable on macOS and send every light-mode reader to the dark fallback.
    let output = output_within(command, PROBE_TIMEOUT)?;
    Some(apple_output_mode(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(not(feature = "os-appearance"))]
fn macos_appearance() -> Option<Mode> {
    None
}

/// Parse `defaults read -g AppleInterfaceStyle`: it prints `Dark` in dark mode and nothing in light.
#[cfg(feature = "os-appearance")]
pub(crate) fn apple_output_mode(output: &str) -> Mode {
    if output.contains("Dark") {
        Mode::Dark
    } else {
        Mode::Light
    }
}

/// A command's result, or `None` if it could not be started or did not finish in time.
///
/// `Command::output` cannot be given a deadline, so the wait happens on another thread and this one gives
/// up. The caller decides what a non-zero exit means, because the two probes disagree about that.
///
/// The cost of a timeout, stated because it is not small: the thread is detached and the child is never
/// reaped, since the `Command` moved into the thread and no handle survives. Both outlive this call, and
/// on the failure this exists for — a wedged WSL interop socket — they can outlive the process. Even the
/// tidier shape could not fix that, because killing the Linux-side stub does not touch the Windows
/// process behind it. So the deadline buys a prompt first frame, not a clean one.
#[cfg(feature = "os-appearance")]
fn output_within(mut command: Command, within: Duration) -> Option<Output> {
    use std::sync::mpsc;

    command
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let (send, receive) = mpsc::channel();
    // `Builder` rather than `thread::spawn`, which panics when the OS refuses a thread. A probe must
    // degrade to "no answer" there, not take down an application that asked about a colour.
    std::thread::Builder::new()
        .name(String::from("tuilith-appearance-probe"))
        .spawn(move || {
            let _ = send.send(command.output());
        })
        .ok()?;
    receive.recv_timeout(within).ok()?.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorfgbg_reads_the_last_segment_as_the_background() {
        assert_eq!(colorfgbg_mode("15;0"), Some(Mode::Dark));
        assert_eq!(colorfgbg_mode("0;15"), Some(Mode::Light));
        assert_eq!(colorfgbg_mode("15;default;0"), Some(Mode::Dark));
        assert_eq!(colorfgbg_mode("15;8"), Some(Mode::Dark));
        assert_eq!(colorfgbg_mode("0;7"), Some(Mode::Light));
        assert_eq!(colorfgbg_mode(" 15 ; 0 "), Some(Mode::Dark));
    }

    #[test]
    fn a_colorfgbg_that_names_no_background_is_no_answer() {
        assert_eq!(colorfgbg_mode(""), None);
        assert_eq!(colorfgbg_mode("default"), None);
        assert_eq!(colorfgbg_mode("15;default"), None);
    }

    #[test]
    fn a_256_colour_background_is_refused_rather_than_called_light() {
        // The rule this parser implements covers the sixteen ANSI colours. Index 16 is pure black and 232
        // is nearly so, and both are above the light half's range — so a rule that returned Light for
        // anything over 15 would put dark text on a black terminal.
        for dark_but_high in ["15;16", "15;232", "15;235", "15;243"] {
            assert_eq!(
                colorfgbg_mode(dark_but_high),
                None,
                "{dark_but_high} was answered rather than refused"
            );
        }
        // …and the refusal is a range check, not an integer overflow, which is what the 999 case proves.
        assert_eq!(colorfgbg_mode("15;999"), None);
        assert_eq!(colorfgbg_mode("15;255"), None);
    }

    #[cfg(feature = "os-appearance")]
    #[test]
    fn the_windows_setting_reads_light_as_light() {
        // `0x1` is the value on a machine whose Windows is in light mode, which is the case this whole
        // chain exists for: such a terminal usually does not answer OSC 11.
        let light = "\r\nHKEY_CURRENT_USER\\...\\Personalize\r\n    AppsUseLightTheme    REG_DWORD    0x1\r\n";
        let dark = "\r\nHKEY_CURRENT_USER\\...\\Personalize\r\n    AppsUseLightTheme    REG_DWORD    0x0\r\n";
        assert_eq!(reg_output_mode(light), Some(Mode::Light));
        assert_eq!(reg_output_mode(dark), Some(Mode::Dark));
    }

    #[cfg(feature = "os-appearance")]
    #[test]
    fn output_that_is_not_a_value_row_is_no_answer() {
        assert_eq!(reg_output_mode(""), None);
        // An error message can mention the value name and end in something that parses, so the parser
        // requires the row's type as well as its name.
        assert_eq!(
            reg_output_mode("ERROR: cannot find AppsUseLightTheme value 0x1"),
            None
        );
        // The shape a UTF-16 reply degrades to once it has been through `from_utf8_lossy`: the value is
        // unreadable, so it must read as "could not tell" rather than as a mode.
        assert_eq!(
            reg_output_mode("A\0p\0p\0s\0U\0s\0e\0L\0i\0g\0h\0t\0T\0h\0e\0m\0e\0"),
            None
        );
    }

    #[cfg(feature = "os-appearance")]
    #[test]
    fn the_macos_setting_treats_a_silent_reply_as_light() {
        // `defaults` prints nothing and exits non-zero in light mode, because the key is simply absent.
        // So the empty string is the light answer rather than a failure, and the probe above must not
        // require a successful exit or light becomes unreachable there.
        assert_eq!(apple_output_mode("Dark\n"), Mode::Dark);
        assert_eq!(apple_output_mode(""), Mode::Light);
    }

    #[test]
    fn the_first_signal_that_answers_wins_and_is_named() {
        // Coerced closures rather than `fn` items: a probe's signature has to be `fn() -> Option<Mode>`
        // whether or not a given one can decline, and a bare `fn` returning only `Some` reads to clippy
        // as a return type that should be unwrapped.
        let light: fn() -> Option<Mode> = || Some(Mode::Light);
        let dark: fn() -> Option<Mode> = || Some(Mode::Dark);
        let silent: fn() -> Option<Mode> = || None;

        let reading = first_answer(&[
            (Source::Osc, silent),
            (Source::ColorFgBg, light),
            (Source::WindowsRegistry, dark),
        ]);
        assert_eq!(reading.mode, Mode::Light);
        assert_eq!(reading.source, Source::ColorFgBg, "order was not honoured");
        assert!(reading.source.answered());

        // Every source has to be attributable, not just the one that happens to sit second. A win by a
        // later probe is the case a reordering bug would otherwise hide.
        let reading = first_answer(&[
            (Source::Osc, silent),
            (Source::ColorFgBg, silent),
            (Source::WindowsRegistry, dark),
            (Source::MacOsDefaults, light),
        ]);
        assert_eq!(reading.mode, Mode::Dark);
        assert_eq!(reading.source, Source::WindowsRegistry);
    }

    #[test]
    fn nothing_answering_is_dark_and_says_it_was_a_guess() {
        let silent: fn() -> Option<Mode> = || None;
        let reading = first_answer(&[(Source::Osc, silent), (Source::ColorFgBg, silent)]);
        assert_eq!(reading.mode, Mode::Dark);
        assert_eq!(reading.source, Source::Nobody);
        assert!(
            !reading.source.answered(),
            "a fallback must not read as an observation"
        );
        assert_eq!(first_answer(&[]).source, Source::Nobody);
    }

    #[test]
    fn only_the_terminal_is_reported_as_describing_the_terminal() {
        // The desktop signals answer a nearby question, and flattening the two is how a dark terminal on
        // a light desktop gets told it is light.
        assert!(Source::Osc.describes_the_terminal());
        for desktop in [
            Source::ColorFgBg,
            Source::WindowsRegistry,
            Source::MacOsDefaults,
            Source::Nobody,
        ] {
            assert!(
                !desktop.describes_the_terminal(),
                "{desktop:?} claims to describe the terminal"
            );
        }
    }

    #[test]
    fn the_terminal_is_asked_first_at_startup_and_never_while_running() {
        assert_eq!(AT_STARTUP[0].0, Source::Osc);
        assert!(
            !WHILE_RUNNING
                .iter()
                .any(|(source, _)| source.describes_the_terminal()),
            "the OSC query reads the terminal and cannot run while the UI owns it"
        );
        // Every running probe is a startup probe too — the running list is a subset, not a second policy.
        for (source, _) in WHILE_RUNNING {
            assert!(
                AT_STARTUP
                    .iter()
                    .any(|(at_startup, _)| at_startup == source),
                "{source:?} is polled but never asked at startup"
            );
        }
        // And the one signal that cannot change goes last there, or it answers every poll and the probes
        // that *can* observe a change never run.
        assert_eq!(
            WHILE_RUNNING.last().map(|(source, _)| *source),
            Some(Source::ColorFgBg)
        );
    }
}
