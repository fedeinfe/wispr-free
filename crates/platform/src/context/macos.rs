//! macOS backend: the AX API, with NSWorkspace as the fallback for the app identity.

use std::ptr::{self, NonNull};

use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{
    CFBoolean, CFDictionary, CFNumber, CFRange, CFRetained, CFString, CFType,
};
use objc2_core_graphics::{
    CGWindowListCopyWindowInfo, CGWindowListOption, kCGWindowLayer, kCGWindowOwnerPID,
};

use super::ax_role::{AxElementInfo, classify};
use super::{AppIdentity, Context, ContextDepth, ContextSource, Focus, TextAround, Utf16Window};

/// How long a busy app may stall one AX call. It applies process-wide once set.
const AX_TIMEOUT_SECONDS: f32 = 0.25;

/// True once the user granted Accessibility, without which AX reads and keystrokes do nothing.
pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Reads the focused element through the system-wide AX element. Stateless and `Send`.
pub struct SystemContext;

impl ContextSource for SystemContext {
    fn read(&self, depth: ContextDepth) -> Context {
        // Context is for the app being dictated into, and that's never this one.
        if let Some(app) = frontmost_app().filter(|app| app.pid == Some(std::process::id())) {
            return without_ax(Some(app), Focus::Unknown);
        }
        let system = unsafe { AXUIElement::new_system_wide() };
        unsafe { system.set_messaging_timeout(AX_TIMEOUT_SECONDS) };

        let app_element = match element_attribute(&system, "AXFocusedApplication") {
            Ok(element) => element,
            // Chromium apps often don't answer here until their accessibility tree is on, which
            // is done below, so the app in front is the one to ask.
            Err(AXError::NoValue) => match frontmost_app().and_then(|app| app.pid) {
                Some(pid) => unsafe { AXUIElement::new_application(pid as i32) },
                None => return without_ax(None, Focus::Nothing),
            },
            Err(_) => return without_ax(frontmost_app(), Focus::Unknown),
        };
        // Electron only builds its accessibility tree when asked; other apps ignore this.
        let enable = CFBoolean::new(true);
        let _ = unsafe {
            app_element
                .set_attribute_value(&CFString::from_static_str("AXManualAccessibility"), enable)
        };

        let app = app_of(&app_element);
        let app_has_window = app.as_ref().and_then(|app| app.pid).is_none_or(has_window);
        let window = element_attribute(&app_element, "AXFocusedWindow").ok();
        let (focus, focused) = match element_attribute(&app_element, "AXFocusedUIElement") {
            Ok(element) => (focus_of(&element), Some(element)),
            // No element and no window: an app with nothing open. With a window, the app may
            // just not expose its content, so it stays Unknown and still gets the paste.
            Err(AXError::NoValue) if window.is_none() && !app_has_window => (Focus::Nothing, None),
            Err(_) => (Focus::Unknown, None),
        };
        let text = match (depth, focus, &focused) {
            (ContextDepth::FocusOnly, ..) | (_, Focus::Secure, _) => None,
            (ContextDepth::WithText, _, Some(element)) => read_text(element),
            _ => None,
        };
        Context {
            app,
            window_title: window.and_then(|window| string_attribute(&window, "AXTitle")),
            focus,
            text,
        }
    }
}

fn without_ax(app: Option<AppIdentity>, focus: Focus) -> Context {
    Context {
        app,
        window_title: None,
        focus,
        text: None,
    }
}

/// Whether `pid` has an ordinary window on screen, counting when it can't be told. This asks the
/// window server, so it holds for an app whose accessibility tree isn't built yet.
fn has_window(pid: u32) -> bool {
    let options =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let Some(windows) = CGWindowListCopyWindowInfo(options, 0) else {
        return true;
    };
    // Every entry the window server returns is a dictionary.
    let windows = unsafe { windows.cast_unchecked::<CFDictionary<CFString, CFType>>() };
    let number = |window: &CFDictionary<CFString, CFType>, key: &CFString| {
        window.get(key)?.downcast_ref::<CFNumber>()?.as_i64()
    };
    let (owner_key, layer_key) = unsafe { (kCGWindowOwnerPID, kCGWindowLayer) };
    (0..windows.len())
        .filter_map(|index| windows.get(index))
        .any(|window| {
            number(&window, owner_key) == Some(i64::from(pid))
                && number(&window, layer_key) == Some(0)
        })
}

fn focus_of(element: &AXUIElement) -> Focus {
    let Some(role) = string_attribute(element, "AXRole") else {
        return Focus::Unknown;
    };
    let subrole = string_attribute(element, "AXSubrole");
    classify(&AxElementInfo {
        role: &role,
        subrole: subrole.as_deref(),
        editable_value: is_settable(element, "AXValue")
            && range_attribute(element, "AXSelectedTextRange").is_some(),
    })
}

