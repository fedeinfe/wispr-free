//! Everything after the recording stops, with fake engines: which stages run on which plan, and
//! what stays usable when one of them fails.

mod support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use support::{BrokenAsr, BrokenDiarizer, FixedDiarizer, LengthAsr, LevelVad, ScriptedLlm};
use tempfile::TempDir;
use wf_core::audio::{AudioClip, FRAME_SAMPLES};
use wf_meetings::{
    Channel, Error, Gap, GapReason, LiveLog, Meeting, MeetingEngines, MeetingPlan, MeetingStore,
    MeetingWriter, ProcessOptions, Segment, SpeakerId, Stage, StageState, SummaryMode,
    SummaryOptions, finish_meeting, generate_summary,
};
use wf_pipeline::{
    Asr, AsrOptions, Diarizer, Llm, LlmPrompt, PipelineError, SpeakerTurn, Transcript, Vad,
};

const BALANCED: MeetingPlan = MeetingPlan {
    refine: true,
    diarize: true,
    summarize: true,
    keep_audio: true,
};

const LITE: MeetingPlan = MeetingPlan {
    refine: false,
    diarize: false,
    summarize: false,
    keep_audio: true,
};

const NOTES: &str = "KEY POINTS\n- heard\nDECISIONS\n- none\nACTIONS\n- none";
const FINAL: &str =
    "TITLE: Tones\nTLDR: heard\nKEY POINTS\n- heard\nDECISIONS\n- none\nACTIONS\n- none";

fn working_llm() -> ScriptedLlm {
    ScriptedLlm::new(|prompt: &LlmPrompt| {
        Ok(if prompt.system.contains("TITLE:") {
            FINAL
        } else {
            NOTES
        }
        .to_owned())
    })
}

/// Hands out fakes and keeps count of what was asked for.
struct Engines {
    asr: Box<dyn FnMut() -> Result<Box<dyn Asr>, PipelineError>>,
    diarizer: Box<dyn FnMut() -> Result<Box<dyn Diarizer>, PipelineError>>,
    llm: Box<dyn FnMut() -> Result<Box<dyn Llm>, PipelineError>>,
    asked: Arc<Mutex<Vec<&'static str>>>,
}

impl Default for Engines {
    fn default() -> Self {
        Self {
            asr: Box::new(|| Ok(Box::new(LengthAsr))),
            diarizer: Box::new(|| Ok(Box::new(two_voices()))),
            llm: Box::new(|| Ok(Box::new(working_llm()))),
            asked: Arc::default(),
        }
    }
}

impl MeetingEngines for Engines {
    fn asr(&mut self) -> Result<Box<dyn Asr>, PipelineError> {
        self.asked.lock().unwrap().push("asr");
        (self.asr)()
    }

    fn vad(&mut self) -> Result<Box<dyn Vad>, PipelineError> {
        Ok(Box::new(LevelVad::default()))
    }

    fn diarizer(&mut self) -> Result<Box<dyn Diarizer>, PipelineError> {
        self.asked.lock().unwrap().push("diarizer");
        (self.diarizer)()
    }

    fn llm(&mut self) -> Result<Box<dyn Llm>, PipelineError> {
        self.asked.lock().unwrap().push("llm");
        (self.llm)()
    }
}

fn turn(start_ms: u64, end_ms: u64, speaker: u32) -> SpeakerTurn {
    SpeakerTurn {
        start: Duration::from_millis(start_ms),
        end: Duration::from_millis(end_ms),
        speaker,
    }
}

/// Matches [`record_meeting`]: one voice for the first 3 s of Others, another for the next 3 s.
fn two_voices() -> FixedDiarizer {
    FixedDiarizer(vec![turn(2_000, 5_000, 5), turn(5_000, 8_000, 9)])
}

fn tone(level: f32) -> Vec<f32> {
    (0..FRAME_SAMPLES)
        .map(|n| level * (std::f32::consts::TAU * n as f32 / 40.0).sin())
        .collect()
}

/// You speak for the first second, Others from 2 s to 8 s, and a last stretch of silence.
fn write_recording(path: &std::path::Path) -> Duration {
    let mut writer = MeetingWriter::create(path).unwrap();
    for n in 0..250 {
        let you = if n < 25 { 0.5 } else { 0.0 };
        let others = if (50..200).contains(&n) { 0.5 } else { 0.0 };
        writer.write_frame(&tone(you), &tone(others)).unwrap();
    }
    writer.finish().unwrap()
}

