//! A models folder the user chose, such as one on an external drive: it can vanish, it holds the
//! user's other files, and moving to it must never lose a model.

mod support;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use support::{TestServer, model, payload};
use wf_models::{DownloadError, ModelEntry, ModelStatus, ModelStore, Progress};

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

fn installed(store: &ModelStore, entry: &ModelEntry, bytes: &[u8]) {
    let dir = store.model_dir(&entry.id);
    fs::create_dir_all(&dir).unwrap();
    for file in &entry.files {
        fs::write(dir.join(&file.name), bytes).unwrap();
    }
}

fn entry(server: &TestServer, id: &str, body: &[u8]) -> ModelEntry {
    model(id, vec![server.file(&format!("/{id}/model.bin"), body)])
}

fn commit_ok() -> impl FnOnce() -> std::io::Result<()> {
    || Ok(())
}

#[test]
fn a_missing_chosen_folder_refuses_downloads_and_creates_nothing() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let drive = dir.path().join("Volumes/SSD/models");
    let store = ModelStore::in_folder(&drive);
    let body = payload(1000);

    assert!(!store.available());
    let error = store
        .download(&entry(&server, "m", &body), &no_cancel(), &mut |_| {})
        .unwrap_err();

    assert!(matches!(error, DownloadError::FolderUnavailable(path) if path == drive));
    assert!(
        !dir.path().join("Volumes").exists(),
        "nothing on the wrong disk"
    );
}

#[test]
fn the_apps_own_folder_is_created_on_demand() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path().join("data/models"));
    let body = payload(1000);
    let entry = entry(&server, "m", &body);

    assert!(store.available());
    store.download(&entry, &no_cancel(), &mut |_| {}).unwrap();

    assert_eq!(store.status(&entry), ModelStatus::Installed);
}

#[test]
fn configured_uses_the_chosen_folder_or_falls_back_to_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let chosen = dir.path().join("gone");
    let default = dir.path().join("default");

    let store = ModelStore::configured(Some(&chosen), &default);
    assert_eq!(store.root(), chosen);
    assert!(!store.available());

    let store = ModelStore::configured(None, &default);
    assert_eq!(store.root(), default);
    assert!(store.available());
}

#[test]
fn models_already_in_a_chosen_folder_are_recognized() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let entry = entry(&server, "m", &body);
    let store = ModelStore::in_folder(dir.path());
    installed(&store, &entry, &body);

    assert_eq!(store.status(&entry), ModelStatus::Installed);
}

#[test]
fn moving_copies_everything_commits_then_removes_the_originals() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(300_000);
    let done = entry(&server, "done", &body);
    let partial = entry(&server, "partial", &body);
    let absent = entry(&server, "absent", &body);
    let source = ModelStore::new(dir.path().join("old"));
    let drive = dir.path().join("ssd");
    fs::create_dir(&drive).unwrap();
    let target = ModelStore::in_folder(&drive);
    installed(&source, &done, &body);
    let partial_dir = source.model_dir("partial");
    fs::create_dir_all(&partial_dir).unwrap();
    fs::write(partial_dir.join("model.bin.part"), &body[..1000]).unwrap();

    let mut last = Progress::default();
    let mut originals_at_commit = false;
    source
        .move_to(
            &target,
            &[done.clone(), partial.clone(), absent.clone()],
            &no_cancel(),
            &mut |p| last = p,
            || {
                originals_at_commit = source.model_dir("done").exists();
                Ok(())
            },
        )
        .unwrap();

    assert!(originals_at_commit, "nothing is removed before the commit");
    assert_eq!(target.status(&done), ModelStatus::Installed);
    assert_eq!(
        fs::read(target.model_dir("done").join("model.bin")).unwrap(),
        body
    );
    assert_eq!(
        target.status(&partial),
        ModelStatus::Partial { downloaded: 1000 }
    );
    assert_eq!(target.status(&absent), ModelStatus::NotInstalled);
    assert!(!source.model_dir("done").exists());
    assert!(!source.model_dir("partial").exists());
    assert_eq!(
        last,
        Progress {
            downloaded: 301_000,
            total: 301_000
        }
    );
}

