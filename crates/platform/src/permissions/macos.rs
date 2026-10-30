//! macOS: TCC. A check never shows a dialog; only [`ask`] can.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::thread;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_application_services::{
    AXIsProcessTrusted, AXIsProcessTrustedWithOptions, kAXTrustedCheckOptionPrompt,
};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFString};

use super::{Access, Permission, Permissions};

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
    fn IOHIDRequestAccess(request: u32) -> bool;
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

/// Not on the caller's thread: each of these may wait for the user.
pub fn ask(permission: Permission) {
    thread::spawn(move || match permission {
        Permission::Microphone => ask_microphone(),
        Permission::Accessibility => ask_accessibility(),
        Permission::InputMonitoring => {
            unsafe { IOHIDRequestAccess(LISTEN_EVENT) };
        }
    });
}

/// The anchors are the ones in macOS's own `TCCServiceList.plist`.
pub fn settings_link(permission: Permission) -> Option<&'static str> {
    Some(match permission {
        Permission::Microphone => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        Permission::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        Permission::InputMonitoring => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
        }
    })
}

fn ask_microphone() {
    let Some((device, media_type)) = capture_device() else {
        return;
    };
    let answered = RcBlock::new(|_granted: Bool| {});
    let _: () = unsafe {
        msg_send![device, requestAccessForMediaType: &*media_type, completionHandler: &*answered]
    };
}

fn ask_accessibility() {
    let prompt = unsafe { kAXTrustedCheckOptionPrompt };
    let options =
        CFDictionary::<CFString, CFBoolean>::from_slices(&[prompt], &[CFBoolean::new(true)]);
    unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) };
}

fn microphone() -> Access {
    let Some((device, media_type)) = capture_device() else {
        return Access::Unknown;
    };
    let status: isize = unsafe { msg_send![device, authorizationStatusForMediaType: &*media_type] };
    from_av_status(status)
}

/// `AVCaptureDevice` and `AVMediaTypeAudio`. AVFoundation is heavy, and only these calls want it,
/// so it's loaded by hand on first use.
fn capture_device() -> Option<(&'static AnyClass, Retained<AnyObject>)> {
    let handle = unsafe { dlopen(AV_FOUNDATION.as_ptr(), RTLD_LAZY) };
    if handle.is_null() {
        return None;
    }
    let media_type = unsafe { dlsym(handle, c"AVMediaTypeAudio".as_ptr()) };
    let device = AnyClass::get(c"AVCaptureDevice")?;
    if media_type.is_null() {
        return None;
    }
    let media_type = unsafe { *media_type.cast::<*const AnyObject>() };
    let media_type = unsafe { Retained::retain(media_type.cast_mut()) }?;
    Some((device, media_type))
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

    #[test]
    fn every_permission_links_to_its_own_pane() {
        for (permission, anchor) in [
            (Permission::Microphone, "Privacy_Microphone"),
            (Permission::Accessibility, "Privacy_Accessibility"),
            (Permission::InputMonitoring, "Privacy_ListenEvent"),
        ] {
            let link = settings_link(permission).expect("macOS has a pane for it");
            assert!(link.starts_with("x-apple.systempreferences:"), "{link}");
            assert!(link.ends_with(anchor), "{link}");
        }
    }
}
