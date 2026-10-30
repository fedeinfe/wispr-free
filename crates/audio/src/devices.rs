//! Which microphone to use: the user's ranking (settings), then the system default.

/// An input device as the OS reports it. `id` is stable across runs and is what rankings store;
/// `name` is for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDevice {
    pub id: String,
    pub name: String,
}

/// Orders `available` devices for use: devices in `ranking` (most preferred first, unplugged ones
/// skipped), then the system default, then the rest as reported. The first entry is the one to
/// open; when it goes away, rank again.
pub fn rank_devices(
    available: &[InputDevice],
    ranking: &[String],
    default_id: Option<&str>,
) -> Vec<InputDevice> {
    let preferred = ranking.iter().map(String::as_str).chain(default_id);
    let mut ranked: Vec<InputDevice> = Vec::with_capacity(available.len());
    for id in preferred {
        if let Some(device) = available.iter().find(|d| d.id == id)
            && !ranked.contains(device)
        {
            ranked.push(device.clone());
        }
    }
    for device in available {
        if !ranked.contains(device) {
            ranked.push(device.clone());
        }
    }
    ranked
}
