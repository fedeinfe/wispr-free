//! Extended window styles do the work here: a no-activate, topmost tool window.

use std::mem;
use std::ptr;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use wf_core::pill::dock::{Display, Point, Rect};
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, TRUE};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetCursorPos, GetWindowLongPtrW, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, SetWindowPos, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST,
};

use super::OverlayError;

const BASE_DPI: f64 = 96.0;

pub fn apply(window: &impl HasWindowHandle) -> Result<(), OverlayError> {
    let hwnd = hwnd(window)?;
    let extra = WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
    // SAFETY: hwnd comes from a live window owned by this thread.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | extra as isize);
    }
    set_topmost(hwnd, SWP_FRAMECHANGED)
}

pub fn raise(window: &impl HasWindowHandle) -> Result<(), OverlayError> {
    set_topmost(hwnd(window)?, 0)
}

pub fn displays(_window_scale: f32) -> Vec<Display> {
    let mut displays = Vec::new();
    // SAFETY: the callback only runs during this call, while `displays` is alive.
    unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(collect_display),
            &raw mut displays as LPARAM,
        );
    }
    displays
}

pub fn cursor_position() -> Option<Point> {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a valid out pointer.
    let ok = unsafe { GetCursorPos(&mut point) };
    (ok != 0).then_some(Point {
        x: f64::from(point.x),
        y: f64::from(point.y),
    })
}

pub fn escape_pressed() -> bool {
    // SAFETY: plain state query. The high bit means the key is down.
    (unsafe { GetAsyncKeyState(i32::from(VK_ESCAPE)) } as u16 & 0x8000) != 0
}

fn hwnd(window: &impl HasWindowHandle) -> Result<HWND, OverlayError> {
    match window.window_handle()?.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as HWND),
        _ => Err(OverlayError::UnsupportedWindow),
    }
}

fn set_topmost(hwnd: HWND, flags: u32) -> Result<(), OverlayError> {
    let flags = flags | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
    // SAFETY: hwnd comes from a live window owned by this thread.
    if unsafe { SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags) } == 0 {
        return Err(OverlayError::Os(std::io::Error::last_os_error().to_string()));
    }
    Ok(())
}

unsafe extern "system" fn collect_display(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> i32 {
    // SAFETY: `data` is the Vec passed to EnumDisplayMonitors in `displays`.
    let displays = unsafe { &mut *(data as *mut Vec<Display>) };
    let mut info: MONITORINFO = unsafe { mem::zeroed() };
    info.cbSize = mem::size_of::<MONITORINFO>() as u32;
    // SAFETY: `monitor` comes from the enumeration and `info` has its size set.
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return TRUE;
    }
    let (mut dpi_x, mut dpi_y) = (0, 0);
    // SAFETY: both are valid out pointers.
    let dpi_failed =
        unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } != 0;
    displays.push(Display {
        bounds: rect(info.rcMonitor),
        work_area: rect(info.rcWork),
        units_per_point: if dpi_failed {
            1.0
        } else {
            f64::from(dpi_x) / BASE_DPI
        },
    });
    TRUE
}

fn rect(r: RECT) -> Rect {
    Rect {
        x: f64::from(r.left),
        y: f64::from(r.top),
        width: f64::from(r.right - r.left),
        height: f64::from(r.bottom - r.top),
    }
}
