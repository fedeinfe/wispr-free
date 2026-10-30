//! The whole flow on fakes, from audio frames to the insertion seam: the state machine, the real
//! VAD chunking, rule cleanup, LLM guards and local passes all run. Only the engines' inference,
//! the mic and the OS are fake.

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use support::{
    FN, PARAKEET, QWEN, Rig, SILERO, WHISPER, base_settings, editor_context, fixture, wait_for,
};
use wf_audio::{CaptureEvent, InputDevice};
use wf_core::audio::{AudioClip, FRAME_SAMPLES};
use wf_core::dictation::PillInput;
use wf_core::pill::PillState;
use wf_core::settings::{CleanupStrength, HistoryRetention, KeepModels, PipelineTier};
use wf_core::shortcut::{Combo, Key};
use wf_dictation::plan::AsrModel;
use wf_dictation::{Event, TrialPart};
use wf_platform::context::AppIdentity;

const HOLD: Duration = Duration::from_millis(800);

fn speech() -> AudioClip {
    fixture("speech_en")
}

fn with_llm() -> wf_core::settings::Settings {
    let mut settings = base_settings();
    settings.pipeline.llm_cleanup = true;
    settings.pipeline.llm_model = Some(QWEN.to_owned());
    settings
}

#[test]
fn the_filler_case_of_the_spec_comes_out_clean_with_the_llm_off() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("um so I I think we should uh ship it");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["So I think we should ship it."]);
    assert!(rig.probe.heard_samples() > 0, "real audio reached ASR");
}

#[test]
fn a_spoken_list_is_written_as_one_without_asking_the_llm() {
    let rig = Rig::new(with_llm());
    rig.probe.hears(
        "Risolvi questi quattro bug primo non funziona il login, secondo non funziona il bottone, \
         terzo non carica",
    );
    rig.probe.llm_says("the model must not get this one");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(
        rig.inserted(),
        [
            "Risolvi questi quattro bug:\n1. non funziona il login\n2. non funziona il bottone\n3. non carica"
        ]
    );
    assert!(rig.probe.llm_prompts.lock().unwrap().is_empty());
}

#[test]
fn a_list_the_llm_leaves_flat_is_still_written_as_one() {
    let rig = Rig::new(with_llm());
    rig.probe
        .hears("oggi ehm devo fare tre cose cucinare, pulire, stirare");
    rig.probe
        .llm_says("Oggi devo fare queste tre cose: cucinare, pulire, stirare.");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(
        rig.inserted(),
        ["Oggi devo fare queste tre cose:\n- Cucinare\n- Pulire\n- Stirare"]
    );
}

#[test]
fn a_terminal_gets_a_list_on_one_line() {
    let rig = Rig::new(base_settings());
    rig.context.0.lock().unwrap().app = Some(AppIdentity {
        id: "com.apple.Terminal".to_owned(),
        name: None,
        pid: None,
    });
    rig.probe
        .hears("Controlla primo il login, secondo il bottone, terzo la home.");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(
        rig.inserted(),
        ["Controlla primo il login, secondo il bottone, terzo la home."]
    );
}

#[test]
fn the_self_correction_case_of_the_spec_comes_out_of_the_llm() {
    let rig = Rig::new(with_llm());
    rig.probe
        .hears("I like the red one, oh no, I meant the blue one");
    rig.probe.llm_says("I like the blue one.");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["I like the blue one."]);
}

#[test]
fn the_pill_follows_a_dictation_from_key_down_to_insertion() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("hello there");

    rig.press(FN);
    rig.wait_pill(PillState::ListeningPtt);
    rig.mic.wait_open();
    rig.mic.play(&speech());
    rig.clock.advance(HOLD);
    rig.release(FN);
    rig.wait_pill(PillState::Processing);
    rig.wait_pill(PillState::Resting);
}

#[test]
fn the_mic_opens_on_key_down_and_closes_when_the_dictation_ends() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("hello");
    assert_eq!(rig.mic.opened.load(Ordering::SeqCst), 0, "off while idle");

    rig.dictate(&speech());
    assert_eq!(rig.mic.opened.load(Ordering::SeqCst), 1);
    rig.wait_inserted();

    rig.mic.wait_closed();
}

#[test]
fn audio_from_the_first_frame_on_reaches_asr() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("hello");
    let clip = AudioClip::new(vec![0.5; FRAME_SAMPLES * 25]);

    rig.dictate(&clip);
    rig.wait_inserted();

    let clips = rig.probe.clips.lock().unwrap();
    assert_eq!(clips[0].samples().len(), clip.samples().len());
}

#[test]
fn nothing_loads_before_the_first_dictation() {
    let rig = Rig::new(with_llm());
    rig.settle();
    assert_eq!(rig.probe.total_loads(), 0);

    rig.press(FN);
    rig.mic.wait_open();

    wait_for("all three engines", || rig.probe.total_loads() == 3);
    rig.clock.advance(HOLD);
    rig.release(FN);
}

