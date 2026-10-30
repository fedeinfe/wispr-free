//! macOS backend: a CGEventTap for keys, modifiers and extra mouse buttons, plus an IOHIDManager
//! that also reports Fn. Fn arrives from both when both work; the state machine ignores the
//! duplicate press.

use std::ffi::{c_char, c_void};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use wf_core::shortcut::{Key, KeyEvent, MouseButton};

use super::keymap::{mac_key, mac_modifier_pressed};
use super::{Callback, KeySignal, KeysError};

type CfRef = *const c_void;
type CgEvent = *mut c_void;
type TapCallback = extern "C" fn(*mut c_void, u32, CgEvent, *mut c_void) -> CgEvent;
type HidValueCallback = extern "C" fn(*mut c_void, i32, *mut c_void, CfRef);

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: CfRef;
    static kCFRunLoopDefaultMode: CfRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFRunLoopGetCurrent() -> CfRef;
    fn CFRunLoopRunInMode(mode: CfRef, seconds: f64, return_after_source: u8) -> i32;
    fn CFRunLoopStop(run_loop: CfRef);
    fn CFRunLoopAddSource(run_loop: CfRef, source: CfRef, mode: CfRef);
    fn CFMachPortCreateRunLoopSource(allocator: CfRef, port: CfRef, order: isize) -> CfRef;
    fn CFRetain(object: CfRef) -> CfRef;
    fn CFRelease(object: CfRef);
    fn CFStringCreateWithCString(allocator: CfRef, text: *const c_char, encoding: u32) -> CfRef;
    fn CFNumberCreate(allocator: CfRef, kind: isize, value: *const c_void) -> CfRef;
    fn CFDictionaryCreate(
        allocator: CfRef,
        keys: *const CfRef,
        values: *const CfRef,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CfRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CfRef;
    fn CGEventTapEnable(tap: CfRef, enable: bool);
    fn CGEventGetFlags(event: CgEvent) -> u64;
    fn CGEventGetIntegerValueField(event: CgEvent, field: u32) -> i64;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDManagerCreate(allocator: CfRef, options: u32) -> CfRef;
    fn IOHIDManagerSetDeviceMatching(manager: CfRef, matching: CfRef);
    fn IOHIDManagerRegisterInputValueCallback(
        manager: CfRef,
        callback: HidValueCallback,
        context: *mut c_void,
    );
    fn IOHIDManagerScheduleWithRunLoop(manager: CfRef, run_loop: CfRef, mode: CfRef);
    fn IOHIDManagerOpen(manager: CfRef, options: u32) -> i32;
    fn IOHIDManagerClose(manager: CfRef, options: u32) -> i32;
    fn IOHIDValueGetElement(value: CfRef) -> CfRef;
    fn IOHIDValueGetIntegerValue(value: CfRef) -> isize;
    fn IOHIDElementGetUsagePage(element: CfRef) -> u32;
    fn IOHIDElementGetUsage(element: CfRef) -> u32;
}

const SESSION_EVENT_TAP: u32 = 1;
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const TAP_OPTION_DEFAULT: u32 = 0;
const TAP_OPTION_LISTEN_ONLY: u32 = 1;

const EVENT_KEY_DOWN: u32 = 10;
const EVENT_KEY_UP: u32 = 11;
const EVENT_FLAGS_CHANGED: u32 = 12;
const EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const EVENT_OTHER_MOUSE_UP: u32 = 26;
const EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

const FIELD_MOUSE_BUTTON_NUMBER: u32 = 3;
const FIELD_KEYCODE: u32 = 9;
const FIELD_SOURCE_PID: u32 = 41;

/// `kCFRunLoopRunFinished`: no sources left, e.g. the tap died because permission was revoked.
const RUN_LOOP_FINISHED: i32 = 1;

const STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const NUMBER_SINT32: isize = 3;

const USAGE_PAGE_GENERIC_DESKTOP: i32 = 0x01;
const USAGE_KEYBOARD: i32 = 0x06;
/// Apple's vendor pages for Fn (AppleHIDUsageTables.h, not in the public SDK): top-case keyboards
/// and external Apple keyboards. Both use usage 3.
const USAGE_PAGES_FN: [u32; 2] = [0x00FF, 0xFF01];
const USAGE_FN: u32 = 0x03;

