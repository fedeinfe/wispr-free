//! macOS: TCC. Nothing here can ask for a permission, so a check never shows a dialog.

use std::ffi::{CStr, c_char, c_int, c_void};

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_application_services::AXIsProcessTrusted;

use super::{Access, Permissions};

pub const MICROPHONE_DENIED: &str = "Allow the microphone in System Settings, Privacy & Security";

const AV_FOUNDATION: &CStr = c"/System/Library/Frameworks/AVFoundation.framework/AVFoundation";
const RTLD_LAZY: c_int = 1;
const LISTEN_EVENT: u32 = 1;

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request: u32) -> u32;
}

pub fn check() -> Permissions {
    Permissions {
        microphone: microphone(),
        accessibility: if unsafe { AXIsProcessTrusted() } {
            Access::Granted
        } else {
            Access::Denied
        },
        input_monitoring: from_hid_access(unsafe { IOHIDCheckAccess(LISTEN_EVENT) }),
    }
}

/// AVFoundation is heavy, and only this check wants it, so it's loaded by hand on first use.
fn microphone() -> Access {
    let handle = unsafe { dlopen(AV_FOUNDATION.as_ptr(), RTLD_LAZY) };
    if handle.is_null() {
        return Access::Unknown;
    }
    let media_type = unsafe { dlsym(handle, c"AVMediaTypeAudio".as_ptr()) };
    let Some(device) = AnyClass::get(c"AVCaptureDevice") else {
        return Access::Unknown;
    };
    if media_type.is_null() {
        return Access::Unknown;
    }
    let media_type = unsafe { *media_type.cast::<*const AnyObject>() };
    let Some(media_type) = (unsafe { Retained::retain(media_type.cast_mut()) }) else {
        return Access::Unknown;
    };
    let status: isize = unsafe { msg_send![device, authorizationStatusForMediaType: &*media_type] };
    from_av_status(status)
}

/// `AVAuthorizationStatus`.
fn from_av_status(status: isize) -> Access {
    match status {
        3 => Access::Granted,
        1 | 2 => Access::Denied,
        _ => Access::Unknown,
    }
}

/// `IOHIDAccessType`.
fn from_hid_access(access: u32) -> Access {
    match access {
        0 => Access::Granted,
        1 => Access::Denied,
        _ => Access::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_answers_map_to_what_the_app_can_do_about_them() {
        assert_eq!(from_av_status(0), Access::Unknown, "not asked yet");
        assert_eq!(from_av_status(1), Access::Denied, "restricted");
        assert_eq!(from_av_status(2), Access::Denied);
        assert_eq!(from_av_status(3), Access::Granted);
        assert_eq!(from_hid_access(0), Access::Granted);
        assert_eq!(from_hid_access(1), Access::Denied);
        assert_eq!(from_hid_access(2), Access::Unknown);
    }
}
