//! Developer tasks behind `cargo xtask`.

#[cfg(feature = "meeting")]
pub mod asr_bench;
pub mod asr_score;
pub mod ax_check;
pub mod bridge;
pub mod bundle;
pub mod dev_app;
pub mod eval;
pub mod meeting_eval;
pub mod meeting_fixture;
#[cfg(feature = "meeting")]
pub mod meeting_run;
pub mod meeting_seed;
pub mod speech_fixture;
