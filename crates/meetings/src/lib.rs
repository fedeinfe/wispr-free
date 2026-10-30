//! The meeting notetaker: two-channel capture ("You" = mic, "Others" = system audio) with a
//! crash-safe incremental writer (ticket 15), live transcript, refine pass, diarization and
//! summary (16), and meeting chat (17).

mod align;
mod chat;
mod detect;
mod diarize;
mod library;
mod live;
mod mic_clients;
mod ogg;
mod process;
mod reader;
mod recorder;
mod refine;
mod store;
mod summary;
mod system_audio;
mod transcript;
mod writer;

pub use align::{Channel, StereoAligner, StereoFrame};
pub use chat::{
    AnswerBlock, ChatOptions, ChatPrompt, ChatSource, ChatTurn, Citation, ParsedAnswer, ask,
    chat_prompt, parse_answer, suggested_questions,
};
pub use detect::{MeetingApp, MeetingDetector, MicClient, classify_client, parse_pw_dump};
pub use diarize::{Turn, assign_speakers, split_at_turns, stable_turns};
pub use library::{
    MeetingDetail, MeetingEntry, SearchHit, SearchIndex, export_file_name, export_markdown,
    meeting_detail, meeting_entries, write_export,
};
pub use live::{LiveEngines, LiveEvent, LiveOptions, LiveTranscript};
pub use mic_clients::{mic_clients, other_app_playing};
pub use process::{MeetingEngines, MeetingPlan, ProcessOptions, ProcessProgress, finish_meeting};
pub use reader::{MeetingAudio, read_meeting, repair_meeting};
pub use recorder::{MeetingConfig, MeetingEvent, MeetingRecorder, TimelineFrame};
pub use refine::{RefineOptions, fill_gaps, refine_transcript};
pub use store::{
    LiveLog, Meeting, MeetingRecord, MeetingStore, NewPart, Part, RefinedTranscript, Stage,
    StageState, Stages,
};
pub use summary::{
    ActionItem, Item, Notes, ParsedSummary, Summary, SummaryMode, SummaryOptions, SummaryProgress,
    TranscriptChunk, generate_summary, parse_notes, parse_summary, rename_speaker_labels,
    transcript_chunks,
};
pub use system_audio::SystemAudioBackend;
pub use transcript::{
    Gap, GapReason, Segment, SpeakerId, SpeakerNames, Transcript, TranscriptRow, parse_timestamp,
    timestamp,
};
pub use writer::MeetingWriter;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("meeting file i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("opus codec: {0}")]
    Codec(#[from] opus::Error),
    #[error("{0} is not a meeting recording")]
    NotAMeeting(std::path::PathBuf),
    #[error("audio capture: {0}")]
    Audio(#[from] wf_audio::Error),
    #[error("the meeting writer thread panicked")]
    WriterPanicked,
    #[error("meeting record: {0}")]
    Record(#[from] serde_json::Error),
    #[error("no meeting {0}")]
    NoSuchMeeting(String),
    #[error("{0} was written by a newer version of the app")]
    NewerRecord(std::path::PathBuf),
    #[error("a recording is still running")]
    StillRecording,
    #[error("the transcript is not final yet, so the audio has to stay")]
    TranscriptNotFinal,
    #[error("this meeting's audio was deleted")]
    AudioDeleted,
    #[error("the transcript is empty")]
    NothingToSummarize,
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    Engine(#[from] wf_pipeline::PipelineError),
}
