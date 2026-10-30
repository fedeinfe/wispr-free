//! Meeting notes as the running app sees them: on, off, suggested, and what a crash leaves.

mod support;

use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use support::{FN, Rig, base_settings, wait_for};
use wf_core::audio::FRAME_SAMPLES;
use wf_core::pill::PillState;
use wf_core::shortcut::{Key, Side};
use wf_dictation::Event;
use wf_meetings::{Channel, MeetingEvent, MeetingWriter, MicClient, read_meeting};

fn zoom() -> MicClient {
    MicClient {
        pid: 4_242,
        name: "us.zoom.xos".into(),
    }
}

fn silent_frame() -> MeetingEvent {
    MeetingEvent::Frame {
        channel: Channel::Others,
        samples: vec![0.0; FRAME_SAMPLES],
    }
}

fn wait_meeting(rig: &Rig, active: bool) {
    rig.wait_event("the meeting state", |event| {
        (event == Event::Meeting(active)).then_some(())
    });
}

#[test]
fn the_shortcut_starts_and_stops_a_meeting_and_the_pill_says_so() {
    let rig = Rig::new(base_settings());
    let shortcut = || {
        rig.press(Key::Alt(Side::Left));
        rig.press(Key::Char('M'));
        rig.release(Key::Char('M'));
        rig.release(Key::Alt(Side::Left));
    };

    shortcut();
    wait_meeting(&rig, true);
    rig.wait_pill(PillState::Meeting);

    shortcut();
    wait_meeting(&rig, false);
    rig.wait_pill(PillState::Resting);
    assert_eq!(rig.room.stopped.load(Ordering::SeqCst), 1);
}

#[test]
fn a_finished_meeting_is_a_finished_file() {
    let rig = Rig::new(base_settings());

    rig.dictation.toggle_meeting();
    wait_meeting(&rig, true);
    let path = rig.room.paths.lock().unwrap()[0].clone();
    assert_eq!(path.extension().unwrap(), "recording");
    rig.dictation.toggle_meeting();
    wait_meeting(&rig, false);

    wait_for("the rename", || path.with_extension("ogg").exists());
    assert!(!path.exists());
}

#[test]
fn dictation_shows_over_a_meeting_and_gives_the_pill_back() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("hello");
    rig.dictation.toggle_meeting();
    rig.wait_pill(PillState::Meeting);

    rig.press(FN);
    rig.wait_pill(PillState::ListeningPtt);
    rig.mic.wait_open();
    rig.mic.play(&support::fixture("speech_en"));
    rig.clock.advance(Duration::from_secs(1));
    rig.release(FN);
    rig.wait_pill(PillState::Processing);

    rig.wait_pill(PillState::Meeting);
}

#[test]
fn a_meeting_that_cannot_start_says_why() {
    let rig = Rig::new(base_settings());
    rig.room.refuse.store(true, Ordering::SeqCst);

    rig.dictation.toggle_meeting();

    assert!(rig.wait_error().contains("the disk said no"));
}

#[test]
fn a_failed_recording_stops_and_says_why() {
    let rig = Rig::new(base_settings());
    rig.dictation.toggle_meeting();
    wait_meeting(&rig, true);

    rig.room.send(MeetingEvent::Failed("disk full".into()));

    assert!(rig.wait_error().contains("disk full"));
    wait_for("the stop", || rig.room.stopped.load(Ordering::SeqCst) == 1);
}

#[test]
fn silence_while_something_plays_is_reported_as_a_permission_problem() {
    let rig = Rig::new(base_settings());
    rig.room.playing.store(true, Ordering::SeqCst);
    rig.dictation.toggle_meeting();
    wait_meeting(&rig, true);

    for _ in 0..130 {
        rig.room.send(silent_frame());
    }

    assert!(rig.wait_error().contains("silent"));
}

#[test]
fn silence_with_nothing_playing_is_just_a_quiet_meeting() {
    let rig = Rig::new(base_settings());
    rig.dictation.toggle_meeting();
    wait_meeting(&rig, true);

    for _ in 0..130 {
        rig.room.send(silent_frame());
    }
    std::thread::sleep(Duration::from_millis(300));

    let errors = rig
        .events
        .try_iter()
        .filter(|event| matches!(event, Event::Pill(PillState::Error { .. })));
    assert_eq!(errors.count(), 0);
}

