//! Manual checks for insertion and context in real apps.
//!
//!   cargo run -p wf-platform --example insertion_probe -- context [--delay 3] [--counts]
//!   cargo run -p wf-platform --example insertion_probe -- insert "some text" [--delay 3]
//!   cargo run -p wf-platform --example insertion_probe -- clipboard
//!
//! `--delay` leaves time to click into the target app. `--counts` prints lengths instead of the
//! window title and surrounding text. On macOS the terminal running this needs Accessibility.

use std::time::Duration;

use wf_platform::context::{
    ContextDepth, ContextSource, SystemContext, TextAround, accessibility_trusted,
};
use wf_platform::insert::{Clipboard, SystemClipboard, SystemInserter};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let delay = args
        .iter()
        .position(|arg| arg == "--delay")
        .and_then(|index| args.get(index + 1)?.parse().ok())
        .unwrap_or(0);
    std::thread::sleep(Duration::from_secs(delay));

    match args.first().map(String::as_str) {
        Some("context") => print_context(args.iter().any(|arg| arg == "--counts")),
        Some("insert") => {
            let text = args.get(1).expect("usage: insert <text>");
            println!("{:?}", SystemInserter::system().insert(text));
        }
        Some("clipboard") => clipboard_round_trip(),
        _ => eprintln!("usage: insertion_probe context|insert <text>|clipboard [--delay N]"),
    }
}

fn print_context(counts_only: bool) {
    println!("accessibility trusted: {}", accessibility_trusted());
    let context = SystemContext.read(ContextDepth::WithText);
    println!("app: {:?}", context.app.as_ref().map(|app| &app.id));
    println!("focus: {:?}", context.focus);
    if counts_only {
        let title = context.window_title.as_ref().map(|t| t.chars().count());
        println!("window title chars: {title:?}");
        println!("text chars: {:?}", context.text.as_ref().map(counts));
    } else {
        println!("window title: {:?}", context.window_title);
        println!("text: {:#?}", context.text);
    }
}

fn counts(text: &TextAround) -> [usize; 3] {
    [&text.before, &text.selection, &text.after].map(|part| part.chars().count())
}

/// Saves, writes, restores. Sends no keystroke, so it is safe to run anywhere.
fn clipboard_round_trip() {
    let mut clipboard = SystemClipboard::new();
    let saved = clipboard.snapshot().expect("snapshot");
    clipboard
        .write_text("wispr-free probe", true)
        .expect("write");
    clipboard.restore(saved).expect("restore");
    println!("clipboard restored");
}