#[test]
fn a_second_dictation_reuses_the_loaded_engines() {
    let rig = Rig::new(with_llm());
    rig.probe.hears("hello");
    rig.probe.llm_says("Hello.");

    rig.dictate(&speech());
    rig.wait_inserted();
    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.probe.total_loads(), 3);
}

#[test]
fn unload_after_use_ends_every_engine_when_the_dictation_is_over() {
    let mut settings = with_llm();
    settings.pipeline.keep_models_loaded = KeepModels::UnloadAfterUse;
    let rig = Rig::new(settings);
    rig.probe.hears("hello");
    rig.probe.llm_says("Hello.");

    rig.dictate(&speech());
    rig.wait_event("the unload", |event| {
        (event == Event::Unloaded).then_some(())
    });

    wait_for("every engine to end", || rig.probe.total_alive() == 0);
}

#[test]
fn five_minutes_idle_unloads_and_a_new_dictation_resets_the_wait() {
    let mut settings = base_settings();
    settings.pipeline.keep_models_loaded = KeepModels::FiveMinutes;
    let rig = Rig::new(settings);
    rig.probe.hears("hello");

    rig.dictate(&speech());
    rig.wait_pill(PillState::Resting);
    rig.hold(Duration::from_secs(299));
    rig.settle();
    assert_eq!(rig.probe.total_alive(), 2);

    rig.hold(Duration::from_secs(2));
    wait_for("the unload", || rig.probe.total_alive() == 0);
}

#[test]
fn always_keeps_the_engines_however_long_the_app_idles() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("hello");

    rig.dictate(&speech());
    rig.wait_inserted();
    rig.hold(Duration::from_secs(24 * 3600));
    rig.settle();

    assert_eq!(rig.probe.total_alive(), 2);
}

#[test]
fn a_changed_keep_setting_applies_without_a_restart() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("hello");
    rig.dictate(&speech());
    rig.wait_pill(PillState::Resting);

    let mut settings = base_settings();
    settings.pipeline.keep_models_loaded = KeepModels::UnloadAfterUse;
    rig.dictation.settings_changed(settings);

    wait_for("the unload", || rig.probe.total_alive() == 0);
}

#[test]
fn cancel_while_recording_inserts_nothing_and_closes_the_mic() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("do not type this");

    rig.press(FN);
    rig.mic.wait_open();
    rig.mic.play(&speech());
    assert!(rig.press(Key::Esc), "Esc stays away from the focused app");
    rig.wait_pill(PillState::Resting);

    rig.mic.wait_closed();
    rig.settle();
    assert_eq!(rig.inserted(), Vec::<String>::new());
}

#[test]
fn cancel_while_processing_stops_the_engine_and_drops_the_result() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("do not type this");
    rig.probe.asr_gate.close();

    rig.dictate(&speech());
    rig.wait_pill(PillState::Processing);
    wait_for("ASR to start", || {
        rig.probe.asr_gate.entered.load(Ordering::SeqCst)
    });
    rig.clock.advance(HOLD);
    assert!(rig.press(Key::Esc));

    rig.wait_pill(PillState::Resting);
    assert_eq!(rig.probe.interrupts.load(Ordering::SeqCst), 1);
    rig.settle();
    assert_eq!(rig.inserted(), Vec::<String>::new());
}

#[test]
fn cancel_during_llm_cleanup_stops_it_and_inserts_nothing() {
    let rig = Rig::new(with_llm());
    rig.probe.hears("do not type this");
    rig.probe.llm_says("Do not type this.");
    rig.probe.llm_gate.close();

    rig.dictate(&speech());
    wait_for("the LLM to start", || {
        rig.probe.llm_gate.entered.load(Ordering::SeqCst)
    });
    rig.press(Key::Esc);

    rig.wait_pill(PillState::Resting);
    rig.settle();
    assert_eq!(rig.inserted(), Vec::<String>::new());
}

#[test]
fn after_a_cancel_the_next_dictation_works() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("second try");

    rig.press(FN);
    rig.mic.wait_open();
    rig.press(Key::Esc);
    rig.release(Key::Esc);
    rig.release(FN);
    rig.mic.wait_closed();

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["Second try."]);
}

#[test]
fn a_short_tap_is_discarded_after_the_tap_window() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("too short");

    rig.press(FN);
    rig.mic.wait_open();
    rig.clock.advance(Duration::from_millis(100));
    rig.release(FN);
    rig.hold(Duration::from_millis(600));

    rig.wait_pill(PillState::Resting);
    rig.mic.wait_closed();
    assert_eq!(rig.inserted(), Vec::<String>::new());
}

