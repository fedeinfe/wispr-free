//! Windows backend: low-level keyboard and mouse hooks on a thread with a message loop.

use std::cell::RefCell;
use std::ptr::null_mut;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use wf_core::shortcut::{Key, KeyEvent, MouseButton};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED,
    LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PostThreadMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_QUIT, WM_SYSKEYDOWN, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use super::keymap::windows_key;
use super::{Callback, KeySignal, KeysError};

thread_local! {
    /// Hook procedures take no user pointer; they run on the thread that installed them.
    static ON_SIGNAL: RefCell<Option<Callback>> = const { RefCell::new(None) };
}

pub struct Listener {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    pub fn start(on_signal: Callback) -> Result<Self, KeysError> {
        let (ready, started) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("wf-keys".into())
            .spawn(move || run(on_signal, &ready))
            .map_err(|error| KeysError::Unavailable(error.to_string()))?;
        let thread_id = started
            .recv()
            .map_err(|error| KeysError::Unavailable(error.to_string()))??;
        Ok(Self {
            thread_id,
            thread: Some(thread),
        })
    }

    pub fn can_swallow(&self) -> bool {
        true
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(on_signal: Callback, ready: &mpsc::Sender<Result<u32, KeysError>>) {
    ON_SIGNAL.with(|slot| *slot.borrow_mut() = Some(on_signal));
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_keyboard), module, 0);
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(on_mouse), module, 0);
        if keyboard.is_null() || mouse.is_null() {
            let error = std::io::Error::last_os_error().to_string();
            let _ = ready.send(Err(KeysError::Unavailable(error)));
            return;
        }
        let _ = ready.send(Ok(GetCurrentThreadId()));

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnhookWindowsHookEx(keyboard);
        UnhookWindowsHookEx(mouse);
    }
}

/// Hands `event` to the callback; `true` means swallow it.
fn deliver(event: KeyEvent) -> bool {
    ON_SIGNAL.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .is_some_and(|on_signal| on_signal(KeySignal::Key(event)))
    })
}

unsafe extern "system" fn on_keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        // Injected events include our own paste keystroke.
        if info.flags & LLKHF_INJECTED == 0 {
            let pressed = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let key = windows_key(info.vkCode);
            if deliver(KeyEvent { key, pressed }) {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn on_mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        let message = wparam as u32;
        if info.flags & LLMHF_INJECTED == 0
            && let Some(button) = mouse_button(message, info.mouseData)
        {
            let pressed = matches!(message, WM_MBUTTONDOWN | WM_XBUTTONDOWN);
            if deliver(KeyEvent {
                key: Key::Mouse(button),
                pressed,
            }) {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

/// For the X-button messages, the high word of `mouse_data` says which one.
fn mouse_button(message: u32, mouse_data: u32) -> Option<MouseButton> {
    match message {
        WM_MBUTTONDOWN | WM_MBUTTONUP => Some(MouseButton::Middle),
        WM_XBUTTONDOWN | WM_XBUTTONUP => Some(match mouse_data >> 16 {
            1 => MouseButton::Back,
            2 => MouseButton::Forward,
            other => MouseButton::Other((other as u8).saturating_add(2)),
        }),
        _ => None,
    }
}
