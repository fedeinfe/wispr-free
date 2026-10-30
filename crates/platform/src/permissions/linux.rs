//! Linux: reading keys needs access to `/dev/input`, which usually means the `input` group. The
//! mic has no permission to ask about outside a sandbox.

use std::fs::{self, File};

use super::{Access, Permission, Permissions};

pub const MICROPHONE_DENIED: &str = "Allow the microphone for this app";

pub fn check() -> Permissions {
    Permissions {
        microphone: Access::Unknown,
        accessibility: Access::Granted,
        input_monitoring: input_devices(),
    }
}

pub fn ask(_permission: Permission) {}

pub fn settings_link(_permission: Permission) -> Option<&'static str> {
    None
}

fn input_devices() -> Access {
    let Ok(devices) = fs::read_dir("/dev/input") else {
        return Access::Unknown;
    };
    let mut any = false;
    for device in devices.flatten() {
        if device.file_name().to_string_lossy().starts_with("event") {
            any = true;
            if File::open(device.path()).is_ok() {
                return Access::Granted;
            }
        }
    }
    if any { Access::Denied } else { Access::Unknown }
}