fn record_meeting() -> (TempDir, Meeting) {
    let dir = tempfile::tempdir().unwrap();
    let mut meeting = MeetingStore::new(dir.path()).create().unwrap();
    record_part(&mut meeting);
    (dir, meeting)
}

fn record_part(meeting: &mut Meeting) -> Duration {
    let part = meeting.begin_part().unwrap();
    let offset = part.offset.as_millis() as u64;
    let length = write_recording(&part.path);
    meeting.end_part(length).unwrap();
    let mut log = LiveLog::open(meeting).unwrap();
    for (channel, start_ms, end_ms, text) in [
        (Channel::You, 0, 1_000, "live you"),
        (Channel::Others, 2_000, 8_000, "live others"),
    ] {
        log.segment(&Segment {
            channel,
            speaker: None,
            start_ms: offset + start_ms,
            end_ms: offset + end_ms,
            text: text.to_owned(),
        })
        .unwrap();
    }
    length
}

fn run(meeting: &mut Meeting, plan: MeetingPlan, engines: &mut Engines) -> Result<(), Error> {
    finish_meeting(
        meeting,
        plan,
        engines,
        &ProcessOptions::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
}

fn texts(meeting: &Meeting) -> Vec<String> {
    let transcript = meeting.transcript().unwrap();
    transcript.segments.into_iter().map(|s| s.text).collect()
}

#[test]
fn balanced_refines_diarizes_and_summarizes() {
    let (_dir, mut meeting) = record_meeting();

    run(&mut meeting, BALANCED, &mut Engines::default()).unwrap();

    let segments = meeting.transcript().unwrap().segments;
    assert!(
        segments.iter().all(|s| s.text.starts_with("heard")),
        "{segments:?}"
    );
    assert_eq!(segments[0].channel, Channel::You);
    assert_eq!(segments[0].speaker, None);
    let speakers: Vec<Option<SpeakerId>> = segments[1..].iter().map(|s| s.speaker).collect();
    assert_eq!(speakers, [Some(SpeakerId(1)), Some(SpeakerId(2))]);
    assert!(segments[1].end_ms <= segments[2].start_ms);
    assert_eq!(
        meeting.summary().unwrap().unwrap().title.as_deref(),
        Some("Tones")
    );
    let stages = &meeting.record().stages;
    assert!(stages.diarization.is_done() && stages.refine.is_done() && stages.summary.is_done());
    assert!(meeting.transcript_is_final().unwrap());
}

#[test]
fn lite_keeps_the_live_transcript_and_loads_nothing_else() {
    let (_dir, mut meeting) = record_meeting();
    let mut engines = Engines::default();

    run(&mut meeting, LITE, &mut engines).unwrap();

    assert_eq!(texts(&meeting), ["live you", "live others"]);
    assert!(engines.asked.lock().unwrap().is_empty());
    let stages = &meeting.record().stages;
    assert!(matches!(stages.refine, StageState::Skipped { .. }));
    assert!(matches!(stages.diarization, StageState::Skipped { .. }));
    assert!(matches!(stages.summary, StageState::Skipped { .. }));
    assert!(meeting.transcript_is_final().unwrap());
}

#[test]
fn a_tier_without_refine_can_still_diarize_the_live_transcript() {
    let (_dir, mut meeting) = record_meeting();
    let plan = MeetingPlan {
        diarize: true,
        ..LITE
    };

    run(&mut meeting, plan, &mut Engines::default()).unwrap();

    let segments = meeting.transcript().unwrap().segments;
    assert_eq!(segments[1].text, "live others");
    assert!(segments[1].speaker.is_some());
    assert_eq!(segments[0].speaker, None);
}

fn with_a_gap(meeting: &Meeting, start_ms: u64, end_ms: u64) {
    LiveLog::open(meeting)
        .unwrap()
        .gap(&Gap {
            channel: Channel::Others,
            start_ms,
            end_ms,
            reason: GapReason::Behind,
        })
        .unwrap();
}

#[test]
fn the_refine_pass_fills_the_gaps_the_live_pass_left() {
    let (_dir, mut meeting) = record_meeting();
    with_a_gap(&meeting, 3_000, 5_000);
    assert_eq!(meeting.transcript().unwrap().gaps.len(), 1);

    run(&mut meeting, BALANCED, &mut Engines::default()).unwrap();

    assert!(meeting.transcript().unwrap().gaps.is_empty());
}

/// Lite has no refine pass, and it's the tier most likely to fall behind. Its own model can
/// still catch up on what it skipped, without redoing the rest.
#[test]
fn without_refine_the_live_model_catches_up_on_its_gaps_alone() {
    let (_dir, mut meeting) = record_meeting();
    with_a_gap(&meeting, 2_000, 8_000);
    let mut engines = Engines::default();

    run(&mut meeting, LITE, &mut engines).unwrap();

    let transcript = meeting.transcript().unwrap();
    assert!(transcript.gaps.is_empty(), "{transcript:?}");
    assert_eq!(engines.asked.lock().unwrap().as_slice(), ["asr"]);
    let heard: Vec<&Segment> = transcript
        .segments
        .iter()
        .filter(|s| s.text.starts_with("heard"))
        .collect();
    assert_eq!(heard.len(), 1, "{transcript:?}");
    assert!(
        heard[0].start_ms >= 1_500 && heard[0].end_ms <= 8_500,
        "{heard:?}"
    );
    assert!(texts(&meeting).contains(&"live you".to_owned()));
}

#[test]
fn gaps_over_silence_clear_without_inventing_text() {
    let (_dir, mut meeting) = record_meeting();
    with_a_gap(&meeting, 8_500, 9_500);

    run(&mut meeting, LITE, &mut Engines::default()).unwrap();

    let transcript = meeting.transcript().unwrap();
    assert!(transcript.gaps.is_empty());
    assert_eq!(transcript.segments.len(), 2, "{transcript:?}");
}

#[test]
fn a_failed_diarization_costs_only_the_speakers() {
    let (_dir, mut meeting) = record_meeting();
    let mut engines = Engines {
        diarizer: Box::new(|| Ok(Box::new(BrokenDiarizer))),
        ..Engines::default()
    };

    run(&mut meeting, BALANCED, &mut engines).unwrap();

    let segments = meeting.transcript().unwrap().segments;
    assert!(segments.iter().all(|s| s.speaker.is_none()));
    assert!(segments.iter().all(|s| s.text.starts_with("heard")));
    let stages = &meeting.record().stages;
    assert!(
        matches!(stages.diarization, StageState::Failed { .. }),
        "{stages:?}"
    );
    assert!(stages.refine.is_done() && stages.summary.is_done());
}

#[test]
fn a_missing_speaker_model_is_a_failed_stage_and_nothing_more() {
    let (_dir, mut meeting) = record_meeting();
    let mut engines = Engines {
        diarizer: Box::new(|| Err(PipelineError::ModelMissing("segmentation".into()))),
        ..Engines::default()
    };

    run(&mut meeting, BALANCED, &mut engines).unwrap();

    let stages = &meeting.record().stages;
    assert!(
        matches!(&stages.diarization, StageState::Failed { reason } if reason.contains("segmentation"))
    );
    assert!(stages.summary.is_done());
}

#[test]
fn a_failed_refine_leaves_the_live_transcript_and_the_summary_uses_it() {
    let (_dir, mut meeting) = record_meeting();
    let mut engines = Engines {
        asr: Box::new(|| Ok(Box::new(BrokenAsr))),
        ..Engines::default()
    };

    run(&mut meeting, BALANCED, &mut engines).unwrap();

    assert_eq!(texts(&meeting), ["live you", "live others"]);
    let stages = &meeting.record().stages;
    assert!(
        matches!(stages.refine, StageState::Failed { .. }),
        "{stages:?}"
    );
    assert!(stages.summary.is_done());
    assert!(!meeting.transcript_is_final().unwrap());
}

/// Fails on the Nth chunk only.
struct FailsOnce {
    calls: usize,
    fail_on: usize,
}

impl Asr for FailsOnce {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        self.calls += 1;
        if self.calls == self.fail_on {
            return Err(PipelineError::Engine("one bad chunk".into()));
        }
        LengthAsr.transcribe(clip, options)
    }
}

