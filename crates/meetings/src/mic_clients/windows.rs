//! Windows: the active audio sessions on every capture endpoint.

use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, eCapture,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{Interface, PWSTR, Result};

use crate::MicClient;

pub fn mic_clients() -> Vec<MicClient> {
    match active_capture_sessions() {
        Ok(clients) => clients,
        Err(e) => {
            tracing::debug!("cannot list audio sessions: {e}");
            Vec::new()
        }
    }
}

fn active_capture_sessions() -> Result<Vec<MicClient>> {
    let mut clients = Vec::new();
    // SAFETY: plain COM calls on interfaces that stay alive for the duration of each call.
    unsafe {
        // Fails harmlessly if this thread already joined a different apartment.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let endpoints = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        for endpoint in 0..endpoints.GetCount()? {
            let device = endpoints.Item(endpoint)?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
            let sessions = manager.GetSessionEnumerator()?;
            for session in 0..sessions.GetCount()? {
                let control = sessions.GetSession(session)?;
                if control.GetState()? != AudioSessionStateActive {
                    continue;
                }
                let pid = control.cast::<IAudioSessionControl2>()?.GetProcessId()?;
                if let Some(name) = process_name(pid) {
                    clients.push(MicClient { pid, name });
                }
            }
        }
    }
    Ok(clients)
}

/// The executable's file name, such as `Zoom.exe`.
fn process_name(pid: u32) -> Option<String> {
    // SAFETY: the buffer outlives the call and `len` holds its size; the handle is closed once.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut path = [0u16; MAX_PATH as usize];
        let mut len = path.len() as u32;
        let queried = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        queried.ok()?;
        let path = String::from_utf16_lossy(&path[..len as usize]);
        path.rsplit('\\').next().map(str::to_owned)
    }
}
