//! Linux backend: one blocking evdev reader per input device (X11 and Wayland alike). Needs read
//! access to `/dev/input/event*`, i.e. the `input` group. evdev can only observe, so events can't
//! be swallowed without grabbing the device. Devices plugged in later are not picked up.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use evdev::{Device, EventType, KeyCode};
use wf_core::shortcut::KeyEvent;

use super::keymap::linux_key;
use super::{Callback, KeySignal, KeysError};

const AUTOREPEAT: i32 = 2;

pub struct Listener {
    stop: Arc<AtomicBool>,
}

impl Listener {
    pub fn start(on_signal: Callback) -> Result<Self, KeysError> {
        let devices: Vec<Device> = evdev::enumerate()
            .map(|(_, device)| device)
            .filter(reports_keys)
            .collect();
        if devices.is_empty() {
            return Err(KeysError::Permission(
                "no readable keyboard: add your user to the input group and log in again",
            ));
        }
        let on_signal = Arc::new(Mutex::new(on_signal));
        let stop = Arc::new(AtomicBool::new(false));
        for device in devices {
            let on_signal = Arc::clone(&on_signal);
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("wf-keys".into())
                .spawn(move || read(device, &on_signal, &stop))
                .map_err(|error| KeysError::Unavailable(error.to_string()))?;
        }
        Ok(Self { stop })
    }

    pub fn can_swallow(&self) -> bool {
        false
    }
}

impl Drop for Listener {
    /// Readers block in `read`, so each one exits on its device's next event.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn reports_keys(device: &Device) -> bool {
    device.supported_keys().is_some_and(|keys| {
        keys.contains(KeyCode::KEY_A)
            || keys.contains(KeyCode::BTN_MIDDLE)
            || keys.contains(KeyCode::BTN_SIDE)
    })
}

fn read(mut device: Device, on_signal: &Mutex<Callback>, stop: &AtomicBool) {
    let signal = |signal| {
        if let Ok(mut on_signal) = on_signal.lock() {
            on_signal(signal);
        }
    };
    loop {
        let Ok(events) = device.fetch_events() else {
            signal(KeySignal::Lost); // unplugged, or access revoked
            return;
        };
        for event in events {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            if event.event_type() == EventType::KEY && event.value() != AUTOREPEAT {
                let key = linux_key(event.code());
                let pressed = event.value() != 0;
                signal(KeySignal::Key(KeyEvent { key, pressed }));
            }
        }
    }
}
