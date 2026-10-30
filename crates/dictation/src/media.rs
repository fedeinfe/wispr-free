//! Media while dictating, and undoing it. System calls here can be slow or wait on another app, so
//! they run on their own thread and a key-down never waits for one. The coordinator says when; this
//! only ever undoes what it did itself.
//!
//! Outputs we muted are written down before the mute and crossed off after the unmute, so a crash
//! can't leave the Mac muted for good: the next launch finishes the job, and an output that was
//! unplugged stays on the list until it can be reached. Paused players aren't written down. Music
//! coming on by itself at some later launch would be worse than staying paused.

use std::fs;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use wf_platform::media::{MediaControl, MediaError};

use crate::Event;

pub(crate) enum Job {
    Mute,
    Pause,
    /// Puts back everything done so far.
    Release,
    /// Pause was just chosen, so this is the moment for the OS's question.
    AskPlayers,
    /// The app is quitting. Says when the sound is back.
    Close(Sender<()>),
}

struct Keeper {
    control: Arc<dyn MediaControl>,
    note: PathBuf,
    /// Outputs we muted and still owe an unmute, as in the note.
    muted: Vec<String>,
    paused: Vec<String>,
    warned: bool,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
}

pub(crate) fn run(
    jobs: Receiver<Job>,
    control: Arc<dyn MediaControl>,
    note: PathBuf,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
) {
    let mut keeper = Keeper {
        muted: read_note(&note),
        control,
        note,
        paused: Vec::new(),
        warned: false,
        on_event,
    };
    keeper.unmute_owed();
    for job in jobs {
        match job {
            Job::Mute => keeper.mute(),
            Job::Pause => keeper.pause(),
            Job::Release => keeper.release(),
            Job::AskPlayers => keeper.control.ask_players(),
            Job::Close(done) => {
                keeper.release();
                let _ = done.send(());
            }
        }
    }
    keeper.release();
}

impl Keeper {
    fn mute(&mut self) {
        self.unmute_owed();
        let output = match self.control.output() {
            Ok(output) => output,
            Err(error) => return self.warn(&error),
        };
        // Muted already, by the user or by us with the unmute still owed. Not ours to change.
        if output.muted {
            return;
        }
        self.muted.push(output.device.clone());
        if let Err(error) = self.write_note() {
            self.muted.pop();
            return self.warn(&MediaError::Refused(error.to_string()));
        }
        if let Err(error) = self.control.set_muted(&output.device, true) {
            self.muted.pop();
            let _ = self.write_note();
            self.warn(&error);
        }
    }

    fn pause(&mut self) {
        self.paused.extend(self.control.pause_players());
    }

    fn release(&mut self) {
        self.unmute_owed();
        let paused = mem::take(&mut self.paused);
        if !paused.is_empty() {
            self.control.resume_players(&paused);
        }
    }

    fn unmute_owed(&mut self) {
        let count = self.muted.len();
        let control = &self.control;
        self.muted
            .retain(|device| control.set_muted(device, false).is_err());
        if self.muted.len() < count {
            let _ = self.write_note();
        }
    }

    fn write_note(&self) -> io::Result<()> {
        if self.muted.is_empty() {
            return fs::remove_file(&self.note);
        }
        let text = serde_json::to_string(&self.muted).map_err(io::Error::other)?;
        if let Some(parent) = self.note.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut temporary = self.note.as_os_str().to_owned();
        temporary.push(".tmp");
        fs::write(&temporary, text)?;
        fs::rename(&temporary, &self.note)
    }

    /// Once per run: an output with no mute would say so on every dictation.
    fn warn(&mut self, error: &MediaError) {
        if !mem::replace(&mut self.warned, true) {
            (self.on_event)(Event::Toast(format!("Couldn't mute: {error}")));
        }
    }
}

/// The outputs a run before this one left muted. A damaged note can't be acted on, so it goes.
fn read_note(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|_| {
        let _ = fs::remove_file(path);
        Vec::new()
    })
}
