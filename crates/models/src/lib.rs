//! The model catalog (bundled, versioned manifest), resumable verified downloads, and
//! hardware detection that recommends a pipeline tier (ticket 11).

mod catalog;
pub mod hardware;
mod store;
mod tier;

pub use catalog::{Catalog, CatalogError, Engine, ModelEntry, ModelFile, ModelKind, TierPreset};
pub use store::{DownloadError, ModelStatus, ModelStore, Progress, resolved};
pub use tier::Tier;
