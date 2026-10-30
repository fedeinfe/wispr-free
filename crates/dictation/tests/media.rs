//! Media while dictating: the flow quiets the system for as long as the mic is open, and every way
//! a dictation can end puts back what it changed and nothing else.

mod support;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use support::{
    FN, FakeMedia, MEDIA_NOTE, MediaCall, PARAKEET, Rig, SILERO, SPEAKERS, base_settings, fixture,
    install_models, wait_for,
};
use wf_audio::CaptureEvent;
use wf_core::dictation::PillInput;
use wf_core::pill::PillState;
use wf_core::settings::{MediaWhileDictating, Settings};
use wf_core::shortcut::Key;
use wf_models::Catalog;
use wf_platform::media::MediaError;

const HOLD: Duration = Duration::from_millis(800);
const PAST_TAP: Duration = Duration::from_millis(600);
/// Long enough that the press can't turn out to be a tap or a stray key.
const PAUSE_DELAY: Duration = Duration::from_secs(1);
const CLOSE_LIMIT: Duration = Duration::from_secs(5);
const MUSIC: &str = "com.apple.Music";

fn with(media: MediaWhileDictating) -> Settings {
    let mut settings = base_settings();
    settings.media.while_dictating = media;
    settings
}

fn muting() -> Rig {
    let rig = Rig::new(with(MediaWhileDictating::Mute));
    rig.probe.hears("hello there");
    rig
}

fn pausing() -> Rig {
    let rig = Rig::new(with(MediaWhileDictating::Pause));
    rig.probe.hears("hello there");
    rig.media.playing.lock().unwrap().push(MUSIC.to_owned());
    rig
}

fn note_exists(rig: &Rig) -> bool {
    rig.data.path().join(MEDIA_NOTE).exists()
}

fn muted_and_back() -> Vec<MediaCall> {
    vec![
        MediaCall::Mute(SPEAKERS.to_owned()),
        MediaCall::Unmute(SPEAKERS.to_owned()),
    ]
}

/// Holds the shortcut until the output has gone quiet.
fn start_recording(rig: &Rig) {
    rig.press(FN);
    rig.mic.wait_open();
    wait_for("the mute", || rig.media.is_muted(SPEAKERS));
}

#[test]
fn leave_media_alone_touches_nothing_at_all() {
    let rig = Rig::new(with(MediaWhileDictating::Nothing));
    rig.probe.hears("hello there");
    rig.media.playing.lock().unwrap().push(MUSIC.to_owned());

    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();
    rig.settle();

    assert_eq!(rig.media.calls(), []);
    assert!(!note_exists(&rig));
}

#[test]
fn the_output_is_muted_while_the_mic_is_open_and_back_when_it_closes() {
    let rig = muting();

    start_recording(&rig);
    assert_eq!(rig.media.calls(), [MediaCall::Mute(SPEAKERS.to_owned())]);

    rig.mic.play(&fixture("speech_en"));
    rig.clock.advance(HOLD);
    rig.release(FN);
    rig.wait_inserted();

    wait_for("the unmute", || !rig.media.is_muted(SPEAKERS));
    assert_eq!(rig.media.calls(), muted_and_back());
}

#[test]
fn an_output_the_user_muted_stays_muted() {
    let rig = muting();
    rig.media.muted.lock().unwrap().push(SPEAKERS.to_owned());

    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();
    rig.settle();

    assert_eq!(rig.media.calls(), []);
    assert!(rig.media.is_muted(SPEAKERS));
    assert!(!note_exists(&rig));
}

#[test]
fn a_cancel_puts_the_sound_back() {
    let rig = muting();

    start_recording(&rig);
    rig.clock.advance(HOLD);
    rig.press(Key::Esc);

    rig.media.wait_calls(2);
    assert_eq!(rig.media.calls(), muted_and_back());
}

#[test]
fn a_tap_too_short_to_be_a_dictation_puts_the_sound_back() {
    let rig = muting();

    start_recording(&rig);
    rig.clock.advance(Duration::from_millis(100));
    rig.release(FN);
    rig.hold(PAST_TAP);

    rig.media.wait_calls(2);
    assert_eq!(rig.media.calls(), muted_and_back());
}

#[test]
fn a_mic_that_dies_puts_the_sound_back() {
    let rig = muting();

    start_recording(&rig);
    rig.mic.send(CaptureEvent::Device(None));

    assert!(rig.wait_error().contains("No microphone"));
    rig.media.wait_calls(2);
    assert_eq!(rig.media.calls(), muted_and_back());
}

#[test]
fn models_that_fail_to_load_put_the_sound_back() {
    let rig = muting();
    rig.probe
        .fail_asr_load
        .store(true, std::sync::atomic::Ordering::SeqCst);

    rig.press(FN);
    rig.mic.wait_open();

    assert!(rig.wait_error().contains("damaged"));
    rig.media.wait_calls(2);
    assert_eq!(rig.media.calls(), muted_and_back());
}

