//! Quieting what plays while the user dictates (spec §3): the mute of the default output, and the
//! players that can say whether they are playing. What to do and when is `wf_dictation`'s.

use thiserror::Error;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::SystemMedia;

/// The sound output in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Stable across launches and reconnects, so it can be written down and found again.
    pub device: String,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MediaError {
    #[error("no sound output found")]
    NoOutput,
    #[error("this sound output can't be muted")]
    NoMuteControl,
    #[error("that sound output is gone")]
    OutputGone,
    #[error("the system refused: {0}")]
    Refused(String),
}

/// The real system call behind each step. Some of these wait on other apps, so never call them
/// from the key path or the insertion.
pub trait MediaControl: Send + Sync {
    fn output(&self) -> Result<Output, MediaError>;
    /// `device` is an [`Output::device`], which may have gone away since.
    fn set_muted(&self, device: &str, muted: bool) -> Result<(), MediaError>;
    /// Pauses every player that says it is playing, and returns those. Leaves the rest alone and
    /// never starts anything.
    fn pause_players(&self) -> Vec<String>;
    /// Resumes players from [`Self::pause_players`] that are still running and still paused.
    fn resume_players(&self, players: &[String]);
    /// Brings up the OS's question about controlling the players, at a moment that suits it.
    fn ask_players(&self);
}

/// No backend outside macOS yet.
#[cfg(not(target_os = "macos"))]
pub struct SystemMedia;

#[cfg(not(target_os = "macos"))]
impl MediaControl for SystemMedia {
    fn output(&self) -> Result<Output, MediaError> {
        Err(MediaError::NoOutput)
    }

    fn set_muted(&self, _device: &str, _muted: bool) -> Result<(), MediaError> {
        Err(MediaError::NoOutput)
    }

    fn pause_players(&self) -> Vec<String> {
        Vec::new()
    }

    fn resume_players(&self, _players: &[String]) {}

    fn ask_players(&self) {}
}
