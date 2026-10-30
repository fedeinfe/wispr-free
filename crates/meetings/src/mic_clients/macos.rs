//! macOS: CoreAudio's process objects, available from 14.2.

use objc2::available;
use objc2_core_audio::{
    kAudioHardwarePropertyProcessObjectList, kAudioObjectSystemObject,
    kAudioProcessPropertyBundleID, kAudioProcessPropertyIsRunningInput,
    kAudioProcessPropertyIsRunningOutput, kAudioProcessPropertyPID,
};

use crate::MicClient;
use crate::system_audio::{object_list, object_property, object_string};

pub fn other_app_playing() -> bool {
    if !available!(macos = 14.2) {
        return false;
    }
    let own_pid = std::process::id() as i32;
    object_list(
        kAudioObjectSystemObject as u32,
        kAudioHardwarePropertyProcessObjectList,
    )
    .into_iter()
    .filter(|&process| {
        object_property::<i32>(process, kAudioProcessPropertyPID).is_some_and(|pid| pid != own_pid)
    })
    .any(|process| {
        object_property::<u32>(process, kAudioProcessPropertyIsRunningOutput)
            .is_some_and(|on| on != 0)
    })
}

pub fn mic_clients() -> Vec<MicClient> {
    if !available!(macos = 14.2) {
        return Vec::new();
    }
    object_list(
        kAudioObjectSystemObject as u32,
        kAudioHardwarePropertyProcessObjectList,
    )
    .into_iter()
    .filter(|&process| {
        object_property::<u32>(process, kAudioProcessPropertyIsRunningInput)
            .is_some_and(|on| on != 0)
    })
    .filter_map(|process| {
        Some(MicClient {
            pid: object_property::<i32>(process, kAudioProcessPropertyPID)?.max(0) as u32,
            name: object_string(process, kAudioProcessPropertyBundleID)?,
        })
    })
    .collect()
}