#[test]
fn a_dictation_refused_at_key_down_changes_nothing() {
    let rig = muting();
    *rig.mic.problem.lock().unwrap() = Some("Allow the microphone".to_owned());

    rig.press(FN);

    assert_eq!(rig.wait_error(), "Allow the microphone");
    rig.settle();
    assert_eq!(rig.media.calls(), []);
}

#[test]
fn hands_free_stays_muted_until_it_stops() {
    let rig = muting();

    rig.dictation.on_pill(PillInput::Click);
    rig.wait_pill(PillState::ListeningHandsFree);
    rig.mic.wait_open();
    wait_for("the mute", || rig.media.is_muted(SPEAKERS));
    rig.mic.play(&fixture("speech_en"));
    rig.hold(Duration::from_secs(30));
    rig.settle();
    assert!(rig.media.is_muted(SPEAKERS));

    rig.dictation.on_pill(PillInput::Stop);
    rig.wait_inserted();

    wait_for("the unmute", || !rig.media.is_muted(SPEAKERS));
    assert_eq!(rig.media.calls(), muted_and_back());
}

#[test]
fn a_double_tap_into_hands_free_keeps_the_mute() {
    let rig = muting();

    start_recording(&rig);
    rig.clock.advance(Duration::from_millis(100));
    rig.release(FN);
    rig.clock.advance(Duration::from_millis(100));
    rig.press(FN);
    rig.wait_pill(PillState::ListeningHandsFree);
    rig.release(FN);
    rig.hold(Duration::from_secs(3));
    rig.settle();

    assert_eq!(rig.media.calls(), [MediaCall::Mute(SPEAKERS.to_owned())]);
    rig.press(Key::Esc);
    rig.media.wait_calls(2);
}

#[test]
fn the_sound_is_back_before_the_text_is_ready() {
    let rig = muting();
    rig.probe.asr_gate.close();

    start_recording(&rig);
    rig.mic.play(&fixture("speech_en"));
    rig.clock.advance(HOLD);
    rig.release(FN);

    wait_for("the unmute", || !rig.media.is_muted(SPEAKERS));
    assert_eq!(rig.inserted(), Vec::<String>::new());
    rig.probe.asr_gate.open();
    rig.wait_inserted();
}

#[test]
fn a_second_recording_during_the_first_ones_processing_mutes_again() {
    let rig = muting();
    rig.probe.asr_gate.close();

    start_recording(&rig);
    rig.mic.play(&fixture("speech_en"));
    rig.clock.advance(HOLD);
    rig.release(FN);
    wait_for("the unmute", || !rig.media.is_muted(SPEAKERS));

    start_recording(&rig);
    rig.clock.advance(HOLD);
    rig.press(Key::Esc);
    rig.media.wait_calls(4);
    rig.probe.asr_gate.open();

    assert!(!rig.media.is_muted(SPEAKERS));
}

#[test]
fn the_mute_comes_off_the_device_it_went_on_when_the_output_changes() {
    let rig = muting();

    start_recording(&rig);
    *rig.media.output.lock().unwrap() = Ok("AirPods".to_owned());
    rig.clock.advance(HOLD);
    rig.press(Key::Esc);

    rig.media.wait_calls(2);
    assert_eq!(rig.media.calls(), muted_and_back());
}

#[test]
fn a_slow_sound_system_holds_up_neither_the_mic_nor_the_paste() {
    let rig = muting();
    rig.media.gate.close();

    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["Hello there."]);
    assert_eq!(rig.media.calls(), []);
    rig.media.gate.open();
    rig.media.wait_calls(2);
}

#[test]
fn an_output_with_no_mute_says_so_once_and_leaves_no_note() {
    let rig = muting();
    *rig.media.output.lock().unwrap() = Err(MediaError::NoMuteControl);

    rig.dictate(&fixture("speech_en"));
    assert!(rig.wait_toast().contains("can't be muted"));
    rig.wait_inserted();
    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();

    assert!(!note_exists(&rig));
    let toasts = std::iter::from_fn(|| rig.events.try_recv().ok())
        .filter(|event| matches!(event, wf_dictation::Event::Toast(_)))
        .count();
    assert_eq!(toasts, 0, "one toast is enough");
}

#[test]
fn the_note_is_on_disk_before_the_output_is_muted_and_gone_after() {
    let rig = muting();

    start_recording(&rig);
    assert_eq!(*rig.media.noted_at_mute.lock().unwrap(), [true]);
    assert!(note_exists(&rig));

    rig.clock.advance(HOLD);
    rig.press(Key::Esc);
    rig.media.wait_calls(2);
    wait_for("the note to go", || !note_exists(&rig));
}

