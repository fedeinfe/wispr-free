//! macOS: CoreAudio for the mute, and Apple Events to the players that have a scripting dictionary.
//!
//! macOS has no public way to learn what is playing. MediaRemote is private and, from 15.4, serves
//! only Apple's own processes. A media key is a toggle, so it would start whatever was paused. The
//! process list says which apps hold the output open, not which are playing. A player that answers
//! `player state` is the one case that can be asked first, so only those are paused.
//!
//! Scripts run in `osascript`: `NSAppleScript` is for the main thread, and a child can be killed
//! when a player hangs or a permission question stays open.

use std::io::Read;
use std::mem::size_of;
use std::process::{Command, Stdio};
use std::ptr::{self, NonNull};
use std::thread;
use std::time::{Duration, Instant};

use objc2_app_kit::NSRunningApplication;
use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectID, AudioObjectIsPropertySettable,
    AudioObjectPropertyAddress, AudioObjectPropertyScope, AudioObjectPropertySelector,
    AudioObjectSetPropertyData, kAudioDevicePropertyDeviceUID, kAudioDevicePropertyMute,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyTranslateUIDToDevice,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject, kAudioObjectUnknown,
};
use objc2_core_foundation::{CFRetained, CFString};
use objc2_foundation::NSString;

use super::{MediaControl, MediaError, Output};

const OSASCRIPT: &str = "/usr/bin/osascript";
/// How long a player gets to answer, in the middle of a dictation.
const SCRIPT_LIMIT: Duration = Duration::from_secs(3);
/// How long the user gets to answer the OS's question about a player.
const ASK_LIMIT: Duration = Duration::from_secs(60);
const SCRIPT_POLL: Duration = Duration::from_millis(10);
const PAUSED: &str = "paused";

/// Bundle ids of the players that report `player state` and take `pause` and `play`.
const PLAYERS: [&str; 2] = ["com.apple.Music", "com.spotify.client"];

pub struct SystemMedia;

impl MediaControl for SystemMedia {
    fn output(&self) -> Result<Output, MediaError> {
        let device = get::<AudioObjectID>(
            kAudioObjectSystemObject as AudioObjectID,
            kAudioHardwarePropertyDefaultOutputDevice,
            kAudioObjectPropertyScopeGlobal,
        )
        .filter(|&device| device != kAudioObjectUnknown as AudioObjectID)
        .ok_or(MediaError::NoOutput)?;
        let uid = uid_of(device).ok_or(MediaError::NoOutput)?;
        if !can_mute(device) {
            return Err(MediaError::NoMuteControl);
        }
        let muted = get::<u32>(
            device,
            kAudioDevicePropertyMute,
            kAudioObjectPropertyScopeOutput,
        )
        .ok_or(MediaError::NoMuteControl)?;
        Ok(Output {
            device: uid,
            muted: muted != 0,
        })
    }

    fn set_muted(&self, device: &str, muted: bool) -> Result<(), MediaError> {
        let device = device_with_uid(device).ok_or(MediaError::OutputGone)?;
        let address = address(kAudioDevicePropertyMute, kAudioObjectPropertyScopeOutput);
        let value = u32::from(muted);
        // SAFETY: the value is the u32 this property holds, and it outlives the call.
        let status = unsafe {
            AudioObjectSetPropertyData(
                device,
                NonNull::from(&address),
                0,
                ptr::null(),
                size_of::<u32>() as u32,
                NonNull::from(&value).cast(),
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(MediaError::Refused(format!("OSStatus {status}")))
        }
    }

    fn pause_players(&self) -> Vec<String> {
        PLAYERS
            .into_iter()
            .filter(|player| is_running(player))
            .filter(|player| {
                run_script(&pause_script(player), SCRIPT_LIMIT).as_deref() == Some(PAUSED)
            })
            .map(str::to_owned)
            .collect()
    }

    fn resume_players(&self, players: &[String]) {
        for player in players.iter().filter(|player| is_running(player)) {
            let _ = run_script(&resume_script(player), SCRIPT_LIMIT);
        }
    }

    fn ask_players(&self) {
        for player in PLAYERS.into_iter().filter(|player| is_running(player)) {
            let _ = run_script(&ask_script(player), ASK_LIMIT);
        }
    }
}

fn address(
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

fn get<T: Copy>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
) -> Option<T> {
    let address = address(selector, scope);
    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let mut size = size_of::<T>() as u32;
    // SAFETY: `value` has room for `size` bytes, and only an exact-size answer is read back.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(value.as_mut_ptr()).cast(),
        )
    };
    // SAFETY: CoreAudio filled in all of it.
    (status == 0 && size as usize == size_of::<T>()).then(|| unsafe { value.assume_init() })
}

