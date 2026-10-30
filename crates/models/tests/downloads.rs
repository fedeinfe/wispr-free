//! Resumable, SHA-256-verified downloads into the model store, against a local HTTP server.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};

use support::{Route, TestServer, model, payload};
use wf_models::{DownloadError, ModelStatus, ModelStore, Progress};

#[test]
fn downloading_a_model_installs_every_file_and_reports_progress() {
    let server = TestServer::start();
    let encoder = payload(300_000);
    let tokens = b"a 0\nb 1\n".to_vec();
    let entry = model(
        "parakeet",
        vec![
            server.file("/p/encoder.onnx", &encoder),
            server.file("/p/tokens.txt", &tokens),
        ],
    );
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());
    assert_eq!(store.status(&entry), ModelStatus::NotInstalled);

    let mut last = Progress::default();
    store
        .download(&entry, &AtomicBool::new(false), &mut |p| last = p)
        .unwrap();

    assert_eq!(store.status(&entry), ModelStatus::Installed);
    assert_eq!(
        std::fs::read(store.model_dir("parakeet").join("encoder.onnx")).unwrap(),
        encoder
    );
    assert_eq!(
        std::fs::read(store.model_dir("parakeet").join("tokens.txt")).unwrap(),
        tokens
    );
    assert_eq!(
        last,
        Progress {
            downloaded: 300_008,
            total: 300_008
        }
    );
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn an_interrupted_download_resumes_with_a_range_request() {
    let server = TestServer::start();
    let body = payload(500_000);
    let mut file = server.file("/m/model.bin", &body);
    file.url = server.route(
        "/m/model.bin",
        Route {
            body: body.clone(),
            cut_first_after: Some(200_000),
            ..Route::default()
        },
    );
    let entry = model("m", vec![file]);
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());

    assert!(store.download(&entry, &no_cancel(), &mut |_| {}).is_err());
    let ModelStatus::Partial { downloaded } = store.status(&entry) else {
        panic!(
            "expected a partial download, got {:?}",
            store.status(&entry)
        );
    };
    assert!(downloaded > 0 && downloaded <= 200_000);

    store.download(&entry, &no_cancel(), &mut |_| {}).unwrap();
    assert_eq!(store.status(&entry), ModelStatus::Installed);
    assert_eq!(
        std::fs::read(store.model_dir("m").join("model.bin")).unwrap(),
        body
    );
    let ranges: Vec<_> = server.requests().into_iter().map(|r| r.range).collect();
    assert_eq!(ranges, [None, Some(format!("bytes={downloaded}-"))]);
}

#[test]
fn a_file_that_does_not_match_its_checksum_is_rejected_and_removed() {
    let server = TestServer::start();
    let mut file = server.file("/m/model.bin", &payload(1000));
    file.sha256 = support::sha256_hex(b"something else");
    let entry = model("m", vec![file]);
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());

    let err = store
        .download(&entry, &no_cancel(), &mut |_| {})
        .unwrap_err();
    assert!(
        matches!(err, DownloadError::ChecksumMismatch { .. }),
        "{err}"
    );
    assert_eq!(store.status(&entry), ModelStatus::NotInstalled);
}

#[test]
fn a_server_without_range_support_restarts_the_file_from_scratch() {
    let server = TestServer::start();
    let body = payload(400_000);
    let mut file = server.file("/m/model.bin", &body);
    file.url = server.route(
        "/m/model.bin",
        Route {
            body: body.clone(),
            cut_first_after: Some(100_000),
            ignore_range: true,
            ..Route::default()
        },
    );
    let entry = model("m", vec![file]);
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());

    assert!(store.download(&entry, &no_cancel(), &mut |_| {}).is_err());
    store.download(&entry, &no_cancel(), &mut |_| {}).unwrap();
    assert_eq!(
        std::fs::read(store.model_dir("m").join("model.bin")).unwrap(),
        body
    );
}

#[test]
fn cancelling_keeps_the_partial_file_for_a_later_resume() {
    let server = TestServer::start();
    let entry = model("m", vec![server.file("/m/model.bin", &payload(2_000_000))]);
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());

    let cancel = AtomicBool::new(false);
    let err = store
        .download(&entry, &cancel, &mut |p| {
            if p.downloaded >= 500_000 {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
    assert!(matches!(err, DownloadError::Cancelled), "{err}");
    assert!(
        matches!(store.status(&entry), ModelStatus::Partial { downloaded } if downloaded >= 500_000)
    );

    store.download(&entry, &no_cancel(), &mut |_| {}).unwrap();
    assert_eq!(store.status(&entry), ModelStatus::Installed);
    assert!(server.requests()[1].range.is_some());
}

#[test]
fn redirects_are_followed_and_keep_the_range() {
    // Hugging Face answers /resolve/ URLs with a redirect to its CDN.
    let server = TestServer::start();
    let body = payload(300_000);
    let mut file = server.file("/cdn/model.bin", &body);
    server.route(
        "/cdn/model.bin",
        Route {
            body: body.clone(),
            cut_first_after: Some(120_000),
            ..Route::default()
        },
    );
    file.url = server.route(
        "/resolve/model.bin",
        Route {
            redirect_to: Some(format!("{}/cdn/model.bin", server.base)),
            ..Route::default()
        },
    );
    let entry = model("m", vec![file]);
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());

    assert!(store.download(&entry, &no_cancel(), &mut |_| {}).is_err());
    store.download(&entry, &no_cancel(), &mut |_| {}).unwrap();
    assert_eq!(
        std::fs::read(store.model_dir("m").join("model.bin")).unwrap(),
        body
    );
    let cdn: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.path == "/cdn/model.bin")
        .collect();
    assert!(cdn[1].range.is_some(), "{cdn:?}");
}

#[test]
fn verify_rehashes_installed_files_and_delete_removes_the_model() {
    let server = TestServer::start();
    let entry = model(
        "m",
        vec![
            server.file("/m/a.bin", &payload(10_000)),
            server.file("/m/b.bin", b"tokens"),
        ],
    );
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());
    assert!(!store.verify(&entry).unwrap());

    store.download(&entry, &no_cancel(), &mut |_| {}).unwrap();
    assert!(store.verify(&entry).unwrap());

    // Same size, different content: only a full hash notices.
    std::fs::write(store.model_dir("m").join("b.bin"), b"TOKENS").unwrap();
    assert_eq!(store.status(&entry), ModelStatus::Installed);
    assert!(!store.verify(&entry).unwrap());

    store.delete(&entry).unwrap();
    assert_eq!(store.status(&entry), ModelStatus::NotInstalled);
    assert!(!store.model_dir("m").exists());
    store.delete(&entry).unwrap(); // deleting twice is fine
}

#[test]
fn the_default_store_lives_in_the_app_data_folder() {
    let root = ModelStore::default_root().expect("a home folder");
    assert!(root.ends_with("models"), "{root:?}");
    assert!(root.to_string_lossy().contains("wispr-free"), "{root:?}");
}
