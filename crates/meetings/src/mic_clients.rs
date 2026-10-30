//! Which processes are capturing from a microphone right now (spec §7).

use crate::MicClient;

/// Asks the OS. Cheap enough to poll every couple of seconds; an empty list can also mean the OS
/// wouldn't say, so treat it only as "nothing to suggest".
pub fn mic_clients() -> Vec<MicClient> {
    platform::mic_clients()
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
}
