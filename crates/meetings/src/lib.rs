//! The meeting notetaker: two-channel capture ("You" = mic, "Others" = system audio) with a
//! crash-safe incremental writer (ticket 15), live transcript, refine pass, diarization and
//! summary (16), and meeting chat (17).

mod align;
mod detect;
mod ogg;
mod reader;
mod writer;

pub use align::{Channel, StereoAligner, StereoFrame};
pub use detect::{MeetingApp, MeetingDetector, MicClient, classify_client, parse_pw_dump};
pub use reader::{MeetingAudio, read_meeting, repair_meeting};
pub use writer::MeetingWriter;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("meeting file i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("opus codec: {0}")]
    Codec(#[from] opus::Error),
    #[error("{0} is not a meeting recording")]
    NotAMeeting(std::path::PathBuf),
}