fn read_text(element: &AXUIElement) -> Option<TextAround> {
    let selection = range_attribute(element, "AXSelectedTextRange")?;
    if let Some(length) = length_of(element)
        && let window = Utf16Window::around(selection.clone(), length)
        && let Some(before) = string_for_range(element, &window.before)
        && let Some(selected) = string_for_range(element, &window.selection)
        && let Some(after) = string_for_range(element, &window.after)
    {
        return Some(TextAround::from_utf16(&before, &selected, &after));
    }
    // Some views only expose their whole value.
    let value = attribute(element, "AXValue").ok()?;
    let units = utf16_units(value.downcast_ref::<CFString>()?);
    let window = Utf16Window::around(selection, units.len());
    Some(TextAround::from_utf16(
        &units[window.before],
        &units[window.selection],
        &units[window.after],
    ))
}

fn app_of(app_element: &AXUIElement) -> Option<AppIdentity> {
    let mut pid = 0;
    let error = unsafe { app_element.pid(NonNull::from(&mut pid)) };
    if error != AXError::Success {
        return frontmost_app();
    }
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .map(|app| identity_of(&app))
        .or_else(frontmost_app)
}

fn frontmost_app() -> Option<AppIdentity> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(identity_of(&app))
}

fn identity_of(app: &NSRunningApplication) -> AppIdentity {
    let name = app.localizedName().map(|name| name.to_string());
    AppIdentity {
        id: app
            .bundleIdentifier()
            .map(|id| id.to_string())
            .or_else(|| name.clone())
            .unwrap_or_default(),
        name,
        pid: u32::try_from(app.processIdentifier()).ok(),
    }
}

fn attribute(element: &AXUIElement, name: &str) -> Result<CFRetained<CFType>, AXError> {
    let mut value: *const CFType = ptr::null();
    let error = unsafe {
        element.copy_attribute_value(&CFString::from_str(name), NonNull::from(&mut value))
    };
    retained(error, value)
}

fn retained(error: AXError, value: *const CFType) -> Result<CFRetained<CFType>, AXError> {
    if error != AXError::Success {
        return Err(error);
    }
    let value = NonNull::new(value.cast_mut()).ok_or(AXError::NoValue)?;
    // The Copy rule: AX hands us an owned reference.
    Ok(unsafe { CFRetained::from_raw(value) })
}

fn element_attribute(
    element: &AXUIElement,
    name: &str,
) -> Result<CFRetained<AXUIElement>, AXError> {
    attribute(element, name)?
        .downcast::<AXUIElement>()
        .map_err(|_| AXError::Failure)
}

fn string_attribute(element: &AXUIElement, name: &str) -> Option<String> {
    let value = attribute(element, name).ok()?;
    Some(value.downcast_ref::<CFString>()?.to_string())
}

fn range_attribute(element: &AXUIElement, name: &str) -> Option<std::ops::Range<usize>> {
    let value = attribute(element, name).ok()?;
    let value = value.downcast_ref::<AXValue>()?;
    let mut range = CFRange {
        location: 0,
        length: 0,
    };
    let found = unsafe { value.value(AXValueType::CFRange, NonNull::from(&mut range).cast()) };
    // Some apps report a location of -1 or NSNotFound when there is no insertion point.
    let start = usize::try_from(range.location).ok().filter(|_| found)?;
    Some(start..start.saturating_add(usize::try_from(range.length).ok()?))
}

fn length_of(element: &AXUIElement) -> Option<usize> {
    let value = attribute(element, "AXNumberOfCharacters").ok()?;
    usize::try_from(value.downcast_ref::<CFNumber>()?.as_i64()?).ok()
}

fn is_settable(element: &AXUIElement, name: &str) -> bool {
    let mut settable = 0;
    let error = unsafe {
        element.is_attribute_settable(&CFString::from_str(name), NonNull::from(&mut settable))
    };
    error == AXError::Success && settable != 0
}

fn string_for_range(element: &AXUIElement, range: &std::ops::Range<usize>) -> Option<Vec<u16>> {
    if range.is_empty() {
        return Some(Vec::new());
    }
    let mut cf_range = CFRange {
        location: range.start as isize,
        length: range.len() as isize,
    };
    let parameter =
        unsafe { AXValue::new(AXValueType::CFRange, NonNull::from(&mut cf_range).cast()) }?;
    let mut value: *const CFType = ptr::null();
    let error = unsafe {
        element.copy_parameterized_attribute_value(
            &CFString::from_static_str("AXStringForRange"),
            &parameter,
            NonNull::from(&mut value),
        )
    };
    let value = retained(error, value).ok()?;
    Some(utf16_units(value.downcast_ref::<CFString>()?))
}

/// UTF-16 units rather than a Rust string: a read range can split a surrogate pair.
fn utf16_units(string: &CFString) -> Vec<u16> {
    let length = string.length();
    let mut units = vec![0; length as usize];
    let range = CFRange {
        location: 0,
        length,
    };
    unsafe { string.characters(range, units.as_mut_ptr()) };
    units
}