/// Lives on the run-loop thread; both callbacks reach it through their user pointer.
struct Context {
    on_signal: Callback,
    tap: CfRef,
    own_pid: i64,
}

pub struct Listener {
    /// Retained, so it outlives the thread if capture dies early.
    run_loop: usize,
    can_swallow: bool,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// What the capture thread hands back once running.
struct Started {
    run_loop: usize,
    can_swallow: bool,
}

impl Listener {
    pub fn start(on_signal: Callback) -> Result<Self, KeysError> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, started) = mpsc::channel();
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("wf-keys".into())
            .spawn(move || run(on_signal, &thread_stop, &ready))
            .map_err(|error| KeysError::Unavailable(error.to_string()))?;
        let Started {
            run_loop,
            can_swallow,
        } = started
            .recv()
            .map_err(|error| KeysError::Unavailable(error.to_string()))??;
        Ok(Self {
            run_loop,
            can_swallow,
            stop,
            thread: Some(thread),
        })
    }

    pub fn can_swallow(&self) -> bool {
        self.can_swallow
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        unsafe { CFRunLoopStop(self.run_loop as CfRef) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        unsafe { CFRelease(self.run_loop as CfRef) };
    }
}

/// Owns everything CoreFoundation for this thread's lifetime; the loop re-checks `stop` so a stop
/// that lands before the first run can't be lost.
fn run(on_signal: Callback, stop: &AtomicBool, ready: &mpsc::Sender<Result<Started, KeysError>>) {
    let context = Box::into_raw(Box::new(Context {
        on_signal,
        tap: null(),
        own_pid: i64::from(std::process::id()),
    }));
    let user_info = context.cast::<c_void>();
    let Some((tap, can_swallow)) = create_tap(user_info) else {
        let _ = ready.send(Err(KeysError::Permission(
            "macOS refused the event tap: allow this app under Privacy & Security > Input \
             Monitoring and Accessibility",
        )));
        drop(unsafe { Box::from_raw(context) });
        return;
    };
    unsafe {
        (*context).tap = tap;
        let run_loop = CFRunLoopGetCurrent();
        let source = CFMachPortCreateRunLoopSource(null(), tap, 0);
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        let hid = open_hid_manager(run_loop, user_info);

        let _ = ready.send(Ok(Started {
            run_loop: CFRetain(run_loop) as usize,
            can_swallow,
        }));
        while !stop.load(Ordering::SeqCst) {
            if CFRunLoopRunInMode(kCFRunLoopDefaultMode, 1.0, 0) == RUN_LOOP_FINISHED {
                ((*context).on_signal)(KeySignal::Lost);
                break;
            }
        }

        CGEventTapEnable(tap, false);
        if let Some(manager) = hid {
            IOHIDManagerClose(manager, 0);
            CFRelease(manager);
        }
        CFRelease(source);
        CFRelease(tap);
        drop(Box::from_raw(context));
    }
}

/// An active tap can swallow events but needs Accessibility; a listen-only tap needs only Input
/// Monitoring.
fn create_tap(user_info: *mut c_void) -> Option<(CfRef, bool)> {
    let events = [
        EVENT_KEY_DOWN,
        EVENT_KEY_UP,
        EVENT_FLAGS_CHANGED,
        EVENT_OTHER_MOUSE_DOWN,
        EVENT_OTHER_MOUSE_UP,
    ]
    .iter()
    .fold(0_u64, |mask, event| mask | 1 << event);
    [TAP_OPTION_DEFAULT, TAP_OPTION_LISTEN_ONLY]
        .into_iter()
        .find_map(|option| {
            let tap = unsafe {
                CGEventTapCreate(
                    SESSION_EVENT_TAP,
                    HEAD_INSERT_EVENT_TAP,
                    option,
                    events,
                    on_tap_event,
                    user_info,
                )
            };
            (!tap.is_null()).then_some((tap, option == TAP_OPTION_DEFAULT))
        })
}

