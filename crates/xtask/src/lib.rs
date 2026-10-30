//! Developer tasks behind `cargo xtask`.

pub mod eval;
pub mod meeting_eval;
pub mod meeting_fixture;
#[cfg(feature = "meeting")]
pub mod meeting_run;