#[test]
fn a_failed_commit_keeps_the_originals() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let entry = entry(&server, "m", &body);
    let source = ModelStore::new(dir.path().join("old"));
    let target_dir = dir.path().join("new");
    fs::create_dir(&target_dir).unwrap();
    installed(&source, &entry, &body);

    let error = source
        .move_to(
            &ModelStore::in_folder(&target_dir),
            std::slice::from_ref(&entry),
            &no_cancel(),
            &mut |_| {},
            || Err(std::io::Error::other("disk full")),
        )
        .unwrap_err();

    assert!(matches!(error, DownloadError::Io(_)));
    assert_eq!(source.status(&entry), ModelStatus::Installed);
}

#[test]
fn cancelling_a_move_keeps_the_originals_and_leaves_no_half_file() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(300_000);
    let entry = entry(&server, "m", &body);
    let source = ModelStore::new(dir.path().join("old"));
    let target_dir = dir.path().join("new");
    fs::create_dir(&target_dir).unwrap();
    let target = ModelStore::in_folder(&target_dir);
    installed(&source, &entry, &body);
    let cancel = AtomicBool::new(false);

    let error = source
        .move_to(
            &target,
            std::slice::from_ref(&entry),
            &cancel,
            &mut |_| cancel.store(true, Ordering::Relaxed),
            commit_ok(),
        )
        .unwrap_err();

    assert!(matches!(error, DownloadError::Cancelled));
    assert_eq!(source.status(&entry), ModelStatus::Installed);
    let left: Vec<_> = fs::read_dir(target.model_dir("m"))
        .unwrap()
        .flatten()
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[test]
fn a_move_skips_files_already_in_place() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let entry = entry(&server, "m", &body);
    let source = ModelStore::new(dir.path().join("old"));
    let target_dir = dir.path().join("new");
    fs::create_dir(&target_dir).unwrap();
    let target = ModelStore::in_folder(&target_dir);
    installed(&source, &entry, &body);
    installed(&target, &entry, &body);

    source
        .move_to(
            &target,
            std::slice::from_ref(&entry),
            &no_cancel(),
            &mut |_| {},
            commit_ok(),
        )
        .unwrap();

    assert_eq!(target.status(&entry), ModelStatus::Installed);
    assert_eq!(source.status(&entry), ModelStatus::NotInstalled);
}

#[test]
fn moving_to_the_same_folder_by_another_name_deletes_nothing() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let entry = entry(&server, "m", &body);
    let real = dir.path().join("models");
    fs::create_dir(&real).unwrap();
    let source = ModelStore::in_folder(&real);
    installed(&source, &entry, &body);
    let dotted = real.join("..").join("models");

    let error = source
        .move_to(
            &ModelStore::in_folder(&dotted),
            std::slice::from_ref(&entry),
            &no_cancel(),
            &mut |_| {},
            commit_ok(),
        )
        .unwrap_err();

    assert!(matches!(error, DownloadError::SameFolder));
    assert_eq!(source.status(&entry), ModelStatus::Installed);
}

#[test]
fn moving_into_a_models_own_folder_deletes_nothing() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let entry = entry(&server, "m", &body);
    let source = ModelStore::new(dir.path().join("old"));
    installed(&source, &entry, &body);
    let inside = source.model_dir("m").join("sub");
    fs::create_dir(&inside).unwrap();

    let error = source
        .move_to(
            &ModelStore::in_folder(&inside),
            std::slice::from_ref(&entry),
            &no_cancel(),
            &mut |_| {},
            commit_ok(),
        )
        .unwrap_err();

    assert!(matches!(error, DownloadError::SameFolder));
    assert_eq!(source.status(&entry), ModelStatus::Installed);
}

#[test]
fn moving_to_a_missing_folder_fails_and_keeps_the_originals() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let entry = entry(&server, "m", &body);
    let source = ModelStore::new(dir.path().join("old"));
    installed(&source, &entry, &body);
    let gone = dir.path().join("gone");

    let error = source
        .move_to(
            &ModelStore::in_folder(&gone),
            std::slice::from_ref(&entry),
            &no_cancel(),
            &mut |_| {},
            commit_ok(),
        )
        .unwrap_err();

    assert!(matches!(error, DownloadError::FolderUnavailable(path) if path == gone));
    assert!(!Path::new(&gone).exists());
    assert_eq!(source.status(&entry), ModelStatus::Installed);
}

