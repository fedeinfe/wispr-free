//! Developer tasks behind `cargo xtask`.

#[cfg(feature = "meeting")]
pub mod asr_bench;
pub mod asr_score;
pub mod bundle;
pub mod eval;
pub mod meeting_eval;
pub mod meeting_fixture;
#[cfg(feature = "meeting")]
pub mod meeting_run;
pub mod speech_fixture;