#[test]
fn a_stray_key_discards_with_a_hint() {
    let rig = Rig::new(base_settings());

    rig.press(FN);
    rig.mic.wait_open();
    rig.press(Key::Char('A'));

    assert!(rig.wait_toast().contains("another key"));
    rig.mic.wait_closed();
}

#[test]
fn double_tap_locks_and_the_next_press_stops_and_inserts() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("hands free");
    let tap = Duration::from_millis(100);

    rig.press(FN);
    rig.mic.wait_open();
    rig.clock.advance(tap);
    rig.release(FN);
    rig.clock.advance(tap);
    rig.press(FN);
    rig.wait_pill(PillState::ListeningHandsFree);
    rig.clock.advance(tap);
    rig.release(FN);

    rig.mic.play(&speech());
    rig.clock.advance(Duration::from_secs(3));
    rig.press(FN);
    rig.release(FN);
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["Hands free."]);
}

#[test]
fn hands_free_stops_itself_after_the_silence_setting() {
    let mut settings = base_settings();
    settings.pipeline.vad_auto_stop = true;
    settings.pipeline.vad_auto_stop_seconds = 1;
    let rig = Rig::new(settings);
    rig.probe.hears("hands free");

    rig.press(FN);
    rig.press(Key::Space);
    rig.release(Key::Space);
    rig.release(FN);
    rig.mic.wait_open();
    rig.wait_pill(PillState::ListeningHandsFree);
    rig.mic.play(&speech());
    rig.mic.play(&AudioClip::new(vec![0.0; 16_000 * 2]));

    rig.wait_inserted();
    assert_eq!(rig.inserted(), ["Hands free."]);
}

#[test]
fn push_to_talk_ignores_the_silence_setting() {
    let mut settings = base_settings();
    settings.pipeline.vad_auto_stop = true;
    settings.pipeline.vad_auto_stop_seconds = 1;
    let rig = Rig::new(settings);
    rig.probe.hears("still going");

    rig.press(FN);
    rig.mic.wait_open();
    rig.mic.play(&speech());
    rig.mic.play(&AudioClip::new(vec![0.0; 16_000 * 2]));
    rig.settle();
    assert_eq!(rig.inserted(), Vec::<String>::new());

    rig.clock.advance(HOLD);
    rig.release(FN);
    rig.wait_inserted();
}

#[test]
fn the_pill_toggles_hands_free_and_its_buttons_stop_and_cancel() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("clicked");

    rig.dictation.on_pill(PillInput::Click);
    rig.wait_pill(PillState::ListeningHandsFree);
    rig.mic.wait_open();
    rig.mic.play(&speech());
    rig.dictation.on_pill(PillInput::Stop);
    rig.wait_inserted();
    assert_eq!(rig.inserted(), ["Clicked."]);

    rig.dictation.on_pill(PillInput::Click);
    rig.wait_pill(PillState::ListeningHandsFree);
    rig.dictation.on_pill(PillInput::Cancel);
    rig.wait_pill(PillState::Resting);
    assert_eq!(rig.inserted().len(), 1);
}

#[test]
fn a_dictation_can_start_while_the_last_one_is_processing() {
    let rig = Rig::new(base_settings());
    rig.probe
        .transcripts
        .lock()
        .unwrap()
        .extend(["first one".to_owned(), "second one".to_owned()]);
    rig.probe.asr_gate.close();

    rig.dictate(&speech());
    wait_for("ASR to start", || {
        rig.probe.asr_gate.entered.load(Ordering::SeqCst)
    });
    rig.mic.wait_closed();

    rig.press(FN);
    rig.wait_pill(PillState::ListeningPtt);
    rig.mic.wait_open();
    rig.mic.play(&speech());
    rig.clock.advance(HOLD);
    rig.release(FN);
    rig.probe.asr_gate.open();

    wait_for("both insertions", || rig.inserted().len() == 2);
    assert_eq!(rig.inserted(), ["First one.", "Second one."]);
}

#[test]
fn a_missing_model_is_an_error_before_the_mic_opens() {
    let rig = Rig::build(base_settings(), &[SILERO]);

    rig.press(FN);

    assert!(rig.wait_error().contains("isn't downloaded"));
    assert_eq!(rig.mic.opened.load(Ordering::SeqCst), 0);
    rig.clock.advance(HOLD);
    rig.release(FN);
    rig.settle();
    assert_eq!(rig.inserted(), Vec::<String>::new());
}

#[test]
fn an_unplugged_models_folder_is_an_error_and_never_gets_recreated() {
    let mut settings = base_settings();
    let gone = std::env::temp_dir().join("wf-dictation-no-such-drive");
    settings.pipeline.models_folder = Some(gone.clone());
    let rig = Rig::new(settings);

    rig.press(FN);

    assert!(rig.wait_error().contains("not connected"));
    assert!(!gone.exists());
}