#[test]
fn sound_on_the_system_channel_clears_any_suspicion() {
    let rig = Rig::new(base_settings());
    rig.room.playing.store(true, Ordering::SeqCst);
    rig.dictation.toggle_meeting();
    wait_meeting(&rig, true);

    rig.room.send(MeetingEvent::Frame {
        channel: Channel::Others,
        samples: vec![0.1; FRAME_SAMPLES],
    });
    for _ in 0..130 {
        rig.room.send(silent_frame());
    }
    std::thread::sleep(Duration::from_millis(300));

    let errors = rig
        .events
        .try_iter()
        .filter(|event| matches!(event, Event::Pill(PillState::Error { .. })));
    assert_eq!(errors.count(), 0);
}

#[test]
fn nothing_is_suggested_without_the_opt_in() {
    let rig = Rig::new(base_settings());
    rig.room.clients.lock().unwrap().push(zoom());

    std::thread::sleep(Duration::from_millis(300));

    let suggested = rig
        .events
        .try_iter()
        .filter(|event| matches!(event, Event::SuggestMeeting(_)));
    assert_eq!(suggested.count(), 0);
}

#[test]
fn the_opt_in_suggests_once_and_never_starts_a_recording_by_itself() {
    let mut settings = base_settings();
    settings.meetings.auto_detect = true;
    let rig = Rig::new(settings);
    rig.room.clients.lock().unwrap().push(zoom());

    let app = rig.wait_event("the suggestion", |event| match event {
        Event::SuggestMeeting(app) => Some(app),
        _ => None,
    });

    assert_eq!(app, "Zoom");
    assert_eq!(rig.room.started.load(Ordering::SeqCst), 0);
}

#[test]
fn turning_the_opt_in_on_applies_without_a_restart() {
    let rig = Rig::new(base_settings());
    rig.room.clients.lock().unwrap().push(zoom());

    let mut settings = base_settings();
    settings.meetings.auto_detect = true;
    rig.dictation.settings_changed(settings);

    rig.wait_event("the suggestion", |event| {
        matches!(event, Event::SuggestMeeting(_)).then_some(())
    });
}

/// A recording cut off by a crash: pages on disk, and no end of stream.
fn crashed_recording(path: &Path) {
    let mut writer = MeetingWriter::create(path).unwrap();
    let frame: Vec<f32> = (0..FRAME_SAMPLES)
        .map(|i| (i as f32 * 0.05).sin() * 0.2)
        .collect();
    for _ in 0..80 {
        writer.write_frame(&frame, &frame).unwrap();
    }
    std::mem::forget(writer);
}

#[test]
fn a_recording_a_crash_left_behind_is_repaired_at_launch() {
    let data = tempfile::tempdir().unwrap();
    let meetings = data.path().join("meetings");
    fs::create_dir_all(&meetings).unwrap();
    let leftover = meetings.join("1000.recording");
    crashed_recording(&leftover);
    let models = data.path().join("models");
    fs::create_dir_all(&models).unwrap();

    let _rig = Rig::with_models(base_settings(), data, models);

    let repaired = leftover.with_extension("ogg");
    wait_for("the repair", || repaired.exists());
    assert!(!leftover.exists());
    assert!(read_meeting(&repaired).unwrap().you.duration() >= Duration::from_secs(1));
}

#[test]
fn a_file_that_is_no_recording_stays_put() {
    let data = tempfile::tempdir().unwrap();
    let meetings = data.path().join("meetings");
    fs::create_dir_all(&meetings).unwrap();
    let junk = meetings.join("1000.recording");
    fs::write(&junk, "not a meeting").unwrap();
    let models = data.path().join("models");
    fs::create_dir_all(&models).unwrap();

    let rig = Rig::with_models(base_settings(), data, models);
    rig.settle();

    assert!(junk.exists());
}