fn uid_of(device: AudioObjectID) -> Option<String> {
    let uid: *const CFString = get(
        device,
        kAudioDevicePropertyDeviceUID,
        kAudioObjectPropertyScopeGlobal,
    )?;
    // SAFETY: CoreAudio returns CFString properties at +1.
    let uid = unsafe { CFRetained::from_raw(NonNull::new(uid.cast_mut())?) };
    Some(uid.to_string()).filter(|uid| !uid.is_empty())
}

fn device_with_uid(uid: &str) -> Option<AudioObjectID> {
    let uid = CFString::from_str(uid);
    let qualifier: *const CFString = &*uid;
    let address = address(
        kAudioHardwarePropertyTranslateUIDToDevice,
        kAudioObjectPropertyScopeGlobal,
    );
    let mut device: AudioObjectID = kAudioObjectUnknown as AudioObjectID;
    let mut size = size_of::<AudioObjectID>() as u32;
    // SAFETY: the qualifier is the CFStringRef this property takes, and `uid` outlives the call.
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&address),
            size_of::<*const CFString>() as u32,
            ptr::from_ref(&qualifier).cast(),
            NonNull::from(&mut size),
            NonNull::from(&mut device).cast(),
        )
    };
    (status == 0 && device != kAudioObjectUnknown as AudioObjectID).then_some(device)
}

fn can_mute(device: AudioObjectID) -> bool {
    let address = address(kAudioDevicePropertyMute, kAudioObjectPropertyScopeOutput);
    let mut settable = 0;
    // SAFETY: both pointers are valid for the call.
    let status = unsafe {
        AudioObjectIsPropertySettable(
            device,
            NonNull::from(&address),
            NonNull::from(&mut settable),
        )
    };
    status == 0 && settable != 0
}

fn is_running(player: &str) -> bool {
    let id = NSString::from_str(player);
    !NSRunningApplication::runningApplicationsWithBundleIdentifier(&id).is_empty()
}

fn pause_script(player: &str) -> String {
    format!(
        "if application id \"{player}\" is running then\n\
         tell application id \"{player}\"\n\
         if player state is playing then\n\
         pause\n\
         return \"{PAUSED}\"\n\
         end if\n\
         end tell\n\
         end if"
    )
}

fn resume_script(player: &str) -> String {
    format!(
        "if application id \"{player}\" is running then\n\
         tell application id \"{player}\"\n\
         if player state is paused then play\n\
         end tell\n\
         end if"
    )
}

/// Reads and changes nothing that matters, which is enough for the OS to ask.
fn ask_script(player: &str) -> String {
    format!(
        "if application id \"{player}\" is running then\n\
         tell application id \"{player}\" to get player state\n\
         end if"
    )
}

/// What the script printed, or `None` if it failed or ran out of time.
fn run_script(script: &str, limit: Duration) -> Option<String> {
    let mut child = Command::new(OSASCRIPT)
        .args(["-e", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => break,
            Some(_) => return None,
            None if started.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => thread::sleep(SCRIPT_POLL),
        }
    }
    let mut printed = String::new();
    child.stdout.take()?.read_to_string(&mut printed).ok()?;
    Some(printed.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pausing_asks_first_and_can_only_pause() {
        let script = pause_script("com.apple.Music");

        assert!(script.contains("if player state is playing then"));
        assert!(script.contains("\npause\n"));
        assert!(!script.contains("playpause"));
        assert!(!script.contains("\nplay\n"));
    }

    #[test]
    fn resuming_touches_only_a_player_that_is_still_paused() {
        let script = resume_script("com.spotify.client");

        assert!(script.contains("if player state is paused then play"));
    }

    #[test]
    fn no_script_can_launch_a_player() {
        for script in [
            pause_script("com.apple.Music"),
            resume_script("com.apple.Music"),
            ask_script("com.apple.Music"),
        ] {
            assert!(script.starts_with("if application id \"com.apple.Music\" is running then"));
            assert!(script.ends_with("end if"));
        }
    }
}
