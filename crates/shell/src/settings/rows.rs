//! What the list-like parts of the pages show, built from plain data.

use wf_audio::{InputDevice, rank_devices};
use wf_core::settings::Settings;
use wf_core::shortcut::{Action, Platform};
use wf_models::hardware::{GpuBackend, HardwareInfo};
use wf_models::{ModelEntry, ModelKind, ModelStatus};

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

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct ShortcutRow {
    pub action: String,
    pub combo: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct LanguageRow {
    pub name: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct ModelRow {
    /// Its place in the catalog, and so in the list of models.
    pub index: i32,
    pub name: String,
    pub size: String,
    /// Which pipeline stage runs on it, or empty.
    pub used_for: String,
    pub detail: String,
    pub status: String,
    pub installed: bool,
    /// Started but not finished, so Download resumes it.
    pub partial: bool,
    /// This row is downloading or verifying.
    pub busy: bool,
    pub downloading: bool,
    /// 0..1 while downloading.
    pub progress: f32,
    /// What the button that gets it says.
    pub download_title: String,
}

pub fn shortcut_rows(settings: &Settings, platform: Platform) -> Vec<ShortcutRow> {
    Action::ALL
        .into_iter()
        .map(|action| ShortcutRow {
            action: action.label().into(),
            combo: settings.shortcuts.get(action).label(platform),
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
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
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

/// `index` is the model's place in the catalog, which is also its row in the list of models.
pub fn model_row(index: usize, model: &ModelEntry, status: ModelStatus) -> ModelRow {
    let kind = match model.kind {
        ModelKind::Vad => "Voice detection",
        ModelKind::Asr => "Speech recognition",
        ModelKind::Llm => "Cleanup, summaries and chat",
        ModelKind::Segmentation => "Finding who speaks when",
        ModelKind::Embedding => "Telling voices apart",
    };
    let detail = format!(
        "{kind} · {} · about {} RAM · {}",
        format_bytes(model.download_size()),
        format_bytes(model.ram_mb * 1024 * 1024),
        model.license,
    );
    let (status_text, installed, partial): (String, _, _) = match status {
        ModelStatus::NotInstalled => ("Not downloaded".into(), false, false),
        ModelStatus::Partial { downloaded } => (
            format!("{} downloaded", format_bytes(downloaded)),
            false,
            true,
        ),
        ModelStatus::Installed => ("Downloaded".into(), true, false),
    };
    ModelRow {
        index: index as i32,
        name: model.name.as_str().into(),
        size: format_bytes(model.download_size()),
        used_for: String::new(),
        detail,
        status: status_text,
        installed,
        partial,
        busy: false,
        downloading: false,
        progress: 0.0,
        download_title: if partial { "Resume" } else { "Download" }.into(),
    }
}

/// What a chosen models folder that's gone means for a model: not "Not downloaded", which would
/// invite downloading it again.
pub fn model_row_unavailable(index: usize, model: &ModelEntry) -> ModelRow {
    ModelRow {
        status: "Folder not connected".into(),
        ..model_row(index, model, ModelStatus::NotInstalled)
    }
}

/// How many models are downloaded and how much room they take. Empty for none.
pub fn downloaded_summary(models: &[&ModelEntry]) -> String {
    let size = format_bytes(models.iter().map(|model| model.download_size()).sum());
    match models.len() {
        0 => String::new(),
        1 => format!("1 model · {size}"),
        count => format!("{count} models · {size}"),
    }
}

/// Under a stage's model menu: whether that model is on disk, and the way to get it if not.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct StageModel {
    pub row: ModelRow,
    /// A stage that's off needs no model, so nothing is missing.
    pub needed: bool,
    pub ready: bool,
    pub missing: bool,
    pub downloading: bool,
    pub text: String,
}

pub fn stage_model(row: ModelRow, needed: bool, folder_missing: bool) -> StageModel {
    let missing = needed && !folder_missing && !row.installed;
    let downloading = missing && row.downloading;
    let text = if !needed {
        "Not used: this stage is off".to_owned()
    } else if folder_missing {
        "Folder not connected".to_owned()
    } else if row.installed {
        "Downloaded".to_owned()
    } else if downloading {
        "Downloading…".to_owned()
    } else if row.partial {
        format!("{} of {}", row.status, row.size)
    } else {
        format!("Not downloaded · {}", row.size)
    };
    StageModel {
        ready: !folder_missing && row.installed,
        needed,
        missing,
        downloading,
        text,
        row,
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
