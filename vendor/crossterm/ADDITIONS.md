# crossterm, vendored: additions

Base: crossterm **0.29.0** (crates.io), file `src/event/sys/unix/parse.rs`. Compiled into
`tuilith::input` through a `#[path]` module; licence beside this file (`LICENSE`, MIT).

Everything a person types still parses exactly as upstream parsed it, into crossterm's own public
event types. The changes are all about the bytes a terminal sends in *reply* to a query, which
upstream either swallowed input over or typed into the application as keys. Each change is marked
`// tuilith:` at the site, and the added functions sit in one block above the tests.

To resync: diff upstream's newer `parse.rs` against this file's base, apply upstream's hunks, and keep
every `tuilith:` site.

## Changes

1. **Every `CSI ?` sequence ends at its final byte** (`parse_csi_private`). Upstream recognised only
   the `u` (keyboard flags) and `c` (DA1) finals and returned "need more bytes" for any other. The
   reader keeps a buffer it is told is incomplete and appends every later byte, so a single mode-2031
   report (`CSI ? 997 ; 1 n`) held all keyboard input until a `u` or `c` happened to arrive —
   crossterm-rs/crossterm#1106, and the reason #1052 has never been mergeable as-is. A sequence is
   now incomplete only while it is in its parameter and intermediate bytes (`0x20–0x3F`); any final
   byte (`0x40–0x7E`) ends it, recognised or not, and an unrecognised one is consumed whole.
2. **Mode-2031 colour-scheme reports** (`CSI ? 997 ; 1 n` dark, `; 2 n` light) parse to
   `InternalEvent::ColorScheme`.
3. **DECRPM** (`CSI ? mode ; setting $ y`) parses to `InternalEvent::ModeReport`.
4. **OSC replies** (`ESC ] … BEL` or `ESC ] … ESC \`, `parse_osc`). Upstream read `ESC ]` as Alt+`]`
   and the rest of an OSC 11 reply as typed characters. OSC 11 (background colour) parses to
   `InternalEvent::Background`; any other OSC reply is consumed whole. `ESC ]` with nothing after
   it, or followed by anything but a digit, is still Alt+`]` exactly as upstream read it. The one
   behaviour change: Alt+`]` followed by a digit *within the same read* now starts an OSC sequence,
   which is dropped at the cap below if it never terminates.
5. **A cap on how long a reply may run unterminated** (`MAX_SEQUENCE`, 256 bytes), for both of the
   above. Upstream had none, so an unterminated sequence held input indefinitely.
6. **`InternalEvent` and `Rgb` come from `tuilith::input`** (`use super::…`) rather than crossterm's
   private module, and the public event types from `crossterm::event`.
7. **Raw-mode check through the public API.** Upstream read its private
   `terminal::sys::is_raw_mode_enabled()`; this reads `crossterm::terminal::is_raw_mode_enabled()`,
   treating an error as raw (so `\n` is Ctrl+J, upstream's raw-mode reading).
8. **`#[cfg(feature = "bracketed-paste")]` removed** (four sites). It names a crossterm feature, which
   is not a feature of this crate; crossterm's default features include it, so `Event::Paste` exists.
9. **Line endings normalised to LF.** Upstream's file is CRLF.
10. **Tests added** in a `tuilith_tests` module beside upstream's, which are kept and run.
11. **Kitty graphics replies** (`ESC _ G i=<id> ; <message>` terminated by ST or BEL, `parse_apc`)
    parse to `InternalEvent::KittyGraphics`. Upstream read `ESC _` as Alt+`_` and the reply as typed
    characters, so asking whether a terminal draws kitty graphics typed garbage into the application.
    `ESC _` alone, or followed by anything but `G`, is still Alt+`_`; the same within-one-read caveat
    and the same `MAX_SEQUENCE` cap as OSC apply.
12. **DA1 keeps its attribute list** (`parse_csi_primary_device_attributes`). Upstream's stub discarded
    it; attribute 4 is how a terminal says it draws sixel.
13. **Device status reports** (`CSI Ps n`, `parse_csi_status_report`) parse to `InternalEvent::Status`.
    Upstream fell through to the modifier-key parser and dropped them. `CSI 0 n` is the answer to
    `CSI 5 n`, which ends a batch of queries because replies come back in order.
14. **The cell-size report** (`CSI 6 ; height ; width t`, `parse_csi_window_report`) parses to
    `InternalEvent::CellSize`; any other window report is consumed whole. Upstream dropped these the
    same way.

## Not vendored

The reader (`src/event/source/unix/mio.rs`) is rewritten in `src/input/unix.rs`, shaped after it but
sharing no lines: it polls level-triggered with `poll(2)` instead of mio's edge-triggered epoll, so a
read that leaves bytes behind is woken again; and it holds a lone ESC at the end of a read for 20 ms
before calling it the Esc key, because a reply split across reads (which happens on WSL) starts with
one.