#[test]
fn a_new_models_folder_applies_without_a_restart() {
    let rig = Rig::build(base_settings(), &[]);
    rig.probe.hears("found them");
    rig.press(FN);
    rig.wait_error();
    rig.release(FN);

    let folder = rig.data.path().join("elsewhere");
    std::fs::create_dir_all(&folder).unwrap();
    let catalog = wf_models::Catalog::bundled();
    support::install_models(&folder, &catalog, &[PARAKEET, SILERO]);
    let mut settings = base_settings();
    settings.pipeline.models_folder = Some(folder);
    rig.dictation.settings_changed(settings);
    rig.settle();

    rig.dictate(&speech());
    rig.wait_inserted();
    assert_eq!(rig.inserted(), ["Found them."]);
}

#[test]
fn a_model_that_fails_to_load_is_an_error() {
    let rig = Rig::new(base_settings());
    rig.probe.fail_asr_load.store(true, Ordering::SeqCst);

    rig.press(FN);

    assert!(rig.wait_error().contains("damaged"));
    rig.mic.wait_closed();
}

#[test]
fn the_error_clears_by_itself() {
    let rig = Rig::build(base_settings(), &[]);
    rig.press(FN);
    rig.wait_error();
    rig.release(FN);

    rig.hold(Duration::from_secs(7));

    rig.wait_pill(PillState::Resting);
}

#[test]
fn a_blocked_app_shows_why_until_it_clears() {
    let rig = Rig::new(base_settings());

    rig.dictation
        .set_blocked(Some("Allow Input Monitoring".into()));
    assert_eq!(rig.wait_error(), "Allow Input Monitoring");
    rig.hold(Duration::from_secs(60));
    rig.settle();

    rig.dictation.set_blocked(None);
    rig.wait_pill(PillState::Resting);
}

#[test]
fn a_mic_the_os_refuses_is_an_error_before_it_opens() {
    let rig = Rig::new(base_settings());
    *rig.mic.problem.lock().unwrap() = Some("Allow the microphone".into());

    rig.press(FN);

    assert_eq!(rig.wait_error(), "Allow the microphone");
    assert_eq!(rig.mic.opened.load(Ordering::SeqCst), 0);
}

#[test]
fn a_mic_that_vanishes_is_an_error() {
    let rig = Rig::new(base_settings());

    rig.press(FN);
    rig.mic.wait_open();
    rig.mic.send(CaptureEvent::Device(None));

    assert!(rig.wait_error().contains("No microphone"));
}

#[test]
fn only_digital_silence_is_reported_as_a_silent_mic() {
    let rig = Rig::new(base_settings());

    rig.dictate(&AudioClip::new(vec![0.0; 16_000]));

    assert!(rig.wait_error().contains("silence"));
}

#[test]
fn a_quiet_room_is_nothing_said_and_inserts_nothing() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("invented words");

    rig.dictate(&fixture("room_tone"));
    rig.wait_pill(PillState::Processing);
    rig.wait_pill(PillState::Resting);

    assert_eq!(rig.inserted(), Vec::<String>::new());
    assert_eq!(rig.probe.heard_samples(), 0);
}

#[test]
fn whisper_always_runs_behind_the_vad() {
    let mut settings = base_settings();
    settings.pipeline.asr_model = Some(WHISPER.to_owned());
    settings.pipeline.vad = false;
    let rig = Rig::new(settings);
    rig.probe.hears("invented words");

    rig.dictate(&fixture("room_tone"));
    rig.wait_pill(PillState::Processing);
    rig.wait_pill(PillState::Resting);

    assert_eq!(rig.probe.vad_loads.load(Ordering::SeqCst), 1);
    assert_eq!(rig.probe.heard_samples(), 0);
    assert!(matches!(
        rig.probe.asr_model.lock().unwrap().as_ref(),
        Some(AsrModel::Whisper(_))
    ));
}

#[test]
fn parakeet_can_take_the_whole_recording_with_vad_off() {
    let mut settings = base_settings();
    settings.pipeline.vad = false;
    let rig = Rig::new(settings);
    rig.probe.hears("no detector");
    let clip = speech();

    rig.dictate(&clip);
    rig.wait_inserted();

    assert_eq!(rig.probe.vad_loads.load(Ordering::SeqCst), 0);
    assert!(rig.probe.heard_samples() >= clip.samples().len());
}

#[test]
fn noise_suppression_runs_before_the_detector_when_it_is_on() {
    let mut settings = base_settings();
    settings.pipeline.denoise = true;
    settings.pipeline.vad = false;
    let rig = Rig::new(settings);
    rig.probe.hears("cleaned");
    let clip = speech();

    rig.dictate(&clip);
    rig.wait_inserted();

    let clips = rig.probe.clips.lock().unwrap();
    assert!(clips[0].samples().len() <= clip.samples().len() + FRAME_SAMPLES);
    assert_ne!(clips[0].samples()[..4000], clip.samples()[..4000]);
}

