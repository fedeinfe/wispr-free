//! What the list-like parts of the pages show, built from plain data.

use slint::SharedString;
use wf_audio::{InputDevice, rank_devices};
use wf_core::settings::Settings;
use wf_core::shortcut::{Action, Platform};
use wf_models::hardware::{GpuBackend, HardwareInfo};
use wf_models::{ModelEntry, ModelKind, ModelStatus};

use crate::{LanguageRow, ModelRow, ShortcutRow};

/// The languages "My languages" offers: ISO 639-1 code and name.
pub const LANGUAGES: [(&str, &str); 15] = [
    ("en", "English"),
    ("it", "Italian"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("pt", "Portuguese"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("tr", "Turkish"),
    ("ar", "Arabic"),
    ("hi", "Hindi"),
    ("ja", "Japanese"),
    ("zh", "Chinese"),
];

pub fn shortcut_rows(settings: &Settings, platform: Platform) -> Vec<ShortcutRow> {
    Action::ALL
        .into_iter()
        .map(|action| ShortcutRow {
            action: action.label().into(),
            combo: settings.shortcuts.get(action).label(platform).into(),
        })
        .collect()
}

pub fn language_rows(selected: &[String]) -> Vec<LanguageRow> {
    LANGUAGES
        .into_iter()
        .map(|(code, name)| LanguageRow {
            name: name.into(),
            selected: selected.iter().any(|language| language == code),
        })
        .collect()
}

/// One line of the microphone list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrophoneLine {
    pub id: String,
    pub name: String,
    pub connected: bool,
}

/// The ranking first, unplugged devices included, then whatever else is connected.
pub fn microphone_lines(
    ranking: &[String],
    devices: &[InputDevice],
    default: Option<&str>,
) -> Vec<MicrophoneLine> {
    let line = |device: &InputDevice, connected| MicrophoneLine {
        id: device.id.clone(),
        name: device.name.clone(),
        connected,
    };
    let mut lines: Vec<_> = ranking
        .iter()
        .map(|id| match devices.iter().find(|device| device.id == *id) {
            Some(device) => line(device, true),
            None => MicrophoneLine {
                id: id.clone(),
                name: id.clone(),
                connected: false,
            },
        })
        .collect();
    for device in rank_devices(devices, ranking, default) {
        if !ranking.contains(&device.id) {
            lines.push(line(&device, true));
        }
    }
    lines
}

pub fn model_row(model: &ModelEntry, status: ModelStatus) -> ModelRow {
    let kind = match model.kind {
        ModelKind::Vad => "Voice detection",
        ModelKind::Asr => "Speech recognition",
        ModelKind::Llm => "Cleanup",
    };
    let detail = format!(
        "{kind} · {} · about {} RAM · {}",
        format_bytes(model.download_size()),
        format_bytes(model.ram_mb * 1024 * 1024),
        model.license,
    );
    let (status_text, installed, partial): (SharedString, _, _) = match status {
        ModelStatus::NotInstalled => ("Not installed".into(), false, false),
        ModelStatus::Partial { downloaded } => (
            format!("{} downloaded", format_bytes(downloaded)).into(),
            false,
            true,
        ),
        ModelStatus::Installed => ("Installed".into(), true, false),
    };
    ModelRow {
        name: model.name.as_str().into(),
        detail: detail.into(),
        status: status_text,
        installed,
        partial,
        busy: false,
        downloading: false,
        progress: 0.0,
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else if bytes >= 10.0 * MB {
        format!("{:.0} MB", bytes / MB)
    } else {
        format!("{:.1} MB", bytes / MB)
    }
}

pub fn hardware_summary(hardware: &HardwareInfo) -> String {
    let mut parts = vec![
        format!("{} CPU cores", hardware.cpu_cores),
        format!("{} RAM", format_bytes(hardware.total_ram_bytes)),
    ];
    for gpu in &hardware.gpus {
        let backend = match gpu.backend {
            GpuBackend::Metal => "Metal",
            GpuBackend::Cuda => "CUDA",
            GpuBackend::Vulkan => "Vulkan",
        };
        parts.push(format!("{} ({backend})", gpu.name));
    }
    parts.join(" · ")
}
