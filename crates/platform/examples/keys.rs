//! Manual check of global key capture: prints every key event and what the dictation state machine
//! makes of it, with the default shortcuts. `cargo run -p wf-platform --example keys`, Ctrl+C quits.
//! On macOS the terminal needs Input Monitoring (and Accessibility to swallow keys).

use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use wf_core::dictation::{Command, DictationMachine};
use wf_core::shortcut::{Bindings, Platform};
use wf_platform::keys::{self, KeySignal};

fn main() -> ExitCode {
    let machine = Arc::new(Mutex::new(DictationMachine::new(Bindings::default_for(
        Platform::current(),
    ))));
    let origin = Instant::now();

    let listener = keys::listen({
        let machine = Arc::clone(&machine);
        move |signal| {
            let now = origin.elapsed();
            let mut machine = machine.lock().unwrap();
            match signal {
                KeySignal::Key(event) => {
                    let reaction = machine.on_key(event, now);
                    println!("{now:10.3?}  {event:?}  swallow={}", reaction.swallow);
                    print_commands(now, &reaction.commands);
                    reaction.swallow
                }
                KeySignal::Lost => {
                    println!("{now:10.3?}  key stream lost");
                    print_commands(now, &machine.on_keys_lost(now));
                    false
                }
            }
        }
    });
    let listener = match listener {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "listening (can swallow keys: {}); press the shortcuts, Ctrl+C quits",
        listener.can_swallow()
    );
    loop {
        thread::sleep(Duration::from_millis(20));
        let now = origin.elapsed();
        print_commands(now, &machine.lock().unwrap().tick(now));
    }
}

fn print_commands(now: Duration, commands: &[Command]) {
    for command in commands {
        println!("{now:10.3?}  => {command:?}");
    }
}