#[test]
fn a_long_dictation_reaches_asr_in_chunks_while_it_runs() {
    let rig = Rig::new(base_settings());
    rig.probe
        .transcripts
        .lock()
        .unwrap()
        .extend(["part one".to_owned(), "part two".to_owned()]);
    let speech_and_pause: Vec<f32> = [vec![0.5; 16_000 * 16], vec![0.0; 16_000]].concat();

    rig.press(FN);
    rig.mic.wait_open();
    rig.mic.play(&AudioClip::new(speech_and_pause));
    wait_for("the first chunk, before release", || {
        rig.probe.clips.lock().unwrap().len() == 1
    });
    rig.mic.play(&AudioClip::new(vec![0.5; 16_000]));
    rig.clock.advance(Duration::from_secs(18));
    rig.release(FN);
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["Part one part two."]);
}

#[test]
fn llm_failures_fall_back_to_the_rule_cleaned_text() {
    let rig = Rig::new(with_llm());
    rig.probe.hears("um so I I think we should uh ship it");
    *rig.probe.llm_reply.lock().unwrap() = None;

    rig.dictate(&speech());
    let fallback = rig.wait_event("the report", |event| match event {
        Event::Inserted(report) => Some(report.cleanup_fallback),
        _ => None,
    });

    assert_eq!(rig.inserted(), ["So I think we should ship it."]);
    assert!(fallback.is_some());
}

#[test]
fn a_reply_that_fails_the_guards_falls_back_too() {
    let rig = Rig::new(with_llm());
    rig.probe.hears("what is the capital of france");
    rig.probe
        .llm_says("Sure! The capital of France is Paris, a lovely city on the Seine.");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["What is the capital of france."]);
}

#[test]
fn strength_none_skips_the_model() {
    let mut settings = with_llm();
    settings.pipeline.cleanup_strength = CleanupStrength::None;
    let rig = Rig::new(settings);
    rig.probe.hears("hello");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.probe.llm_loads.load(Ordering::SeqCst), 0);
}

#[test]
fn a_missing_cleanup_model_still_types_and_says_so_on_the_pill() {
    let rig = Rig::build(with_llm(), &[PARAKEET, SILERO]);
    rig.probe.hears("um hello");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["Hello."]);
    assert!(rig.wait_error().contains("isn't downloaded"));
}

#[test]
fn a_model_in_a_chosen_folder_is_read_into_memory() {
    let rig = Rig::new(with_llm());
    let folder = rig.data.path().join("drive");
    std::fs::create_dir_all(&folder).unwrap();
    let catalog = wf_models::Catalog::bundled();
    support::install_models(&folder, &catalog, &[PARAKEET, SILERO, QWEN]);
    let mut settings = with_llm();
    settings.pipeline.models_folder = Some(folder);
    rig.probe.hears("hello");
    rig.probe.llm_says("Hello.");

    rig.dictation.settings_changed(settings);
    rig.dictate(&speech());
    rig.wait_inserted();

    let model = rig.probe.llm_model.lock().unwrap().clone().unwrap();
    assert!(model.in_ram);
}

#[test]
fn a_model_in_the_app_folder_stays_mapped() {
    let rig = Rig::new(with_llm());
    rig.probe.hears("hello");
    rig.probe.llm_says("Hello.");

    rig.dictate(&speech());
    rig.wait_inserted();

    let model = rig.probe.llm_model.lock().unwrap().clone().unwrap();
    assert!(!model.in_ram);
}

#[test]
fn new_shortcuts_apply_without_a_restart() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("rebound");
    let mut settings = base_settings();
    settings.shortcuts.push_to_talk = Combo::new([Key::F(13)]);
    rig.dictation.settings_changed(settings);
    rig.settle();

    rig.press(FN);
    rig.settle();
    assert_eq!(rig.mic.opened.load(Ordering::SeqCst), 0);
    rig.release(FN);

    rig.press(Key::F(13));
    rig.mic.wait_open();
    rig.mic.play(&speech());
    rig.clock.advance(HOLD);
    rig.release(Key::F(13));
    rig.wait_inserted();
}

#[test]
fn paused_keys_are_left_alone_and_held_keys_are_forgotten_afterwards() {
    let rig = Rig::new(base_settings());

    rig.dictation.pause_keys(true);
    assert!(!rig.press(FN));
    rig.settle();
    assert_eq!(rig.mic.opened.load(Ordering::SeqCst), 0);

    rig.dictation.pause_keys(false);
    rig.press(FN);
    rig.mic.wait_open();
}

