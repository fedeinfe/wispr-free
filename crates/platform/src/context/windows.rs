//! Windows backend: the foreground window for the app and title, UI Automation for the
//! focused element and its text.

use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextRange, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start,
    TextUnit_Character, UIA_TextPatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
};
use windows::core::PWSTR;

use super::uia_role::classify;
use super::{
    AppIdentity, CONTEXT_CHARS, Context, ContextDepth, ContextSource, Focus, MAX_SELECTION_CHARS,
    TextAround,
};

/// `GetText` argument for "the whole range".
const NO_LIMIT: i32 = -1;

/// Windows has no Accessibility permission to wait for.
pub fn accessibility_trusted() -> bool {
    true
}

pub struct SystemContext;

impl ContextSource for SystemContext {
    fn read(&self, depth: ContextDepth) -> Context {
        let window = unsafe { GetForegroundWindow() };
        if window.is_invalid() {
            return Context {
                app: None,
                window_title: None,
                focus: Focus::Nothing,
                text: None,
            };
        }
        let (focus, text) = read_focused(depth).unwrap_or((Focus::Unknown, None));
        Context {
            app: app_of(window),
            window_title: title_of(window),
            focus,
            text,
        }
    }
}

/// Balances `CoInitializeEx` on the calling thread.
struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn read_focused(depth: ContextDepth) -> windows::core::Result<(Focus, Option<TextAround>)> {
    let _com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .is_ok()
        .then_some(ComGuard);
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }?;
    let element = unsafe { automation.GetFocusedElement() }?;
    let control_type = unsafe { element.CurrentControlType() }?;
    let is_password = unsafe { element.CurrentIsPassword() }?.as_bool();
    let focus = classify(control_type.0, is_password);
    let text = match (depth, focus) {
        (ContextDepth::FocusOnly, _) | (_, Focus::Secure) => None,
        (ContextDepth::WithText, _) => read_text(&element).ok(),
    };
    Ok((focus, text))
}

fn read_text(element: &IUIAutomationElement) -> windows::core::Result<TextAround> {
    let pattern: IUIAutomationTextPattern =
        unsafe { element.GetCurrentPatternAs(UIA_TextPatternId) }?;
    let selection = unsafe { pattern.GetSelection()?.GetElement(0) }?;
    let before = text_beside(&selection, Side::Before)?;
    let selected = range_text(&selection, 2 * MAX_SELECTION_CHARS as i32)?;
    let after = text_beside(&selection, Side::After)?;
    Ok(TextAround::new(&before, &selected, &after))
}

enum Side {
    Before,
    After,
}

/// Collapses a copy of the selection to one edge and grows it over the neighbouring text.
fn text_beside(selection: &IUIAutomationTextRange, side: Side) -> windows::core::Result<String> {
    let start = TextPatternRangeEndpoint_Start;
    let end = TextPatternRangeEndpoint_End;
    let range = unsafe { selection.Clone() }?;
    let count = CONTEXT_CHARS as i32;
    unsafe {
        match side {
            Side::Before => {
                range.MoveEndpointByRange(end, selection, start)?;
                range.MoveEndpointByUnit(start, TextUnit_Character, -count)?;
            }
            Side::After => {
                range.MoveEndpointByRange(start, selection, end)?;
                range.MoveEndpointByUnit(end, TextUnit_Character, count)?;
            }
        }
    }
    // The range is already bounded; a GetText limit would cut the end nearest the cursor.
    range_text(&range, NO_LIMIT)
}

fn range_text(range: &IUIAutomationTextRange, max_units: i32) -> windows::core::Result<String> {
    Ok(unsafe { range.GetText(max_units) }?.to_string())
}

fn title_of(window: HWND) -> Option<String> {
    let mut buffer = [0u16; 512];
    let length = unsafe { GetWindowTextW(window, &mut buffer) };
    (length > 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
}

fn app_of(window: HWND) -> Option<AppIdentity> {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    let path = exe_path(pid)?;
    let path = Path::new(&path);
    Some(AppIdentity {
        id: path.file_name()?.to_string_lossy().into_owned(),
        name: path.file_stem().map(|s| s.to_string_lossy().into_owned()),
        pid: Some(pid),
    })
}

fn exe_path(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    result
        .ok()
        .map(|()| String::from_utf16_lossy(&buffer[..length as usize]))
}