#[test]
fn a_chunk_the_refine_pass_loses_keeps_its_live_text() {
    let (_dir, mut meeting) = record_meeting();
    let mut engines = Engines {
        asr: Box::new(|| {
            Ok(Box::new(FailsOnce {
                calls: 0,
                fail_on: 1,
            }))
        }),
        ..Engines::default()
    };

    run(&mut meeting, BALANCED, &mut engines).unwrap();

    let found = texts(&meeting);
    assert!(found.contains(&"live you".to_owned()), "{found:?}");
    assert!(found.iter().any(|t| t.starts_with("heard")), "{found:?}");
    assert!(meeting.record().stages.refine.is_done());
}

#[test]
fn a_failed_summary_leaves_the_transcript_and_can_be_retried() {
    let (_dir, mut meeting) = record_meeting();
    let mut engines = Engines {
        llm: Box::new(|| Err(PipelineError::ModelMissing("llm".into()))),
        ..Engines::default()
    };

    run(&mut meeting, BALANCED, &mut engines).unwrap();

    assert!(meeting.record().stages.refine.is_done());
    assert!(matches!(
        meeting.record().stages.summary,
        StageState::Failed { .. }
    ));
    assert!(meeting.summary().unwrap().is_none());
    assert!(!texts(&meeting).is_empty());

    generate_summary(
        &mut meeting,
        &mut working_llm(),
        SummaryMode::Regenerate,
        &SummaryOptions::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert!(meeting.record().stages.summary.is_done());
}

#[test]
fn a_meeting_nobody_spoke_in_has_no_summary_to_fail() {
    let dir = tempfile::tempdir().unwrap();
    let mut meeting = MeetingStore::new(dir.path()).create().unwrap();
    let part = meeting.begin_part().unwrap();
    let mut writer = MeetingWriter::create(&part.path).unwrap();
    for _ in 0..50 {
        writer.write_frame(&tone(0.0), &tone(0.0)).unwrap();
    }
    meeting.end_part(writer.finish().unwrap()).unwrap();
    let mut engines = Engines {
        diarizer: Box::new(|| Ok(Box::new(FixedDiarizer(Vec::new())))),
        ..Engines::default()
    };

    run(&mut meeting, BALANCED, &mut engines).unwrap();

    assert!(matches!(
        meeting.record().stages.summary,
        StageState::Skipped { .. }
    ));
}

/// Counts live instances, so a test can see that stages don't hold each other's models.
struct Counted<T> {
    inner: T,
    live: Arc<AtomicUsize>,
}

impl<T> Counted<T> {
    fn new(inner: T, live: &Arc<AtomicUsize>) -> Self {
        live.fetch_add(1, Ordering::SeqCst);
        Self {
            inner,
            live: live.clone(),
        }
    }
}

impl<T> Drop for Counted<T> {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Asr for Counted<LengthAsr> {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        self.inner.transcribe(clip, options)
    }
}

impl Diarizer for Counted<FixedDiarizer> {
    fn diarize(&mut self, clip: &AudioClip) -> Result<Vec<SpeakerTurn>, PipelineError> {
        self.inner.diarize(clip)
    }
}

/// Models take gigabytes. Each stage drops its own before the next one loads.
#[test]
fn no_two_stages_hold_a_model_at_once() {
    let (_dir, mut meeting) = record_meeting();
    let live = Arc::new(AtomicUsize::new(0));
    let (asr_live, diarizer_live, llm_live) = (live.clone(), live.clone(), live.clone());
    let mut engines = Engines {
        asr: Box::new(move || {
            assert_eq!(
                asr_live.load(Ordering::SeqCst),
                0,
                "ASR loaded on top of a model"
            );
            Ok(Box::new(Counted::new(LengthAsr, &asr_live)))
        }),
        diarizer: Box::new(move || {
            assert_eq!(diarizer_live.load(Ordering::SeqCst), 0);
            Ok(Box::new(Counted::new(two_voices(), &diarizer_live)))
        }),
        llm: Box::new(move || {
            assert_eq!(
                llm_live.load(Ordering::SeqCst),
                0,
                "the LLM loaded on top of a model"
            );
            Ok(Box::new(working_llm()))
        }),
        ..Engines::default()
    };

    run(&mut meeting, BALANCED, &mut engines).unwrap();

    assert_eq!(
        engines.asked.lock().unwrap().as_slice(),
        ["diarizer", "asr", "llm"]
    );
}

#[test]
fn a_rename_survives_refining_again_after_a_resumed_recording() {
    let (_dir, mut meeting) = record_meeting();
    run(&mut meeting, BALANCED, &mut Engines::default()).unwrap();
    meeting.rename_speaker(SpeakerId(2), "Giulia").unwrap();
    let first = record_part(&mut meeting);
    assert_eq!(meeting.record().stages.refine, StageState::Pending);
    assert!(meeting.summary_is_stale().unwrap());

    // Second run: the engine swaps its numbers and meets a new voice in the new part.
    let later = first.as_millis() as u64;
    let mut engines = Engines {
        diarizer: Box::new(move || {
            Ok(Box::new(FixedDiarizer(vec![
                turn(2_000, 5_000, 1),
                turn(5_000, 8_000, 0),
                turn(later + 2_000, later + 5_000, 0),
                turn(later + 5_000, later + 8_000, 2),
            ])))
        }),
        ..Engines::default()
    };
    run(&mut meeting, BALANCED, &mut engines).unwrap();

    let transcript = meeting.transcript().unwrap();
    let names = &meeting.record().speakers;
    let named: Vec<String> = transcript
        .segments
        .iter()
        .filter(|s| s.channel == Channel::Others)
        .map(|s| s.display_name(names))
        .collect();
    assert_eq!(named, ["Speaker 1", "Giulia", "Giulia", "Speaker 3"]);
    assert!(!meeting.summary_is_stale().unwrap());
}

#[test]
fn audio_is_deleted_when_the_plan_says_so_and_the_transcript_is_final() {
    let (dir, mut meeting) = record_meeting();
    let plan = MeetingPlan {
        keep_audio: false,
        ..BALANCED
    };

    run(&mut meeting, plan, &mut Engines::default()).unwrap();

    assert!(meeting.record().audio_deleted);
    let folder = dir.path().join(&meeting.record().id);
    assert!(!folder.join("part-000.ogg").exists());
    assert!(!texts(&meeting).is_empty());
}

#[test]
fn audio_stays_when_a_stage_that_needs_it_failed() {
    let (_dir, mut meeting) = record_meeting();
    let plan = MeetingPlan {
        keep_audio: false,
        ..BALANCED
    };
    let mut engines = Engines {
        asr: Box::new(|| Err(PipelineError::ModelMissing("asr".into()))),
        ..Engines::default()
    };

    run(&mut meeting, plan, &mut engines).unwrap();

    assert!(!meeting.record().audio_deleted);
    meeting.audio().unwrap();
}

/// New audio after the old audio was deleted: no refine is possible, but nothing else is lost.
#[test]
fn a_resumed_meeting_without_audio_keeps_what_it_has() {
    let (_dir, mut meeting) = record_meeting();
    let plan = MeetingPlan {
        keep_audio: false,
        ..BALANCED
    };
    run(&mut meeting, plan, &mut Engines::default()).unwrap();
    let refined = texts(&meeting);
    record_part(&mut meeting);

    run(&mut meeting, BALANCED, &mut Engines::default()).unwrap();

    let stages = &meeting.record().stages;
    assert!(
        matches!(stages.refine, StageState::Skipped { .. }),
        "{stages:?}"
    );
    assert!(
        matches!(stages.diarization, StageState::Skipped { .. }),
        "{stages:?}"
    );
    assert!(stages.summary.is_done());
    let found = texts(&meeting);
    assert!(found.starts_with(&refined), "{found:?}");
    assert!(
        found.contains(&"live you".to_owned()),
        "the new part keeps its live text"
    );
}

#[test]
fn cancelling_keeps_stages_pending_for_another_go() {
    let (_dir, mut meeting) = record_meeting();
    let cancel = AtomicBool::new(true);

    let result = finish_meeting(
        &mut meeting,
        BALANCED,
        &mut Engines::default(),
        &ProcessOptions::default(),
        &cancel,
        &mut |_| {},
    );

    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(meeting.record().stages.refine, StageState::Pending);
    assert_eq!(texts(&meeting), ["live you", "live others"]);
    run(&mut meeting, BALANCED, &mut Engines::default()).unwrap();
    assert!(meeting.record().stages.refine.is_done());
}

#[test]
fn refining_reports_how_far_along_it_is() {
    let (_dir, mut meeting) = record_meeting();
    let mut progress = Vec::new();

    finish_meeting(
        &mut meeting,
        BALANCED,
        &mut Engines::default(),
        &ProcessOptions::default(),
        &AtomicBool::new(false),
        &mut |update| progress.push(update),
    )
    .unwrap();

    use wf_meetings::ProcessProgress::{Refining, Stage as Starting};
    assert!(progress.contains(&Starting(Stage::Diarization)));
    assert!(progress.contains(&Starting(Stage::Refine)));
    assert!(progress.contains(&Starting(Stage::Summary)));
    let last = progress.iter().rev().find_map(|update| match update {
        Refining { done_ms, total_ms } => Some((*done_ms, *total_ms)),
        _ => None,
    });
    let (done, total) = last.expect("refine reported progress");
    assert_eq!(done, total);
}

#[test]
fn a_store_failure_is_an_error_not_a_stage_state() {
    let (dir, mut meeting) = record_meeting();
    std::fs::remove_dir_all(dir.path().join(&meeting.record().id)).unwrap();

    assert!(run(&mut meeting, LITE, &mut Engines::default()).is_err());
}
