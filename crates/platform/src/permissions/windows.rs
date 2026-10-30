//! Windows: the Privacy settings' microphone switch, which is a registry value. Keys and paste
//! need no permission.

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};

use super::{Access, Permission, Permissions};

pub const MICROPHONE_DENIED: &str = "Allow the microphone in Windows Settings, Privacy & security";

const CONSENT_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

pub fn check() -> Permissions {
    Permissions {
        microphone: microphone(),
        accessibility: Access::Granted,
        input_monitoring: Access::Granted,
    }
}

pub fn ask(_permission: Permission) {}

pub fn settings_link(permission: Permission) -> Option<&'static str> {
    (permission == Permission::Microphone).then_some("ms-settings:privacy-microphone")
}

fn microphone() -> Access {
    let key = wide(CONSENT_KEY);
    let name = wide("Value");
    let mut value = [0u16; 16];
    let mut size = size_of_val(&value) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return Access::Unknown;
    }
    let length = value.iter().position(|&unit| unit == 0).unwrap_or(0);
    match String::from_utf16_lossy(&value[..length]).as_str() {
        "Allow" => Access::Granted,
        "Deny" => Access::Denied,
        _ => Access::Unknown,
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
