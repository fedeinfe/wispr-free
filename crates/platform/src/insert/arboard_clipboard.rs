//! Windows and Linux clipboard through arboard, which Slint already depends on. It keeps text
//! and images; other formats (file lists, rich text) are not restored.

use arboard::ImageData;

use super::{ChangeMark, Clipboard, ClipboardItem, ClipboardSnapshot, InsertError};

const TEXT_FORMAT: &str = "text/plain";
const IMAGE_FORMAT: &str = "image/rgba";

fn clipboard_error(error: arboard::Error) -> InsertError {
    InsertError::Clipboard(error.to_string())
}

/// Holds its arboard handle: on Linux the text stays pasteable only while the handle lives.
#[derive(Default)]
pub struct SystemClipboard {
    handle: Option<arboard::Clipboard>,
    #[cfg(target_os = "linux")]
    last_written: Option<String>,
}

impl SystemClipboard {
    pub fn new() -> Self {
        Self::default()
    }

    fn handle(&mut self) -> Result<&mut arboard::Clipboard, InsertError> {
        if self.handle.is_none() {
            self.handle = Some(arboard::Clipboard::new().map_err(clipboard_error)?);
        }
        Ok(self.handle.as_mut().expect("just set"))
    }
}

impl Clipboard for SystemClipboard {
    fn snapshot(&mut self) -> Result<ClipboardSnapshot, InsertError> {
        let clipboard = self.handle()?;
        let mut formats = Vec::new();
        if let Ok(text) = clipboard.get_text() {
            formats.push((TEXT_FORMAT.to_owned(), text.into_bytes()));
        }
        if let Ok(image) = clipboard.get_image() {
            let format = format!("{IMAGE_FORMAT};{}x{}", image.width, image.height);
            formats.push((format, image.bytes.into_owned()));
        }
        Ok(ClipboardSnapshot {
            items: vec![ClipboardItem { formats }],
        })
    }

    fn write_text(&mut self, text: &str, transient: bool) -> Result<ChangeMark, InsertError> {
        let set = self.handle()?.set();
        let set = if transient {
            exclude_from_history(set)
        } else {
            set
        };
        set.text(text).map_err(clipboard_error)?;
        #[cfg(target_os = "linux")]
        {
            self.last_written = Some(text.to_owned());
        }
        Ok(ChangeMark(change_count()))
    }

    #[cfg(target_os = "windows")]
    fn changed_since(&mut self, mark: ChangeMark) -> bool {
        change_count() != mark.0
    }

    /// X11 and Wayland have no change counter, so compare with what we wrote.
    #[cfg(target_os = "linux")]
    fn changed_since(&mut self, _mark: ChangeMark) -> bool {
        let written = self.last_written.clone();
        let current = self.handle().ok().and_then(|c| c.get_text().ok());
        current != written
    }

    /// Text wins when the clipboard held both: arboard can't write them together.
    fn restore(&mut self, snapshot: ClipboardSnapshot) -> Result<(), InsertError> {
        let clipboard = self.handle()?;
        let formats = snapshot.items.into_iter().flat_map(|item| item.formats);
        let (texts, images): (Vec<_>, Vec<_>) =
            formats.partition(|(format, _)| format == TEXT_FORMAT);
        if let Some((_, bytes)) = texts.into_iter().next() {
            let text = String::from_utf8_lossy(&bytes);
            clipboard.set_text(text).map_err(clipboard_error)
        } else if let Some((format, bytes)) = images.into_iter().next() {
            let (width, height) = image_size(&format)
                .ok_or_else(|| InsertError::Clipboard(format!("bad image format {format}")))?;
            let image = ImageData {
                width,
                height,
                bytes: bytes.into(),
            };
            clipboard.set_image(image).map_err(clipboard_error)
        } else {
            clipboard.clear().map_err(clipboard_error)
        }
    }
}

fn image_size(format: &str) -> Option<(usize, usize)> {
    let (_, size) = format.split_once(';')?;
    let (width, height) = size.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

#[cfg(target_os = "windows")]
fn exclude_from_history(set: arboard::Set<'_>) -> arboard::Set<'_> {
    use arboard::SetExtWindows;
    set.exclude_from_history()
}

#[cfg(target_os = "linux")]
fn exclude_from_history(set: arboard::Set<'_>) -> arboard::Set<'_> {
    use arboard::SetExtLinux;
    set.exclude_from_history()
}

#[cfg(target_os = "windows")]
fn change_count() -> u64 {
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    u64::from(unsafe { GetClipboardSequenceNumber() })
}

#[cfg(target_os = "linux")]
fn change_count() -> u64 {
    0
}
