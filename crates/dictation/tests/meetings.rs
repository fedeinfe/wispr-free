//! Meeting notes as the running app sees them: on, off, suggested, what happens to a meeting once
//! it stops, and what a crash leaves.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use support::{FN, QWEN, Rig, base_settings, wait_for};
use wf_core::audio::{AudioClip, FRAME_SAMPLES};
use wf_core::pill::PillState;
use wf_core::settings::{KeepModels, PipelineTier, Settings};
use wf_core::shortcut::{Key, Side};
use wf_dictation::plan::meeting_plan;
use wf_dictation::probe::ClipRoom;
use wf_dictation::{ChatUpdate, Event, MeetingNews, MeetingRoom, SummaryMode, WorkStep};
use wf_meetings::{
    Channel, ChatTurn, LiveEvent, Meeting, MeetingEvent, MeetingStore, MeetingWriter, MicClient,
    Stage, StageState, read_meeting,
};
use wf_models::Catalog;

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

/// With a summary model, which dictation doesn't need.
fn meeting_settings() -> Settings {
    let mut settings = base_settings();
    settings.pipeline.llm_model = Some(QWEN.to_owned());
    settings
}

fn meeting_rig(settings: Settings) -> Rig {
    let rig = Rig::new(settings);
    rig.probe.hears("we ship on friday");
    rig.probe.writes_summaries.store(true, Ordering::SeqCst);
    rig.probe.llm_says("Friday [00:01].");
    rig
}

fn speech() -> AudioClip {
    let mut samples: Vec<f32> = (0..32_000).map(|i| (i as f32 * 0.05).sin() * 0.2).collect();
    samples.extend(vec![0.0; 16_000]);
    AudioClip::new(samples)
}

fn silence() -> AudioClip {
    AudioClip::new(vec![0.0; speech().samples().len()])
}

/// Long enough to tell the language from, so the live transcript shows it at the pause.
fn long_speech() -> AudioClip {
    let mut samples: Vec<f32> = (0..144_000)
        .map(|i| (i as f32 * 0.05).sin() * 0.2)
        .collect();
    samples.extend(vec![0.0; 16_000]);
    AudioClip::new(samples)
}

/// Every toast up to the end of the work.
fn toasts_until_done(rig: &Rig) -> Vec<String> {
    let mut toasts = Vec::new();
    rig.wait_event("the work to end", |event| match event {
        Event::Toast(text) => {
            toasts.push(text);
            None
        }
        Event::Meetings(MeetingNews::WorkDone { .. }) => Some(()),
        _ => None,
    });
    toasts
}

fn start_meeting(rig: &Rig) -> String {
    rig.dictation.toggle_meeting();
    rig.wait_event("the recording", |event| match event {
        Event::Meetings(MeetingNews::Recording(id)) => Some(id),
        _ => None,
    })
}

fn wait_work_done(rig: &Rig, id: &str) -> Option<String> {
    rig.wait_event("the work to end", |event| match event {
        Event::Meetings(MeetingNews::WorkDone { id: done, problem }) if done == id => Some(problem),
        _ => None,
    })
}

/// One meeting that heard something, stopped and went through everything.
fn finished_meeting(rig: &Rig) -> String {
    let id = start_meeting(rig);
    rig.room.play(&speech(), &silence());
    rig.dictation.toggle_meeting();
    assert_eq!(wait_work_done(rig, &id), None);
    id
}

fn store(rig: &Rig) -> MeetingStore {
    MeetingStore::new(rig.data.path().join("meetings"))
}

fn meeting(rig: &Rig, id: &str) -> Meeting {
    store(rig).open(id).unwrap()
}

fn text_of(meeting: &Meeting) -> String {
    let transcript = meeting.transcript().unwrap();
    let lines = transcript.segments.iter().map(|line| line.text.as_str());
    lines.collect::<Vec<_>>().join(" ")
}