#[test]
fn settings_reach_the_ranking_the_language_and_the_dictionary() {
    let mut settings = base_settings();
    settings.general.microphones = vec!["usb-mic".to_owned()];
    settings.general.languages = vec!["it".to_owned()];
    settings.style.dictionary = vec!["Coxwell".to_owned()];
    let rig = Rig::new(settings);
    rig.probe.hears("ciao");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(*rig.mic.ranking.lock().unwrap(), ["usb-mic"]);
    let options = rig.probe.asr_options.lock().unwrap().clone().unwrap();
    assert_eq!(options.language.as_deref(), Some("it"));
    assert_eq!(options.bias_words, ["Coxwell"]);
}

#[test]
fn several_languages_leave_detection_to_the_engine() {
    let mut settings = base_settings();
    settings.general.languages = vec!["en".to_owned(), "it".to_owned()];
    let rig = Rig::new(settings);
    rig.probe.hears("hello");

    rig.dictate(&speech());
    rig.wait_inserted();

    let options = rig.probe.asr_options.lock().unwrap().clone().unwrap();
    assert_eq!(options.language, None);
}

#[test]
fn nearby_words_bias_asr_and_match_the_text_before_the_cursor() {
    let rig = Rig::new(base_settings());
    {
        let mut context = rig.context.0.lock().unwrap();
        context.text = Some(support::around("Meeting with Federica and "));
    }
    rig.probe.hears("Marco");

    rig.dictate(&speech());
    rig.wait_inserted();

    let options = rig.probe.asr_options.lock().unwrap().clone().unwrap();
    assert!(options.bias_words.contains(&"Federica".to_owned()));
    assert_eq!(rig.inserted(), ["marco."]);
}

#[test]
fn with_context_off_nothing_on_screen_is_used() {
    let mut settings = base_settings();
    settings.privacy.context_awareness = false;
    let rig = Rig::new(settings);
    {
        let mut context = rig.context.0.lock().unwrap();
        context.text = Some(support::around("Meeting with Federica and "));
    }
    rig.probe.hears("Marco");

    rig.dictate(&speech());
    rig.wait_inserted();

    let options = rig.probe.asr_options.lock().unwrap().clone().unwrap();
    assert_eq!(options.bias_words, Vec::<String>::new());
    assert_eq!(rig.inserted(), ["Marco."]);
}

#[test]
fn the_style_of_the_focused_app_applies() {
    let rig = Rig::new(base_settings());
    {
        let mut context = rig.context.0.lock().unwrap();
        context.app.as_mut().unwrap().id = "net.whatsapp.WhatsApp".to_owned();
    }
    rig.probe.hears("see you at noon");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["See you at noon"]);
}

#[test]
fn snippets_and_replacements_run_after_cleanup() {
    let mut settings = base_settings();
    settings.style.replacements = vec![wf_core::settings::Replacement {
        wrong: "github".into(),
        right: "GitHub".into(),
    }];
    settings.style.snippets = vec![wf_core::settings::Snippet {
        trigger: "my signature".into(),
        text: "Federico".into(),
    }];
    let rig = Rig::new(settings);
    rig.probe.hears("push to github my signature");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["Push to GitHub Federico."]);
}

#[test]
fn press_enter_presses_enter_after_the_text() {
    let mut settings = base_settings();
    settings.style.press_enter_command = true;
    let rig = Rig::new(settings);
    rig.probe.hears("send it press enter");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["Send it."]);
    assert_eq!(rig.log.enters.load(Ordering::SeqCst), 1);
}

#[test]
fn a_lone_press_enter_pastes_nothing() {
    let mut settings = base_settings();
    settings.style.press_enter_command = true;
    let rig = Rig::new(settings);
    rig.probe.hears("press enter");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.inserted(), Vec::<String>::new());
    assert_eq!(rig.log.enters.load(Ordering::SeqCst), 1);
}

#[test]
fn press_enter_is_plain_text_while_the_setting_is_off() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("send it press enter");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.log.enters.load(Ordering::SeqCst), 0);
}

#[test]
fn no_text_field_asks_before_copying_and_keeps_the_text() {
    let rig = Rig::new(base_settings());
    rig.log.no_focus.store(true, Ordering::SeqCst);
    rig.probe.hears("nowhere to go");

    rig.dictate(&speech());

    let text = rig.wait_no_text_field();
    assert_eq!(text, "Nowhere to go.");
    assert!(rig.log.copied.lock().unwrap().is_empty(), "nobody said yes");
    rig.dictation.copy_text(text);
    assert_eq!(rig.wait_toast(), "Copied");
    assert_eq!(*rig.log.copied.lock().unwrap(), ["Nowhere to go."]);
    rig.log.no_focus.store(false, Ordering::SeqCst);
    rig.dictation.paste_last();
    wait_for("paste last", || rig.inserted().len() == 2);
    assert_eq!(rig.inserted()[1], "Nowhere to go.");
}

