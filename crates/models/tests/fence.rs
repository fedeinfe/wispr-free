//! A fenced store reads anywhere and changes nothing outside its fence, wherever links lead. This
//! is what keeps a dev run out of the user's real models folder.

use std::fs::{self, File};
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use wf_models::{Catalog, DownloadError, ModelEntry, ModelStatus, ModelStore};

fn model() -> ModelEntry {
    Catalog::bundled().models.remove(0)
}

fn lay_out(root: &Path, model: &ModelEntry) {
    let dir = root.join(&model.id);
    fs::create_dir_all(&dir).unwrap();
    for file in &model.files {
        let handle = File::create(dir.join(&file.name)).unwrap();
        handle.set_len(file.size).unwrap();
    }
}

fn store(root: &Path, fence: &Path) -> ModelStore {
    ModelStore::in_folder(root).fenced(Some(fence.to_owned()))
}

#[test]
fn outside_the_fence_a_model_reads_as_usual_but_stays() {
    let folder = tempfile::tempdir().unwrap();
    let (fence, outside) = (folder.path().join("run"), folder.path().join("real"));
    fs::create_dir_all(&fence).unwrap();
    let model = model();
    lay_out(&outside, &model);

    let store = store(&outside, &fence);

    assert!(!store.writable());
    assert_eq!(store.status(&model), ModelStatus::Installed);
    let refused = store.delete(&model).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(outside.join(&model.id).is_dir());
}

#[test]
fn nothing_can_be_downloaded_outside_the_fence_and_the_network_is_not_touched() {
    let folder = tempfile::tempdir().unwrap();
    let (fence, outside) = (folder.path().join("run"), folder.path().join("real"));
    fs::create_dir_all(&outside).unwrap();
    let model = model();

    let refused = store(&outside, &fence).download(&model, &AtomicBool::new(false), &mut |_| {});

    assert!(matches!(refused, Err(DownloadError::ReadOnly(_))));
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn a_move_is_refused_from_or_to_a_folder_outside_the_fence_before_anything_is_copied() {
    let folder = tempfile::tempdir().unwrap();
    let fence = folder.path().join("run");
    let (inside, outside) = (fence.join("models"), folder.path().join("real"));
    fs::create_dir_all(&inside).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let model = model();
    lay_out(&outside, &model);
    let (from, to) = (store(&outside, &fence), store(&inside, &fence));
    let cancel = AtomicBool::new(false);

    let refused = from.move_to(&to, &[model.clone()], &cancel, &mut |_| {}, || Ok(()));

    assert!(matches!(refused, Err(DownloadError::ReadOnly(_))));
    assert!(outside.join(&model.id).is_dir());
    assert_eq!(fs::read_dir(&inside).unwrap().count(), 0);

    fs::remove_dir_all(outside.join(&model.id)).unwrap();
    lay_out(&inside, &model);
    let refused = to.move_to(&from, &[model.clone()], &cancel, &mut |_| {}, || Ok(()));

    assert!(matches!(refused, Err(DownloadError::ReadOnly(_))));
    assert!(inside.join(&model.id).is_dir());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn a_link_in_the_fence_is_followed_to_where_it_leads() {
    let folder = tempfile::tempdir().unwrap();
    let (fence, outside) = (folder.path().join("run"), folder.path().join("real"));
    fs::create_dir_all(&fence).unwrap();
    let model = model();
    lay_out(&outside, &model);
    let link = fence.join("linked");
    symlink(&outside, &link).unwrap();

    let store = store(&link, &fence);

    assert!(!store.writable());
    assert!(store.delete(&model).is_err());
    assert!(outside.join(&model.id).is_dir());
}

#[test]
fn a_fence_reached_through_a_link_is_the_same_fence() {
    let folder = tempfile::tempdir().unwrap();
    let fence = folder.path().join("run");
    fs::create_dir_all(fence.join("models")).unwrap();
    let alias = folder.path().join("alias");
    symlink(&fence, &alias).unwrap();
    let model = model();
    lay_out(&fence.join("models"), &model);

    let store = store(&fence.join("models"), &alias);

    assert!(store.writable());
    store.delete(&model).unwrap();
    assert!(!fence.join("models").join(&model.id).exists());
}

#[test]
fn climbing_out_of_the_fence_with_dots_is_refused() {
    let folder = tempfile::tempdir().unwrap();
    let fence = folder.path().join("run");
    fs::create_dir_all(&fence).unwrap();

    let store = store(&fence.join("missing/../../real"), &fence);

    assert!(!store.writable());
}

#[test]
fn with_no_fence_a_store_changes_what_it_likes() {
    let folder = tempfile::tempdir().unwrap();
    let model = model();
    lay_out(folder.path(), &model);

    let store = ModelStore::in_folder(folder.path());

    assert!(store.writable());
    store.delete(&model).unwrap();
    assert!(!folder.path().join(&model.id).exists());
}