/// Best effort: the tap alone still reports Fn on most keyboards.
unsafe fn open_hid_manager(run_loop: CfRef, user_info: *mut c_void) -> Option<CfRef> {
    unsafe {
        let manager = IOHIDManagerCreate(null(), 0);
        if manager.is_null() {
            return None;
        }
        let matching = keyboard_matching();
        IOHIDManagerSetDeviceMatching(manager, matching);
        CFRelease(matching);
        IOHIDManagerRegisterInputValueCallback(manager, on_hid_value, user_info);
        IOHIDManagerScheduleWithRunLoop(manager, run_loop, kCFRunLoopDefaultMode);
        if IOHIDManagerOpen(manager, 0) != 0 {
            CFRelease(manager);
            return None;
        }
        Some(manager)
    }
}

/// `{ DeviceUsagePage: GenericDesktop, DeviceUsage: Keyboard }`.
unsafe fn keyboard_matching() -> CfRef {
    unsafe {
        let string = |text: &[u8]| {
            CFStringCreateWithCString(null(), text.as_ptr().cast(), STRING_ENCODING_UTF8)
        };
        let number =
            |value: &i32| CFNumberCreate(null(), NUMBER_SINT32, (value as *const i32).cast());
        let keys = [string(b"DeviceUsagePage\0"), string(b"DeviceUsage\0")];
        let values = [number(&USAGE_PAGE_GENERIC_DESKTOP), number(&USAGE_KEYBOARD)];
        let dictionary = CFDictionaryCreate(
            null(),
            keys.as_ptr(),
            values.as_ptr(),
            2,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        );
        for object in keys.into_iter().chain(values) {
            CFRelease(object);
        }
        dictionary
    }
}

extern "C" fn on_tap_event(
    _proxy: *mut c_void,
    kind: u32,
    event: CgEvent,
    user_info: *mut c_void,
) -> CgEvent {
    let context = unsafe { &mut *user_info.cast::<Context>() };
    if matches!(
        kind,
        EVENT_TAP_DISABLED_BY_TIMEOUT | EVENT_TAP_DISABLED_BY_USER_INPUT
    ) {
        unsafe { CGEventTapEnable(context.tap, true) };
        (context.on_signal)(KeySignal::Lost);
        return event;
    }
    if unsafe { CGEventGetIntegerValueField(event, FIELD_SOURCE_PID) } == context.own_pid {
        return event; // our own paste keystroke
    }
    match key_event(kind, event) {
        Some(key_event) if (context.on_signal)(KeySignal::Key(key_event)) => null_mut(),
        _ => event,
    }
}

fn key_event(kind: u32, event: CgEvent) -> Option<KeyEvent> {
    let field = |field| unsafe { CGEventGetIntegerValueField(event, field) };
    let keycode = field(FIELD_KEYCODE) as u16;
    match kind {
        EVENT_KEY_DOWN => Some(KeyEvent::press(mac_key(keycode))),
        EVENT_KEY_UP => Some(KeyEvent::release(mac_key(keycode))),
        EVENT_FLAGS_CHANGED => {
            let flags = unsafe { CGEventGetFlags(event) };
            let key = mac_key(keycode);
            mac_modifier_pressed(keycode, flags).map(|pressed| KeyEvent { key, pressed })
        }
        EVENT_OTHER_MOUSE_DOWN | EVENT_OTHER_MOUSE_UP => {
            let key = Key::Mouse(match field(FIELD_MOUSE_BUTTON_NUMBER) {
                2 => MouseButton::Middle,
                3 => MouseButton::Back,
                4 => MouseButton::Forward,
                other => MouseButton::Other(other as u8),
            });
            let pressed = kind == EVENT_OTHER_MOUSE_DOWN;
            Some(KeyEvent { key, pressed })
        }
        _ => None,
    }
}

extern "C" fn on_hid_value(
    user_info: *mut c_void,
    _result: i32,
    _sender: *mut c_void,
    value: CfRef,
) {
    let context = unsafe { &mut *user_info.cast::<Context>() };
    let (page, usage, level) = unsafe {
        let element = IOHIDValueGetElement(value);
        (
            IOHIDElementGetUsagePage(element),
            IOHIDElementGetUsage(element),
            IOHIDValueGetIntegerValue(value),
        )
    };
    if USAGE_PAGES_FN.contains(&page) && usage == USAGE_FN {
        let pressed = level != 0;
        (context.on_signal)(KeySignal::Key(KeyEvent {
            key: Key::Fn,
            pressed,
        }));
    }
}