fn wait_chat(rig: &Rig, run: u64) -> Result<ChatTurn, String> {
    rig.wait_event("the answer", |event| match event {
        Event::Meetings(MeetingNews::Chat {
            run: answered,
            update: ChatUpdate::Done(answer),
        }) if answered == run => Some(answer),
        _ => None,
    })
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
fn a_meeting_ends_with_a_transcript_and_a_summary_to_open() {
    let rig = meeting_rig(meeting_settings());

    let id = start_meeting(&rig);
    let path = rig.room.paths.lock().unwrap()[0].clone();
    assert_eq!(
        path,
        rig.data
            .path()
            .join("meetings")
            .join(&id)
            .join("part-000.ogg")
    );
    assert_eq!(rig.dictation.meetings_status().recording, Some(id.clone()));
    rig.room.play(&speech(), &silence());
    rig.dictation.toggle_meeting();
    assert_eq!(wait_work_done(&rig, &id), None);

    let done = meeting(&rig, &id);
    assert!(!done.is_recording());
    assert_eq!(
        done.record().parts[0].duration_ms,
        Some(rig.room.played().as_millis() as u64)
    );
    assert!(text_of(&done).contains("we ship on friday"));
    assert_eq!(
        done.summary().unwrap().unwrap().title.as_deref(),
        Some("The launch")
    );
    assert!(done.record().stages.summary.is_done());
    assert_eq!(rig.dictation.meetings_status().work, []);
    assert_eq!(rig.dictation.meetings_status().recording, None);
}

#[test]
fn the_user_hears_when_the_notes_are_ready() {
    let rig = meeting_rig(meeting_settings());
    start_meeting(&rig);
    rig.room.play(&speech(), &silence());
    rig.dictation.toggle_meeting();
    assert_eq!(toasts_until_done(&rig), ["Meeting notes are ready"]);
}

#[test]
fn the_recording_starts_before_any_model_has_loaded() {
    let rig = meeting_rig(meeting_settings());
    let id = start_meeting(&rig);
    assert_eq!(rig.probe.total_loads(), 0);
    rig.dictation.toggle_meeting();
    wait_work_done(&rig, &id);
}

#[test]
fn the_transcript_grows_while_the_meeting_runs() {
    let rig = meeting_rig(meeting_settings());
    let id = start_meeting(&rig);

    rig.room.play(&long_speech(), &silence());
    let line = rig.wait_event("a live line", |event| match event {
        Event::Meetings(MeetingNews::Live {
            id: meeting,
            event: LiveEvent::Segment(line),
        }) if meeting == id => Some(line),
        _ => None,
    });

    assert_eq!(line.text, "we ship on friday");
    assert_eq!(line.channel, Channel::You);
    assert!(meeting(&rig, &id).is_recording());
    rig.dictation.toggle_meeting();
    wait_work_done(&rig, &id);
}

#[test]
fn both_channels_are_heard_each_on_its_own() {
    let rig = meeting_rig(meeting_settings());
    let id = start_meeting(&rig);
    rig.room.play(&speech(), &speech());
    rig.dictation.toggle_meeting();
    wait_work_done(&rig, &id);

    let transcript = meeting(&rig, &id).transcript().unwrap();
    let channels: Vec<Channel> = transcript
        .segments
        .iter()
        .map(|line| line.channel)
        .collect();
    assert!(channels.contains(&Channel::You) && channels.contains(&Channel::Others));
}

#[test]
fn work_shows_each_stage_as_it_is_reached() {
    let rig = meeting_rig(meeting_settings());
    let id = start_meeting(&rig);
    rig.room.play(&speech(), &silence());
    rig.dictation.toggle_meeting();

    let mut steps = Vec::new();
    rig.wait_event("the work to end", |event| match event {
        Event::Meetings(MeetingNews::Working { step, .. }) => {
            steps.push(step);
            None
        }
        Event::Meetings(MeetingNews::WorkDone { .. }) => Some(()),
        _ => None,
    });

    assert_eq!(steps[0], WorkStep::Waiting);
    assert_eq!(steps[1], WorkStep::Finishing);
    assert!(steps.contains(&WorkStep::Stage(Stage::Diarization)));
    assert!(steps.contains(&WorkStep::Stage(Stage::Summary)));
    assert!(meeting(&rig, &id).record().stages.summary.is_done());
}

#[test]
fn turning_keep_audio_off_leaves_no_recording_behind() {
    let mut settings = meeting_settings();
    settings.meetings.keep_audio = false;
    let rig = meeting_rig(settings);

    let id = finished_meeting(&rig);

    let done = meeting(&rig, &id);
    assert!(done.record().audio_deleted);
    assert!(!rig.room.paths.lock().unwrap()[0].exists());
    assert!(text_of(&done).contains("we ship on friday"));
}

#[test]
fn keeping_audio_is_the_default_and_keeps_it() {
    let rig = meeting_rig(meeting_settings());
    let id = finished_meeting(&rig);
    assert!(!meeting(&rig, &id).record().audio_deleted);
    assert!(rig.room.paths.lock().unwrap()[0].exists());
}

#[test]
fn with_the_live_transcript_off_the_refine_pass_makes_the_transcript() {
    let mut settings = meeting_settings();
    settings.meetings.live_transcript = false;
    settings.meetings.refine_pass = false;
    let rig = meeting_rig(settings);

    let id = start_meeting(&rig);
    rig.room.play(&speech(), &silence());
    rig.settle();
    assert_eq!(
        rig.probe.total_loads(),
        0,
        "nothing transcribes while it records"
    );
    rig.dictation.toggle_meeting();
    assert_eq!(wait_work_done(&rig, &id), None);

    let done = meeting(&rig, &id);
    assert!(text_of(&done).contains("we ship on friday"));
    assert!(done.record().stages.refine.is_done());
}

#[test]
fn the_refine_and_speaker_toggles_are_honoured() {
    let mut settings = meeting_settings();
    settings.meetings.refine_pass = false;
    settings.meetings.diarization = false;
    let rig = meeting_rig(settings);

    let id = finished_meeting(&rig);

    let stages = meeting(&rig, &id).record().stages.clone();
    assert!(matches!(stages.refine, StageState::Skipped { .. }));
    assert!(matches!(stages.diarization, StageState::Skipped { .. }));
    assert_eq!(rig.probe.diarizer_loads.load(Ordering::SeqCst), 0);
}

#[test]
fn the_speaker_engine_goes_once_its_stage_is_over() {
    let rig = meeting_rig(meeting_settings());
    let id = start_meeting(&rig);
    rig.room.play(&speech(), &speech());
    rig.dictation.toggle_meeting();
    wait_work_done(&rig, &id);

    assert_eq!(rig.probe.diarizer_loads.load(Ordering::SeqCst), 1);
    assert_eq!(rig.probe.diarizer_alive.load(Ordering::SeqCst), 0);
    assert!(meeting(&rig, &id).record().stages.diarization.is_done());
}

#[test]
fn the_summary_follows_the_template_and_my_languages() {
    let mut settings = meeting_settings();
    settings.meetings.summary_template = "Start with risks.".to_owned();
    settings.general.languages = vec!["it".to_owned()];
    let rig = meeting_rig(settings);

    finished_meeting(&rig);

    let prompts = rig.probe.llm_prompts.lock().unwrap();
    let last = prompts.last().unwrap();
    assert!(last.contains("Start with risks."), "{last}");
    assert!(
        last.contains("Italian") || last.contains("\"it\""),
        "{last}"
    );
}

#[test]
fn live_transcription_is_held_to_my_language_too() {
    let mut settings = meeting_settings();
    settings.general.languages = vec!["it".to_owned()];
    let rig = meeting_rig(settings);
    finished_meeting(&rig);
    let options = rig.probe.asr_options.lock().unwrap().clone().unwrap();
    assert_eq!(options.language.as_deref(), Some("it"));
}

#[test]
fn one_failed_stage_is_that_stage_only_and_is_said_out_loud() {
    let rig = meeting_rig(meeting_settings());
    rig.probe.fail_diarizer.store(true, Ordering::SeqCst);

    let id = start_meeting(&rig);
    rig.room.play(&speech(), &speech());
    rig.dictation.toggle_meeting();
    assert_eq!(
        toasts_until_done(&rig),
        ["Meeting notes are ready, but a step failed"]
    );

    let stages = meeting(&rig, &id).record().stages.clone();
    assert!(
        matches!(&stages.diarization, StageState::Failed { reason } if reason.contains("mixed up"))
    );
    assert!(stages.summary.is_done());
}

#[test]
fn a_missing_summary_model_fails_only_the_summary() {
    let rig = Rig::build(
        meeting_settings(),
        &[
            support::PARAKEET,
            support::SILERO,
            support::SEGMENTATION,
            support::EMBEDDING,
        ],
    );
    rig.probe.hears("we ship on friday");

    let id = finished_meeting(&rig);

    let done = meeting(&rig, &id);
    assert!(matches!(
        done.record().stages.summary,
        StageState::Failed { .. }
    ));
    assert!(text_of(&done).contains("we ship on friday"));
}

#[test]
fn a_tier_without_a_summary_model_just_skips_it() {
    let rig = meeting_rig(base_settings());
    let id = finished_meeting(&rig);
    let stages = meeting(&rig, &id).record().stages.clone();
    assert!(matches!(stages.summary, StageState::Skipped { .. }));
}

#[test]
fn what_runs_after_a_meeting_comes_from_the_tier_and_the_toggles() {
    let catalog = Catalog::bundled();
    let tiered = |tier| {
        let mut settings = Settings::default();
        settings.pipeline.tier = tier;
        settings
    };

    let lite = meeting_plan(&tiered(PipelineTier::Lite), &catalog);
    assert!(!lite.refine && !lite.diarize && !lite.summarize, "{lite:?}");
    let balanced = meeting_plan(&tiered(PipelineTier::Balanced), &catalog);
    assert!(
        balanced.diarize && balanced.summarize && balanced.keep_audio,
        "{balanced:?}"
    );

    let mut off = tiered(PipelineTier::High);
    off.meetings.refine_pass = false;
    off.meetings.diarization = false;
    off.meetings.keep_audio = false;
    off.pipeline.llm_cleanup = false;
    let off = meeting_plan(&off, &catalog);
    assert!(!off.refine && !off.diarize && !off.keep_audio, "{off:?}");
    assert!(off.summarize, "turning cleanup off leaves the summary on");
}

#[test]
fn the_maximum_length_warns_then_stops_the_recording() {
    let mut settings = meeting_settings();
    settings.meetings.max_length_minutes = 10;
    let rig = meeting_rig(settings);

    let id = start_meeting(&rig);
    rig.clock.advance(Duration::from_secs(9 * 60));
    assert_eq!(rig.wait_toast(), "One minute of meeting notes left");
    assert!(rig.dictation.meetings_status().recording.is_some());

    rig.clock.advance(Duration::from_secs(60));
    assert_eq!(rig.wait_toast(), "Meeting notes stopped at 10 minutes");
    wait_work_done(&rig, &id);
    assert_eq!(rig.dictation.meetings_status().recording, None);
    assert_eq!(rig.room.stopped.load(Ordering::SeqCst), 1);
}

#[test]
fn a_new_maximum_applies_to_the_running_recording() {
    let rig = meeting_rig(meeting_settings());
    let id = start_meeting(&rig);
    rig.clock.advance(Duration::from_secs(5 * 60));
    rig.settle();

    let mut settings = meeting_settings();
    settings.meetings.max_length_minutes = 3;
    rig.dictation.settings_changed(settings);

    assert_eq!(rig.wait_toast(), "Meeting notes stopped at 3 minutes");
    wait_work_done(&rig, &id);
}

#[test]
fn resuming_adds_a_part_and_finishing_covers_both() {
    let mut settings = meeting_settings();
    settings.meetings.refine_pass = false;
    let rig = meeting_rig(settings);
    let id = finished_meeting(&rig);
    let first = rig.room.played();

    rig.dictation.resume_meeting(&id);
    rig.wait_event("the second recording", |event| {
        matches!(event, Event::Meetings(MeetingNews::Recording(_))).then_some(())
    });
    rig.room.play(&speech(), &silence());
    rig.dictation.toggle_meeting();
    assert_eq!(wait_work_done(&rig, &id), None);

    let done = meeting(&rig, &id);
    assert_eq!(done.record().parts.len(), 2);
    assert_eq!(done.record().parts[1].offset_ms, first.as_millis() as u64);
    let lines = done.transcript().unwrap();
    assert!(lines.segments.last().unwrap().start_ms >= first.as_millis() as u64);
    assert_eq!(store(&rig).list().unwrap().len(), 1);
}

#[test]
fn only_one_meeting_records_at_a_time() {
    let rig = meeting_rig(meeting_settings());
    let id = finished_meeting(&rig);
    let running = start_meeting(&rig);

    rig.dictation.resume_meeting(&id);

    assert_eq!(rig.wait_error(), "A meeting is already recording");
    assert_eq!(
        rig.dictation.meetings_status().recording,
        Some(running.clone())
    );
    rig.dictation.toggle_meeting();
    wait_work_done(&rig, &running);
}

#[test]
fn a_recording_that_cannot_start_leaves_nothing_in_the_list() {
    let rig = meeting_rig(meeting_settings());
    rig.room.refuse.store(true, Ordering::SeqCst);
    rig.dictation.toggle_meeting();
    rig.wait_error();
    assert_eq!(store(&rig).list().unwrap(), []);
}

#[test]
fn a_resume_that_cannot_start_keeps_the_meeting() {
    let rig = meeting_rig(meeting_settings());
    let id = finished_meeting(&rig);
    rig.room.refuse.store(true, Ordering::SeqCst);

    rig.dictation.resume_meeting(&id);
    rig.wait_error();

    let kept = meeting(&rig, &id);
    assert_eq!(kept.record().parts.len(), 1);
    assert!(!kept.is_recording());
}

#[test]
fn cancel_stops_the_stage_and_only_that() {
    let rig = meeting_rig(meeting_settings());
    rig.probe.diarizer_gate.close();
    let id = start_meeting(&rig);
    rig.room.play(&speech(), &speech());
    rig.dictation.toggle_meeting();
    wait_for("the speaker stage", || {
        rig.probe.diarizer_gate.entered.load(Ordering::SeqCst)
    });

    rig.dictation.cancel_meeting_work(&id);
    assert_eq!(wait_work_done(&rig, &id), None);

    assert_eq!(rig.probe.scoped_interrupts.load(Ordering::SeqCst), 1);
    assert_eq!(rig.probe.interrupts.load(Ordering::SeqCst), 0);
    let stopped = meeting(&rig, &id);
    assert_eq!(stopped.record().stages.diarization, StageState::Pending);
    assert_eq!(stopped.record().stages.summary, StageState::Pending);
    assert!(text_of(&stopped).contains("we ship on friday"));
    assert_eq!(rig.dictation.meetings_status().work, []);
}

#[test]
fn a_cancelled_meeting_finishes_when_asked_again() {
    let rig = meeting_rig(meeting_settings());
    rig.probe.diarizer_gate.close();
    let id = start_meeting(&rig);
    rig.room.play(&speech(), &speech());
    rig.dictation.toggle_meeting();
    wait_for("the speaker stage", || {
        rig.probe.diarizer_gate.entered.load(Ordering::SeqCst)
    });
    rig.dictation.cancel_meeting_work(&id);
    wait_work_done(&rig, &id);

    rig.dictation.finish_meeting(&id);
    assert_eq!(wait_work_done(&rig, &id), None);

    let stages = meeting(&rig, &id).record().stages.clone();
    assert!(stages.diarization.is_done() && stages.summary.is_done());
}

#[test]
fn a_summary_is_written_again_on_request() {
    let rig = meeting_rig(meeting_settings());
    let id = finished_meeting(&rig);
    let before = rig.probe.llm_prompts.lock().unwrap().len();

    rig.dictation
        .summarize_meeting(&id, SummaryMode::Regenerate);
    assert_eq!(wait_work_done(&rig, &id), None);

    assert!(rig.probe.llm_prompts.lock().unwrap().len() > before);
}

#[test]
fn a_second_request_for_a_busy_meeting_is_not_queued() {
    let rig = meeting_rig(meeting_settings());
    let id = finished_meeting(&rig);
    rig.probe.llm_gate.close();
    rig.probe.llm_gate.entered.store(false, Ordering::SeqCst);

    rig.dictation
        .summarize_meeting(&id, SummaryMode::Regenerate);
    rig.dictation
        .summarize_meeting(&id, SummaryMode::Regenerate);
    wait_for("the summary", || {
        rig.probe.llm_gate.entered.load(Ordering::SeqCst)
    });
    assert_eq!(rig.dictation.meetings_status().work.len(), 1);

    rig.probe.llm_gate.open();
    wait_work_done(&rig, &id);
}

#[test]
fn a_chat_answer_streams_and_is_saved_with_the_meeting() {
    let rig = meeting_rig(meeting_settings());
    let id = finished_meeting(&rig);

    let chat = rig.dictation.ask_meeting(&id, "When do we ship?");
    let mut streamed = String::new();
    let answer = rig.wait_event("the answer", |event| match event {
        Event::Meetings(MeetingNews::Chat { run, update }) if run == chat.run => match update {
            ChatUpdate::Token(token) => {
                streamed.push_str(&token);
                None
            }
            ChatUpdate::Done(answer) => Some(answer),
        },
        _ => None,
    });

    let turn = answer.unwrap();
    assert_eq!(turn.answer, "Friday [00:01].");
    assert_eq!(streamed, "streaming ");
    assert_eq!(meeting(&rig, &id).chat().unwrap(), [turn]);
    assert!(
        rig.probe
            .llm_prompts
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("we ship on friday")
    );
}

#[test]
fn stop_ends_only_that_answer_and_keeps_what_came() {
    let rig = meeting_rig(meeting_settings());
    let id = finished_meeting(&rig);
    let loads = rig.probe.llm_loads.load(Ordering::SeqCst);
    rig.probe.llm_gate.close();
    rig.probe.llm_gate.entered.store(false, Ordering::SeqCst);

    let chat = rig.dictation.ask_meeting(&id, "When do we ship?");
    wait_for("the model", || {
        rig.probe.llm_gate.entered.load(Ordering::SeqCst)
    });
    chat.stop();
    let turn = wait_chat(&rig, chat.run).unwrap();

    assert!(turn.stopped);
    assert_eq!(turn.answer, "streaming");
    assert_eq!(meeting(&rig, &id).chat().unwrap(), [turn]);
    assert_eq!(rig.probe.interrupts.load(Ordering::SeqCst), 0);

    let next = rig.dictation.ask_meeting(&id, "And after that?");
    assert!(!wait_chat(&rig, next.run).unwrap().stopped);
    assert_eq!(
        rig.probe.llm_loads.load(Ordering::SeqCst),
        loads + 1,
        "the host came back"
    );
}

#[test]
fn a_chat_needs_a_model_and_says_so() {
    let rig = meeting_rig(base_settings());
    let id = finished_meeting(&rig);
    let chat = rig.dictation.ask_meeting(&id, "When do we ship?");
    let problem = wait_chat(&rig, chat.run).unwrap_err();
    assert!(problem.contains("needs a model"), "{problem}");
}

#[test]
fn dictation_keeps_working_while_a_chat_answer_runs() {
    let rig = meeting_rig(meeting_settings());
    let id = finished_meeting(&rig);
    rig.probe.llm_gate.close();
    rig.probe.llm_gate.entered.store(false, Ordering::SeqCst);
    let chat = rig.dictation.ask_meeting(&id, "When do we ship?");
    wait_for("the model", || {
        rig.probe.llm_gate.entered.load(Ordering::SeqCst)
    });

    rig.probe.hears("hello there");
    rig.dictate(&speech());
    rig.wait_inserted();
    assert_eq!(rig.inserted(), ["Hello there."]);

    rig.probe.llm_gate.open();
    assert!(wait_chat(&rig, chat.run).is_ok());
}

#[test]
fn a_dictation_cancelled_during_a_chat_leaves_the_answer_alone() {
    let mut settings = meeting_settings();
    settings.pipeline.llm_cleanup = true;
    let rig = meeting_rig(settings);
    let id = finished_meeting(&rig);
    rig.probe.llm_gate.close();
    rig.probe.llm_gate.entered.store(false, Ordering::SeqCst);
    let chat = rig.dictation.ask_meeting(&id, "When do we ship?");
    wait_for("the model", || {
        rig.probe.llm_gate.entered.load(Ordering::SeqCst)
    });

    rig.press(FN);
    rig.mic.wait_open();
    rig.mic.play(&speech());
    rig.clock.advance(Duration::from_secs(2));
    rig.release(FN);
    rig.wait_pill(PillState::Processing);
    rig.clock.advance(Duration::from_secs(1));
    rig.press(Key::Esc);
    rig.wait_pill(PillState::Resting);

    rig.probe.llm_gate.open();
    assert!(!wait_chat(&rig, chat.run).unwrap().stopped);
}

#[test]
fn the_model_stays_for_meeting_work_and_goes_after_it() {
    let mut settings = meeting_settings();
    settings.pipeline.keep_models_loaded = KeepModels::UnloadAfterUse;
    let rig = meeting_rig(settings);
    let id = finished_meeting(&rig);
    wait_for("the model to go", || {
        rig.probe.llm_alive.load(Ordering::SeqCst) == 0
    });

    rig.probe.llm_gate.close();
    rig.probe.llm_gate.entered.store(false, Ordering::SeqCst);
    let chat = rig.dictation.ask_meeting(&id, "When do we ship?");
    wait_for("the model", || {
        rig.probe.llm_gate.entered.load(Ordering::SeqCst)
    });
    rig.settle();
    assert_eq!(rig.probe.llm_alive.load(Ordering::SeqCst), 1);

    rig.probe.llm_gate.open();
    wait_chat(&rig, chat.run).unwrap();
    wait_for("the model to go", || {
        rig.probe.llm_alive.load(Ordering::SeqCst) == 0
    });
}

#[test]
fn quitting_during_a_meeting_closes_the_recording() {
    let rig = meeting_rig(meeting_settings());
    let id = start_meeting(&rig);
    rig.room.play(&speech(), &silence());

    rig.dictation.close_meetings(Duration::from_secs(5));

    let closed = meeting(&rig, &id);
    assert!(!closed.is_recording());
    assert_eq!(
        closed.record().parts[0].duration_ms,
        Some(rig.room.played().as_millis() as u64)
    );
    assert_eq!(
        closed.record().stages.summary,
        StageState::Pending,
        "no new work starts"
    );
    assert!(read_meeting(&rig.room.paths.lock().unwrap()[0]).is_ok());
    assert_eq!(rig.room.stopped.load(Ordering::SeqCst), 1);
}

#[test]
fn quitting_without_a_meeting_does_not_wait() {
    let rig = meeting_rig(meeting_settings());
    rig.dictation.close_meetings(Duration::from_secs(30));
    assert_eq!(rig.room.stopped.load(Ordering::SeqCst), 0);
}

#[test]
fn a_quit_cut_short_is_finished_at_the_next_launch() {
    let rig = meeting_rig(meeting_settings());
    rig.probe.asr_gate.close();
    let id = start_meeting(&rig);
    rig.room.play(&long_speech(), &silence());
    wait_for("the transcriber to stick", || {
        rig.probe.asr_gate.entered.load(Ordering::SeqCst)
    });

    rig.dictation.close_meetings(Duration::from_millis(50));

    assert!(
        meeting(&rig, &id).is_recording(),
        "the live transcript is stuck"
    );
    let data = tempfile::tempdir().unwrap();
    let copied = data.path().join("meetings").join(&id);
    fs::create_dir_all(&copied).unwrap();
    let folder = rig.data.path().join("meetings").join(&id);
    for file in fs::read_dir(&folder).unwrap() {
        let file = file.unwrap().path();
        fs::copy(&file, copied.join(file.file_name().unwrap())).unwrap();
    }
    let models = data.path().join("models");
    fs::create_dir_all(&models).unwrap();
    let next = Rig::with_models(meeting_settings(), data, models);
    next.wait_event("the list", |event| {
        (event == Event::Meetings(MeetingNews::ListChanged)).then_some(())
    });

    assert!(!meeting(&next, &id).is_recording());
    rig.probe.asr_gate.open();
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
fn the_bars_follow_the_others_when_the_mic_is_quiet() {
    let rig = Rig::new(base_settings());
    rig.dictation.toggle_meeting();
    wait_meeting(&rig, true);

    for level in [0.001, 0.3] {
        for _ in 0..8 {
            rig.room.send(MeetingEvent::Frame {
                channel: Channel::Others,
                samples: vec![level; FRAME_SAMPLES],
            });
        }
    }

    rig.wait_event("the others' level", |event| match event {
        Event::Level(level) if level > 0.5 => Some(()),
        _ => None,
    });
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
fn a_clip_room_plays_a_meeting_through_the_real_file_and_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("part.ogg");
    let room = ClipRoom::new(speech(), silence(), 100.0);
    let (timeline, frames) = mpsc::sync_channel(1_000);

    let tape = room
        .record(path.clone(), Vec::new(), Some(timeline))
        .unwrap();
    wait_for("the clip to play", || room.played());
    let length = tape.stop().unwrap();

    assert_eq!(length, speech().duration());
    let played = speech().samples().len() / FRAME_SAMPLES;
    assert_eq!(frames.try_iter().count(), played);
    assert_eq!(read_meeting(&path).unwrap().you.duration(), length);
}

/// Starts the app over `prepare`d data, with nothing installed.
fn launch(prepare: impl FnOnce(&Path)) -> Rig {
    let data = tempfile::tempdir().unwrap();
    let meetings: PathBuf = data.path().join("meetings");
    fs::create_dir_all(&meetings).unwrap();
    prepare(&meetings);
    let models = data.path().join("models");
    fs::create_dir_all(&models).unwrap();
    Rig::with_models(base_settings(), data, models)
}

#[test]
fn a_meeting_a_crash_cut_off_is_closed_at_launch() {
    let mut id = String::new();
    let rig = launch(|meetings| {
        let mut meeting = MeetingStore::new(meetings).create().unwrap();
        crashed_recording(&meeting.begin_part().unwrap().path);
        id = meeting.record().id.clone();
    });

    rig.wait_event("the list", |event| {
        (event == Event::Meetings(MeetingNews::ListChanged)).then_some(())
    });

    let repaired = meeting(&rig, &id);
    assert!(!repaired.is_recording());
    assert!(repaired.length() >= Duration::from_secs(1));
}

#[test]
fn a_recording_a_crash_left_behind_comes_in_as_a_meeting() {
    let rig = launch(|meetings| crashed_recording(&meetings.join("1000.recording")));

    rig.wait_event("the list", |event| {
        (event == Event::Meetings(MeetingNews::ListChanged)).then_some(())
    });

    let records = store(&rig).list().unwrap();
    assert_eq!(records.len(), 1);
    let adopted = meeting(&rig, &records[0].id);
    assert!(!adopted.is_recording());
    assert!(adopted.audio().unwrap().you.duration() >= Duration::from_secs(1));
    assert!(!rig.data.path().join("meetings/1000.recording").exists());
}

#[test]
fn a_finished_flat_recording_comes_in_as_a_meeting_too() {
    let rig = launch(|meetings| {
        let path = meetings.join("1000.ogg");
        let mut writer = MeetingWriter::create(&path).unwrap();
        writer
            .write_frame(&[0.1; FRAME_SAMPLES], &[0.1; FRAME_SAMPLES])
            .unwrap();
        writer.finish().unwrap();
    });

    rig.wait_event("the list", |event| {
        (event == Event::Meetings(MeetingNews::ListChanged)).then_some(())
    });

    assert_eq!(store(&rig).list().unwrap().len(), 1);
}

#[test]
fn a_file_that_is_no_recording_stays_put() {
    let rig =
        launch(|meetings| fs::write(meetings.join("1000.recording"), "not a meeting").unwrap());
    rig.settle();
    assert!(rig.data.path().join("meetings/1000.recording").exists());
    assert_eq!(store(&rig).list().unwrap(), []);
}
