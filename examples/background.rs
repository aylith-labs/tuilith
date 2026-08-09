//! What this terminal says its background is, and which signal said it.
//!
//! Run it when an application drew in the wrong polarity: the answer names the signal, so a wrong guess
//! reads as a wrong guess rather than as a wrong theme.
//!
//! ```text
//! cargo run --example background
//! ```

fn main() {
    let at_startup = tuilith::background::read();
    let while_running = tuilith::background::read_while_running();

    println!("before the UI starts: {}", describe(at_startup));
    println!("from a running loop:  {}", describe(while_running));

    if at_startup.mode != while_running.mode {
        // Read off the two sources rather than asserting the usual cause: this example exists to make a
        // guess inspectable, so inventing one here would be the very thing it is arguing against.
        println!(
            "\nthe two disagree: {} says {}, {} says {} — so an application that re-reads while running \
             will change polarity mid-session",
            at_startup.source.label(),
            at_startup.mode.label(),
            while_running.source.label(),
            while_running.mode.label(),
        );
    }
    if at_startup.source.answered() && !at_startup.source.describes_the_terminal() {
        println!(
            "\nthat answer describes the desktop, not this terminal — a deliberately dark terminal on a \
             light desktop would be reported as light, and nothing here can tell"
        );
    }
    for (name, value) in [
        ("TERM", std::env::var("TERM")),
        ("COLORTERM", std::env::var("COLORTERM")),
        ("COLORFGBG", std::env::var("COLORFGBG")),
        ("WSL_DISTRO_NAME", std::env::var("WSL_DISTRO_NAME")),
    ] {
        println!("{name:<16} {}", value.unwrap_or_else(|_| "(unset)".into()));
    }
}

fn describe(reading: tuilith::Reading) -> String {
    let standing = if reading.source.describes_the_terminal() {
        "observed"
    } else if reading.source.answered() {
        "inferred"
    } else {
        "guessed"
    };
    format!(
        "{} ({standing}), according to {}",
        reading.mode.label(),
        reading.source.label()
    )
}
