//! The window's disk work, on a thread of its own: a long transcript or a big library never holds
//! up the UI. Tasks run in the order they were asked for, so an open after a save sees the save.

use std::path::Path;
use std::sync::mpsc::Receiver;

use wf_meetings::{
    Error, MeetingStore, SearchIndex, SpeakerId, meeting_detail, meeting_entries, write_export,
};

use super::{Event, Mailbox};

pub(super) enum Task {
    List,
    Search(String),
    Open(String),
    SaveNotes {
        id: String,
        text: String,
    },
    SaveTitle {
        id: String,
        title: String,
    },
    NameSpeaker {
        id: String,
        speaker: SpeakerId,
        name: String,
    },
    Export {
        id: String,
        when: String,
    },
    Delete(String),
}

pub(super) fn run(
    store: &MeetingStore,
    export_dir: &Path,
    tasks: Receiver<Task>,
    mailbox: &Mailbox,
) {
    let mut index = None;
    for task in tasks {
        if let Err(problem) = perform(task, store, export_dir, &mut index, mailbox) {
            mailbox.post(Event::Failed(problem));
        }
    }
}

fn perform(
    task: Task,
    store: &MeetingStore,
    export_dir: &Path,
    index: &mut Option<SearchIndex>,
    mailbox: &Mailbox,
) -> Result<(), String> {
    let wrong = |what: &str, error: Error| format!("{what}: {error}");
    match task {
        Task::List => {
            *index = None;
            let entries =
                meeting_entries(store).map_err(|e| wrong("Couldn't read the meetings", e))?;
            mailbox.post(Event::Listed(entries));
        }
        Task::Search(query) => {
            if index.is_none() {
                let built = SearchIndex::build(store).map_err(|e| wrong("Couldn't search", e))?;
                *index = Some(built);
            }
            if let Some(index) = index {
                let hits = index.search(&query);
                mailbox.post(Event::Found { query, hits });
            }
        }
        Task::Open(id) => {
            let detail =
                meeting_detail(store, &id).map_err(|e| wrong("Couldn't open the meeting", e))?;
            mailbox.post(Event::Opened(Box::new(detail)));
        }
        Task::SaveNotes { id, text } => {
            let saved = store
                .open(&id)
                .and_then(|mut meeting| meeting.set_notes(&text));
            saved.map_err(|e| wrong("Couldn't save the notes", e))?;
            *index = None;
        }
        Task::SaveTitle { id, title } => {
            let title = title.trim();
            let saved = store
                .open(&id)
                .and_then(|mut meeting| meeting.set_title(Some(title).filter(|t| !t.is_empty())));
            saved.map_err(|e| wrong("Couldn't save the title", e))?;
            *index = None;
            mailbox.post(Event::Changed);
        }
        Task::NameSpeaker { id, speaker, name } => {
            let saved = store
                .open(&id)
                .and_then(|mut meeting| meeting.rename_speaker(speaker, name.trim()));
            saved.map_err(|e| wrong("Couldn't save the name", e))?;
            *index = None;
            mailbox.post(Event::Renamed(id));
        }
        Task::Export { id, when } => {
            let path = meeting_detail(store, &id)
                .and_then(|detail| write_export(export_dir, &detail, &when))
                .map_err(|e| wrong("Couldn't export the meeting", e))?;
            mailbox.post(Event::Exported(path));
        }
        Task::Delete(id) => {
            store
                .delete(&id)
                .map_err(|e| wrong("Couldn't delete the meeting", e))?;
            mailbox.post(Event::Deleted(id));
        }
    }
    Ok(())
}
