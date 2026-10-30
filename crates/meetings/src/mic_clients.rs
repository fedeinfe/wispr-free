//! Which processes are capturing from a microphone right now (spec §7), and whether any other
//! process is playing sound.

use crate::MicClient;

/// Asks the OS. Cheap enough to poll every couple of seconds; an empty list can also mean the OS
/// wouldn't say, so treat it only as "nothing to suggest".
pub fn mic_clients() -> Vec<MicClient> {
    platform::mic_clients()
}

/// Whether another process is sending audio to an output. The macOS tap records silence without
/// an error when its permission is denied, so silence is only suspicious while this is true.
pub fn other_app_playing() -> bool {
    platform::other_app_playing()
}

#[cfg(target_os = "macos")]
#[path = "mic_clients/macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "mic_clients/windows.rs"]
mod platform;

#[cfg(target_os = "linux")]
mod platform {
    use std::process::Command;

    use crate::{MicClient, parse_pw_dump};

    pub fn mic_clients() -> Vec<MicClient> {
        match Command::new("pw-dump").output() {
            Ok(output) => parse_pw_dump(&String::from_utf8_lossy(&output.stdout)),
            Err(e) => {
                tracing::debug!("pw-dump is not available: {e}");
                Vec::new()
            }
        }
    }

    /// PipeWire's monitor reports what it can, and a denied permission can't silence it.
    pub fn other_app_playing() -> bool {
        false
    }
}
