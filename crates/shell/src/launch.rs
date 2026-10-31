//! Which app a process is. Everything built to keep dev runs off the user's Mac is lifted here, and
//! only here: a run is the real app when it's the real bundle, started plainly. Any other process
//! that reaches this code, whether a test, a tool, the dev bundle or a copy of the binary, is a dev
//! run. It decides from what the OS says, so nothing a caller passes can make a run real.

use std::env;

use wf_dictation::remote::ENGINE_HOST_ARG;
use wf_platform::desktop;

/// The id `cargo xtask bundle` gives the real app.
pub const REAL_BUNDLE_ID: &str = "com.wispr-free.app";
const CLIP_RUN_ARG: &str = "--probe-clip";
/// What may follow a plain start. The tools' flags aren't here: with one of them, even the real
/// bundle stays a dev run.
const REAL_APP_ARGS: [&str; 1] = ["--debug-pill"];

/// What a process that was just started should do.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Launch {
    /// The user's app: their data, keys, microphone and login entry.
    Real,
    /// Its own data folder, and nothing of the Mac's that a person didn't ask for.
    Dev,
    /// Say `text` and end, starting nothing.
    Exit { text: String, failed: bool },
}

/// Whose rules a running `Shell` follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    Real,
    Dev,
}

/// The decision. `arguments` leaves out the program's own name.
pub fn decide(bundle_id: Option<&str>, arguments: &[String]) -> Launch {
    let asks = |arg: &str| arguments.iter().any(|given| given == arg);
    if asks("--version") || asks("-V") {
        return Launch::Exit {
            text: format!("wispr-free {}", env!("CARGO_PKG_VERSION")),
            failed: false,
        };
    }
    // Engines run in wf-engine-host (ADR-0005). An older app that was running when this one was
    // installed still starts this path as a host, and a script may still ask it for a clip run.
    // Neither may open a real app, with real keys.
    if asks(ENGINE_HOST_ARG) {
        return Launch::Exit {
            text: "the engine host is wf-engine-host now".to_owned(),
            failed: true,
        };
    }
    if arguments.iter().any(|arg| arg.starts_with(CLIP_RUN_ARG)) {
        return Launch::Exit {
            text: "clip runs are in wf-probe now".to_owned(),
            failed: true,
        };
    }
    let plain = arguments
        .iter()
        .all(|arg| REAL_APP_ARGS.contains(&arg.as_str()));
    if bundle_id == Some(REAL_BUNDLE_ID) && plain {
        Launch::Real
    } else {
        Launch::Dev
    }
}

/// [`decide`], for this process.
#[uniffi::export]
pub fn this_launch() -> Launch {
    let arguments: Vec<String> = env::args().skip(1).collect();
    decide(desktop::bundle_id().as_deref(), &arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV_BUNDLE_ID: &str = "com.wispr-free.app.dev";

    fn launch(bundle_id: Option<&str>, arguments: &[&str]) -> Launch {
        let arguments: Vec<_> = arguments.iter().map(|arg| (*arg).to_owned()).collect();
        decide(bundle_id, &arguments)
    }

    #[test]
    fn a_plain_start_of_the_real_bundle_is_the_real_app() {
        assert_eq!(launch(Some(REAL_BUNDLE_ID), &[]), Launch::Real);
    }

    #[test]
    fn the_real_app_keeps_its_debug_menu() {
        assert_eq!(
            launch(Some(REAL_BUNDLE_ID), &["--debug-pill"]),
            Launch::Real
        );
    }

    #[test]
    fn nothing_but_the_real_bundle_is_the_real_app() {
        for bundle_id in [
            None,
            Some(DEV_BUNDLE_ID),
            Some("com.wispr-free.app.other"),
            Some("com.apple.dt.xctest.tool"),
            Some(""),
        ] {
            assert_eq!(launch(bundle_id, &[]), Launch::Dev, "{bundle_id:?}");
        }
    }

    #[test]
    fn a_dev_bundle_stays_a_dev_run_by_hand_too() {
        assert_eq!(launch(Some(DEV_BUNDLE_ID), &["--by-hand"]), Launch::Dev);
    }

    /// A tool's flag means a tool started this, so even on the real bundle it gets its own folder
    /// and no keys.
    #[test]
    fn a_tool_on_the_real_bundle_stays_a_dev_run() {
        for flag in [
            "--behind",
            "--by-hand",
            "--rehearse",
            "--setup",
            "--walk-meeting",
            "--pill-state=locked",
            "--pick=Quit wispr-free",
            "--shots=/tmp/shots",
            "--window-shots=/tmp/shots",
            "--pick-folder=/tmp/models",
            "--tour=1",
            "--anything-new",
        ] {
            assert_eq!(launch(Some(REAL_BUNDLE_ID), &[flag]), Launch::Dev, "{flag}");
            let mixed = ["--debug-pill", flag];
            assert_eq!(launch(Some(REAL_BUNDLE_ID), &mixed), Launch::Dev, "{flag}");
        }
    }

    #[test]
    fn the_version_flag_says_the_version_and_starts_nothing() {
        let version = Launch::Exit {
            text: format!("wispr-free {}", env!("CARGO_PKG_VERSION")),
            failed: false,
        };
        for flag in ["--version", "-V"] {
            assert_eq!(launch(Some(REAL_BUNDLE_ID), &[flag]), version);
            assert_eq!(launch(None, &[flag]), version);
        }
    }

    #[test]
    fn the_app_refuses_to_be_an_engine_host_or_run_a_clip() {
        for flag in ["--engine-host", "--probe-clip=clip.wav", "--probe-clip"] {
            let said = launch(Some(REAL_BUNDLE_ID), &[flag]);
            assert!(
                matches!(said, Launch::Exit { failed: true, .. }),
                "{flag}: {said:?}"
            );
        }
    }

    /// A test binary is no bundle at all, so what the tests run can't become the real app.
    #[test]
    fn this_process_is_a_dev_run() {
        assert_eq!(this_launch(), Launch::Dev);
    }
}
