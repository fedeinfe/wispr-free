//! One loaded engine in one process (ADR-0005). Nothing here touches the window system.

use std::process::ExitCode;

use wf_dictation::remote::{ENGINE_HOST_ARG, run_engine_host};

// What a host ran on when it was the app's own binary.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == ENGINE_HOST_ARG) {
        return match run_engine_host() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    eprintln!("usage: wf-engine-host {ENGINE_HOST_ARG} | --version");
    ExitCode::FAILURE
}