#[test]
fn a_refused_paste_is_an_error_and_the_text_stays_available() {
    let rig = Rig::new(base_settings());
    rig.log.refused.store(true, Ordering::SeqCst);
    rig.probe.hears("hello");

    rig.dictate(&speech());

    assert!(rig.wait_error().contains("accessibility"));
}

#[test]
fn paste_last_fires_only_after_the_combo_is_released() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("again please");
    rig.dictate(&speech());
    rig.wait_inserted();

    use wf_core::shortcut::Side;
    let keys = [Key::Ctrl(Side::Left), Key::Meta(Side::Left), Key::Char('V')];
    keys.iter().for_each(|&key| {
        rig.press(key);
    });
    rig.settle();
    assert_eq!(rig.inserted().len(), 1);
    keys.iter().for_each(|&key| {
        rig.release(key);
    });

    wait_for("paste last", || rig.inserted().len() == 2);
    assert_eq!(rig.inserted()[1], "Again please.");
}

#[test]
fn copy_last_puts_the_text_on_the_clipboard() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("copy me");
    rig.dictate(&speech());
    rig.wait_inserted();

    use wf_core::shortcut::Side;
    rig.press(Key::Ctrl(Side::Left));
    rig.press(Key::Meta(Side::Left));
    rig.press(Key::Char('C'));

    wait_for("copy last", || rig.log.copied.lock().unwrap().len() == 1);
    assert_eq!(rig.log.copied.lock().unwrap()[0], "Copy me.");
}

#[test]
fn paste_last_with_no_history_says_so() {
    let rig = Rig::new(base_settings());

    rig.dictation.paste_last();

    assert!(rig.wait_toast().contains("Nothing"));
}

#[test]
fn history_feeds_recent_transcripts_and_a_menu_pick_pastes_it() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("first");
    rig.dictate(&speech());
    rig.wait_inserted();
    rig.probe.hears("second");
    rig.dictate(&speech());

    let recent = rig.wait_event("two recents", |event| match event {
        Event::Recent(recent) if recent.len() == 2 => Some(recent),
        _ => None,
    });
    assert_eq!(recent, ["Second.", "First."]);

    rig.dictation.paste_recent(1);
    wait_for("the pick", || rig.inserted().len() == 3);
    assert_eq!(rig.inserted()[2], "First.");
}

#[test]
fn never_store_writes_no_history_file_but_paste_last_still_works() {
    let mut settings = base_settings();
    settings.privacy.history_retention = HistoryRetention::Never;
    let rig = Rig::new(settings);
    rig.probe.hears("secret");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert!(!rig.data.path().join("history.sqlite3").exists());
    rig.dictation.paste_last();
    wait_for("paste last", || rig.inserted().len() == 2);
}

#[test]
fn forgetting_history_removes_the_file_and_the_last_text() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("secret");
    rig.dictate(&speech());
    rig.wait_inserted();

    rig.dictation.forget_history();
    assert!(!rig.data.path().join("history.sqlite3").exists());

    rig.dictation.paste_last();
    assert!(rig.wait_toast().contains("Nothing"));
}

#[test]
fn a_trial_reports_what_it_heard_and_inserts_nothing() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("um testing one two");

    rig.dictation.start_trial();
    rig.mic.wait_open();
    rig.mic.play(&speech());
    rig.dictation.on_pill(PillInput::Stop);

    let text = rig.wait_event("the trial", |event| match event {
        Event::Trial(text) => Some(text),
        _ => None,
    });
    assert_eq!(text, "Testing one two.");
    assert_eq!(rig.inserted(), Vec::<String>::new());
}

fn wait_trial_failure(rig: &Rig) -> (TrialPart, String) {
    rig.wait_event("a failed trial", |event| match event {
        Event::TrialFailed { part, reason } => Some((part, reason)),
        _ => None,
    })
}

fn wait_trial(rig: &Rig) -> String {
    rig.wait_event("the trial", |event| match event {
        Event::Trial(text) => Some(text),
        _ => None,
    })
}

#[test]
fn while_armed_the_shortcut_runs_a_trial_and_inserts_nothing() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("um testing one two");

    rig.dictation.arm_trial(true);
    rig.dictate(&speech());

    assert_eq!(wait_trial(&rig), "Testing one two.");
    assert_eq!(rig.inserted(), Vec::<String>::new());
}

#[test]
fn a_tap_too_short_to_count_leaves_the_trial_armed() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("testing");
    rig.dictation.arm_trial(true);

    rig.press(FN);
    rig.mic.wait_open();
    rig.release(FN);
    rig.hold(HOLD);
    rig.mic.wait_closed();
    rig.dictate(&speech());

    assert_eq!(wait_trial(&rig), "Testing.");
    assert_eq!(rig.inserted(), Vec::<String>::new());
}