#[test]
fn deleting_a_model_leaves_the_users_other_files_alone() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let entry = entry(&server, "m", &body);
    let store = ModelStore::in_folder(dir.path());
    installed(&store, &entry, &body);
    let photo = dir.path().join("holiday.jpg");
    fs::write(&photo, "precious").unwrap();

    store.delete(&entry).unwrap();

    assert_eq!(fs::read(&photo).unwrap(), b"precious");
}

#[test]
fn preparing_creates_the_apps_own_folder_but_never_a_chosen_one() {
    let dir = tempfile::tempdir().unwrap();
    let own = dir.path().join("data/models");
    ModelStore::new(&own).prepare().unwrap();
    assert!(own.is_dir());

    let gone = dir.path().join("Volumes/SSD");
    let error = ModelStore::in_folder(&gone).prepare().unwrap_err();
    assert!(matches!(error, DownloadError::FolderUnavailable(_)));
    assert!(!dir.path().join("Volumes").exists());
}

#[cfg(unix)]
#[test]
fn moving_onto_a_link_to_the_same_files_deletes_nothing() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let entry = entry(&server, "m", &body);
    let source = ModelStore::new(dir.path().join("old"));
    installed(&source, &entry, &body);
    let target_dir = dir.path().join("new");
    fs::create_dir(&target_dir).unwrap();
    let target = ModelStore::in_folder(&target_dir);
    std::os::unix::fs::symlink(source.model_dir("m"), target.model_dir("m")).unwrap();

    let error = source
        .move_to(
            &target,
            std::slice::from_ref(&entry),
            &no_cancel(),
            &mut |_| {},
            commit_ok(),
        )
        .unwrap_err();

    assert!(matches!(error, DownloadError::SameFolder));
    assert_eq!(source.status(&entry), ModelStatus::Installed);
}

#[test]
fn a_damaged_file_in_the_target_is_replaced_not_trusted() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let entry = entry(&server, "m", &body);
    let source = ModelStore::new(dir.path().join("old"));
    let target_dir = dir.path().join("new");
    fs::create_dir(&target_dir).unwrap();
    let target = ModelStore::in_folder(&target_dir);
    installed(&source, &entry, &body);
    installed(&target, &entry, &vec![0; body.len()]);

    source
        .move_to(
            &target,
            std::slice::from_ref(&entry),
            &no_cancel(),
            &mut |_| {},
            commit_ok(),
        )
        .unwrap();

    assert!(target.verify(&entry).unwrap(), "the good copy must survive");
}

#[cfg(unix)]
#[test]
fn originals_that_wont_delete_dont_undo_a_finished_move() {
    use std::os::unix::fs::PermissionsExt;

    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let stuck = entry(&server, "stuck", &body);
    let free = entry(&server, "free", &body);
    let source = ModelStore::new(dir.path().join("old"));
    let target_dir = dir.path().join("new");
    fs::create_dir(&target_dir).unwrap();
    let target = ModelStore::in_folder(&target_dir);
    installed(&source, &stuck, &body);
    installed(&source, &free, &body);
    let locked = source.model_dir("stuck");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

    let result = source.move_to(
        &target,
        &[stuck.clone(), free.clone()],
        &no_cancel(),
        &mut |_| {},
        commit_ok(),
    );
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    let error = result.unwrap_err();
    assert!(
        matches!(&error, DownloadError::OriginalsLeft(path) if path == source.root()),
        "{error}"
    );
    assert_eq!(target.status(&stuck), ModelStatus::Installed);
    assert_eq!(target.status(&free), ModelStatus::Installed);
    assert_eq!(
        source.status(&free),
        ModelStatus::NotInstalled,
        "one failure doesn't stop the rest"
    );
}

#[test]
fn cancelling_a_move_with_nothing_left_to_copy_still_cancels() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let body = payload(1000);
    let entry = entry(&server, "m", &body);
    let source = ModelStore::new(dir.path().join("old"));
    let target_dir = dir.path().join("new");
    fs::create_dir(&target_dir).unwrap();
    let target = ModelStore::in_folder(&target_dir);
    installed(&source, &entry, &body);
    installed(&target, &entry, &body);
    let mut committed = false;

    let error = source
        .move_to(
            &target,
            std::slice::from_ref(&entry),
            &AtomicBool::new(true),
            &mut |_| {},
            || {
                committed = true;
                Ok(())
            },
        )
        .unwrap_err();

    assert!(matches!(error, DownloadError::Cancelled));
    assert!(!committed);
    assert_eq!(source.status(&entry), ModelStatus::Installed);
}
