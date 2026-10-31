//! The folder picker, which `Desktop` leaves to the toolkit until `NSOpenPanel` replaces it.

use std::path::Path;
use std::rc::Rc;

use wf_shell::settings::system::PickFolder;

pub fn folder_picker() -> PickFolder {
    Rc::new(|start: &Path, done| {
        let dialog = rfd::AsyncFileDialog::new()
            .set_title("Choose a folder for models")
            .set_directory(start);
        // Not the blocking dialog: its nested run loop would re-enter Slint's event handler.
        let picked = slint::spawn_local(async move {
            if let Some(folder) = dialog.pick_folder().await {
                done(folder.path().to_path_buf());
            }
        });
        if let Err(error) = picked {
            eprintln!("could not open the folder picker: {error}");
        }
    })
}