#[test]
fn a_note_that_can_not_be_written_means_no_mute() {
    let rig = muting();
    fs::create_dir(rig.data.path().join(MEDIA_NOTE)).unwrap();

    rig.press(FN);
    rig.mic.wait_open();

    assert!(rig.wait_toast().contains("Couldn't mute"));
    assert_eq!(rig.media.calls(), []);
}

#[test]
fn quitting_mid_dictation_unmutes_and_nothing_mutes_after_that() {
    let rig = muting();

    start_recording(&rig);
    rig.dictation.close_media(CLOSE_LIMIT);

    assert_eq!(rig.media.calls(), muted_and_back());
    assert!(!note_exists(&rig));

    rig.clock.advance(HOLD);
    rig.press(Key::Esc);
    rig.release(Key::Esc);
    rig.release(FN);
    rig.settle();
    rig.press(FN);
    rig.mic.wait_open();
    rig.settle();
    assert_eq!(rig.media.calls().len(), 2);
}

#[test]
fn stopping_the_flow_mid_dictation_unmutes() {
    let rig = muting();
    let media = Arc::clone(&rig.media);

    start_recording(&rig);
    drop(rig);

    wait_for("the unmute", || !media.is_muted(SPEAKERS));
}

/// What a crash leaves: the note, and a muted device.
fn crashed_mid_dictation() -> (tempfile::TempDir, Arc<FakeMedia>, Vec<u8>) {
    let crashed = muting();
    start_recording(&crashed);
    let note = fs::read(crashed.data.path().join(MEDIA_NOTE)).unwrap();
    let data = tempfile::tempdir().unwrap();
    fs::write(data.path().join(MEDIA_NOTE), &note).unwrap();
    let media = Arc::new(FakeMedia::default());
    media.muted.lock().unwrap().push(SPEAKERS.to_owned());
    (data, media, note)
}

#[test]
fn the_next_launch_unmutes_what_a_crash_left_muted() {
    let (data, media, _) = crashed_mid_dictation();
    let models = data.path().join("models");
    fs::create_dir_all(&models).unwrap();

    let rig = Rig::with_media(with(MediaWhileDictating::Nothing), data, models, media);

    wait_for("the unmute", || !rig.media.is_muted(SPEAKERS));
    wait_for("the note to go", || !note_exists(&rig));
    assert_eq!(
        rig.media.calls(),
        [MediaCall::Unmute(SPEAKERS.to_owned())],
        "whatever the setting says now"
    );
}

#[test]
fn a_device_missing_at_launch_is_unmuted_once_it_is_back() {
    let (data, media, _) = crashed_mid_dictation();
    media.gone.lock().unwrap().push(SPEAKERS.to_owned());
    let models = data.path().join("models");
    fs::create_dir_all(&models).unwrap();
    install_models(&models, &Catalog::bundled(), &[PARAKEET, SILERO]);
    let rig = Rig::with_media(with(MediaWhileDictating::Mute), data, models, media);
    rig.probe.hears("hello there");
    *rig.media.output.lock().unwrap() = Ok("AirPods".to_owned());
    rig.settle();
    assert!(note_exists(&rig), "still owed");

    rig.media.gone.lock().unwrap().clear();
    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();

    wait_for("both unmuted", || {
        !rig.media.is_muted(SPEAKERS) && !rig.media.is_muted("AirPods")
    });
    wait_for("the note to go", || !note_exists(&rig));
}

#[test]
fn a_damaged_note_is_dropped() {
    let data = tempfile::tempdir().unwrap();
    fs::write(data.path().join(MEDIA_NOTE), "not json").unwrap();
    let models = data.path().join("models");
    fs::create_dir_all(&models).unwrap();

    let rig = Rig::with_models(with(MediaWhileDictating::Nothing), data, models);

    wait_for("the note to go", || !note_exists(&rig));
    assert_eq!(rig.media.calls(), []);
}

#[test]
fn pause_waits_until_the_press_can_no_longer_be_a_tap() {
    let rig = pausing();

    rig.press(FN);
    rig.mic.wait_open();
    rig.hold(Duration::from_millis(900));
    rig.settle();
    assert_eq!(rig.media.calls(), []);

    rig.hold(Duration::from_millis(100));

    rig.media.wait_calls(1);
    assert_eq!(rig.media.calls(), [MediaCall::Pause]);
}

#[test]
fn a_tap_never_reaches_a_player() {
    let rig = pausing();

    rig.press(FN);
    rig.mic.wait_open();
    rig.clock.advance(Duration::from_millis(400));
    rig.release(FN);
    rig.hold(PAST_TAP);
    rig.wait_pill(PillState::Resting);
    rig.hold(Duration::from_secs(2));
    rig.settle();

    assert_eq!(rig.media.calls(), []);
}

