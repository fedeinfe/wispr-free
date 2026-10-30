//! Touches the real internet, so it's ignored by default.
//! Run with `cargo test -p wf-models --test live_download -- --ignored`.

use std::fs;
use std::sync::atomic::AtomicBool;

use wf_models::{Catalog, ModelStatus, ModelStore};

#[test]
#[ignore = "downloads about 2 MB from Hugging Face"]
fn silero_downloads_over_https_and_resumes_through_the_cdn() {
    let catalog = Catalog::bundled();
    let silero = catalog.model("silero-vad-v5").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());
    let cancel = AtomicBool::new(false);

    store.download(silero, &cancel, &mut |_| {}).unwrap();
    assert!(store.verify(silero).unwrap());

    // Turn the finished file back into a partial one and resume it for real.
    let file = store.model_dir(&silero.id).join(&silero.files[0].name);
    let part = store.model_dir(&silero.id).join("silero_vad.onnx.part");
    let bytes = fs::read(&file).unwrap();
    fs::write(&part, &bytes[..1_000_000]).unwrap();
    fs::remove_file(&file).unwrap();
    assert_eq!(
        store.status(silero),
        ModelStatus::Partial {
            downloaded: 1_000_000
        }
    );

    let mut first = None;
    store
        .download(silero, &cancel, &mut |p| {
            first.get_or_insert(p.downloaded);
        })
        .unwrap();
    assert!(store.verify(silero).unwrap());
    assert!(
        first.unwrap() > 1_000_000,
        "restarted instead of resuming: {first:?}"
    );
}