#[test]
fn disarming_gives_the_shortcut_back_to_the_focused_app() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("hello there");
    rig.dictation.arm_trial(true);
    rig.dictation.arm_trial(false);

    rig.dictate(&speech());
    rig.wait_inserted();

    assert_eq!(rig.inserted(), ["Hello there."]);
}

#[test]
fn a_trial_without_its_models_blames_the_models() {
    let rig = Rig::build(base_settings(), &[SILERO]);

    rig.dictation.start_trial();

    let (part, reason) = wait_trial_failure(&rig);
    assert_eq!(part, TrialPart::Models);
    assert!(reason.contains("isn't downloaded"), "{reason}");
    assert_eq!(rig.mic.opened.load(Ordering::SeqCst), 0);
}

#[test]
fn a_trial_on_a_refused_mic_blames_the_mic() {
    let rig = Rig::new(base_settings());
    *rig.mic.problem.lock().unwrap() = Some("Allow the microphone".into());

    rig.dictation.start_trial();

    assert_eq!(
        wait_trial_failure(&rig),
        (TrialPart::Microphone, "Allow the microphone".to_owned())
    );
}

#[test]
fn a_trial_on_a_vanished_mic_blames_the_mic() {
    let rig = Rig::new(base_settings());

    rig.dictation.start_trial();
    rig.mic.wait_open();
    rig.mic.send(CaptureEvent::Device(None));

    let (part, reason) = wait_trial_failure(&rig);
    assert_eq!(part, TrialPart::Microphone);
    assert!(reason.contains("No microphone"), "{reason}");
    assert!(rig.wait_error().contains("No microphone"));
}

#[test]
fn a_trial_on_digital_silence_blames_the_mic() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("never heard");

    rig.dictation.start_trial();
    rig.mic.wait_open();
    rig.mic.play(&AudioClip::new(vec![0.0; 16_000]));
    rig.dictation.on_pill(PillInput::Stop);

    let (part, reason) = wait_trial_failure(&rig);
    assert_eq!(part, TrialPart::Microphone);
    assert!(reason.contains("silence"), "{reason}");
}

#[test]
fn a_trial_that_hears_no_words_says_so() {
    let rig = Rig::new(base_settings());
    rig.probe.hears("never heard");

    rig.dictation.start_trial();
    rig.mic.wait_open();
    rig.mic.play(&AudioClip::new(vec![0.000_1; 16_000]));
    rig.dictation.on_pill(PillInput::Stop);

    let (part, reason) = wait_trial_failure(&rig);
    assert_eq!(part, TrialPart::Microphone);
    assert!(reason.contains("No speech"), "{reason}");
}

#[test]
fn a_trial_whose_model_will_not_load_blames_the_models() {
    let rig = Rig::new(base_settings());
    rig.probe.fail_asr_load.store(true, Ordering::SeqCst);

    rig.dictation.start_trial();

    let (part, reason) = wait_trial_failure(&rig);
    assert_eq!(part, TrialPart::Models);
    assert!(reason.contains("load the models"), "{reason}");
}

#[test]
fn a_cancelled_trial_is_not_a_failure() {
    let rig = Rig::new(base_settings());

    rig.dictation.start_trial();
    rig.mic.wait_open();
    rig.dictation.on_pill(PillInput::Cancel);
    rig.mic.wait_closed();
    rig.settle();

    let events: Vec<_> = rig.events.try_iter().collect();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::TrialFailed { .. })),
        "{events:?}"
    );
}

#[test]
fn a_normal_dictation_reports_no_trial_failure() {
    let rig = Rig::build(base_settings(), &[SILERO]);

    rig.press(FN);
    rig.wait_error();

    let events: Vec<_> = rig.events.try_iter().collect();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::TrialFailed { .. })),
        "{events:?}"
    );
}

#[test]
fn the_tier_picks_the_models_when_it_is_not_custom() {
    let mut settings = base_settings();
    settings.pipeline.tier = PipelineTier::Lite;
    settings.pipeline.asr_model = None;
    let rig = Rig::new(settings);
    rig.probe.hears("hello");

    rig.dictate(&speech());
    rig.wait_inserted();

    assert!(matches!(
        rig.probe.asr_model.lock().unwrap().as_ref(),
        Some(AsrModel::Whisper(_))
    ));
    assert_eq!(*rig.context.0.lock().unwrap(), editor_context());
}

#[test]
fn levels_reach_the_ui_while_recording() {
    let rig = Rig::new(base_settings());

    rig.press(FN);
    rig.mic.wait_open();
    rig.mic.send(CaptureEvent::Device(Some(InputDevice {
        id: "mic".into(),
        name: "Mic".into(),
    })));
    rig.mic.play(&speech());

    rig.wait_event("a level", |event| {
        matches!(event, Event::Level(_)).then_some(())
    });
}