#[test]
fn a_stray_key_never_reaches_a_player() {
    let rig = pausing();

    rig.press(FN);
    rig.mic.wait_open();
    rig.clock.advance(Duration::from_millis(300));
    rig.press(Key::Char('A'));
    rig.wait_toast();
    rig.hold(Duration::from_secs(2));
    rig.settle();

    assert_eq!(rig.media.calls(), []);
}

#[test]
fn what_was_paused_is_resumed_and_only_that() {
    let rig = pausing();

    rig.press(FN);
    rig.mic.wait_open();
    rig.hold(PAUSE_DELAY);
    rig.media.wait_calls(1);
    rig.mic.play(&fixture("speech_en"));
    rig.release(FN);
    rig.wait_inserted();

    rig.media.wait_calls(2);
    assert_eq!(
        rig.media.calls(),
        [MediaCall::Pause, MediaCall::Resume(vec![MUSIC.to_owned()])]
    );
}

#[test]
fn nothing_playing_means_nothing_to_resume() {
    let rig = pausing();
    rig.media.playing.lock().unwrap().clear();

    rig.press(FN);
    rig.mic.wait_open();
    rig.hold(PAUSE_DELAY);
    rig.media.wait_calls(1);
    rig.mic.play(&fixture("speech_en"));
    rig.release(FN);
    rig.wait_inserted();
    rig.settle();

    assert_eq!(rig.media.calls(), [MediaCall::Pause]);
}

#[test]
fn a_cancel_resumes_what_was_paused() {
    let rig = pausing();

    rig.press(FN);
    rig.mic.wait_open();
    rig.hold(PAUSE_DELAY);
    rig.media.wait_calls(1);
    rig.press(Key::Esc);

    rig.media.wait_calls(2);
    assert_eq!(rig.media.calls()[1], MediaCall::Resume(vec![MUSIC.to_owned()]));
}

#[test]
fn quitting_resumes_what_was_paused() {
    let rig = pausing();

    rig.press(FN);
    rig.mic.wait_open();
    rig.hold(PAUSE_DELAY);
    rig.media.wait_calls(1);
    rig.dictation.close_media(CLOSE_LIMIT);

    assert_eq!(rig.media.calls()[1], MediaCall::Resume(vec![MUSIC.to_owned()]));
}

#[test]
fn a_crash_while_paused_starts_nothing_at_the_next_launch() {
    let rig = pausing();

    rig.press(FN);
    rig.mic.wait_open();
    rig.hold(PAUSE_DELAY);
    rig.media.wait_calls(1);

    assert!(!note_exists(&rig), "music coming on by itself would be a surprise");
}

#[test]
fn the_setting_applies_without_a_restart() {
    let rig = Rig::new(with(MediaWhileDictating::Nothing));
    rig.probe.hears("hello there");
    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();
    assert_eq!(rig.media.calls(), []);

    rig.dictation
        .settings_changed(with(MediaWhileDictating::Mute));
    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();
    rig.media.wait_calls(2);
    assert_eq!(rig.media.calls(), muted_and_back());

    rig.dictation
        .settings_changed(with(MediaWhileDictating::Nothing));
    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();
    rig.settle();
    assert_eq!(rig.media.calls(), muted_and_back());
}

#[test]
fn turning_the_setting_off_mid_dictation_still_puts_the_sound_back() {
    let rig = muting();

    start_recording(&rig);
    rig.dictation
        .settings_changed(with(MediaWhileDictating::Nothing));
    rig.settle();
    assert!(rig.media.is_muted(SPEAKERS), "it finishes as it started");

    rig.clock.advance(HOLD);
    rig.press(Key::Esc);

    wait_for("the unmute", || !rig.media.is_muted(SPEAKERS));
}

#[test]
fn choosing_pause_asks_the_os_right_then_rather_than_mid_dictation() {
    let rig = Rig::new(with(MediaWhileDictating::Nothing));

    rig.dictation
        .settings_changed(with(MediaWhileDictating::Pause));
    rig.media.wait_calls(1);
    rig.dictation
        .settings_changed(with(MediaWhileDictating::Pause));
    rig.settle();

    assert_eq!(rig.media.calls(), [MediaCall::Ask]);
}

#[test]
fn a_meeting_is_never_muted_for_a_dictation() {
    let rig = muting();
    rig.dictation.toggle_meeting();
    rig.wait_event("the meeting", |event| {
        (event == wf_dictation::Event::Meeting(true)).then_some(())
    });
    rig.settle();

    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();
    rig.settle();

    assert_eq!(rig.media.calls(), []);
    rig.dictation.toggle_meeting();
}
