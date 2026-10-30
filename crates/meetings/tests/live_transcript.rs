//! Live transcription from the recording's timeline: timestamps, a slow model, and what the
//! transcript shows when audio goes missing.

mod support;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use support::{BrokenAsr, LengthAsr, LevelVad};
use tempfile::TempDir;
use wf_core::audio::{AudioClip, FRAME_SAMPLES};
use wf_meetings::{
    Channel, GapReason, LiveEngines, LiveEvent, LiveOptions, LiveTranscript, Meeting, MeetingStore,
    NewPart, StereoFrame, TimelineFrame,
};
use wf_pipeline::{Asr, AsrOptions, PipelineError, Transcript};

fn engines(asr: Box<dyn Asr>) -> LiveEngines {
    LiveEngines {
        asr,
        you_vad: Box::new(LevelVad::default()),
        others_vad: Box::new(LevelVad::default()),
    }
}

fn new_meeting() -> (TempDir, Meeting, NewPart) {
    let dir = tempfile::tempdir().unwrap();
    let mut meeting = MeetingStore::new(dir.path()).create().unwrap();
    let part = meeting.begin_part().unwrap();
    (dir, meeting, part)
}

fn frame(index: u64, you: f32, others: f32) -> TimelineFrame {
    TimelineFrame {
        index,
        frame: StereoFrame {
            you: vec![you; FRAME_SAMPLES],
            others: vec![others; FRAME_SAMPLES],
        },
    }
}

/// Plays `spec`, a list of (frames, you, others) levels, and returns the next index.
fn play(live: &LiveTranscript, start: u64, spec: &[(u64, f32, f32)]) -> u64 {
    let feed = live.feed();
    let mut index = start;
    for &(frames, you, others) in spec {
        for _ in 0..frames {
            feed.send(frame(index, you, others)).unwrap();
            index += 1;
        }
    }
    index
}

/// 1 s of speech on one channel, 3 s of silence, 1 s on the other, then silence again. 25 frames
/// make a second.
const TWO_SPEAKERS: &[(u64, f32, f32)] = &[
    (25, 0.5, 0.0),
    (75, 0.0, 0.0),
    (25, 0.0, 0.5),
    (75, 0.0, 0.0),
];

#[test]
fn each_channel_is_transcribed_on_the_meetings_clock() {
    let (_dir, meeting, part) = new_meeting();
    let live = LiveTranscript::start(
        &meeting,
        &part,
        engines(Box::new(LengthAsr)),
        LiveOptions::default(),
    )
    .unwrap();

    play(&live, 0, TWO_SPEAKERS);
    live.finish().unwrap();

    let transcript = meeting.transcript().unwrap();
    assert_eq!(transcript.segments.len(), 2, "{transcript:?}");
    let you = &transcript.segments[0];
    let others = &transcript.segments[1];
    assert_eq!(you.channel, Channel::You);
    assert_eq!(others.channel, Channel::Others);
    assert_eq!(others.speaker, None, "diarization comes later");
    assert!(you.start_ms < 200, "{you:?}");
    assert!(others.start_ms.abs_diff(4_000) < 300, "{others:?}");
    assert!(others.end_ms.abs_diff(5_000) < 300, "{others:?}");
    assert!(you.text.starts_with("heard"));
    assert!(transcript.gaps.is_empty());
}

#[test]
fn a_resumed_recording_continues_the_clock() {
    let dir = tempfile::tempdir().unwrap();
    let mut meeting = MeetingStore::new(dir.path()).create().unwrap();
    meeting.begin_part().unwrap();
    meeting.end_part(Duration::from_secs(600)).unwrap();
    let part = meeting.begin_part().unwrap();
    let live = LiveTranscript::start(
        &meeting,
        &part,
        engines(Box::new(LengthAsr)),
        LiveOptions::default(),
    )
    .unwrap();

    play(&live, 0, TWO_SPEAKERS);
    live.finish().unwrap();

    let segments = meeting.transcript().unwrap().segments;
    assert!(segments[0].start_ms >= 600_000, "{segments:?}");
    assert!(segments[1].start_ms.abs_diff(604_000) < 300, "{segments:?}");
}

#[test]
fn segments_are_announced_as_they_arrive() {
    let (_dir, meeting, part) = new_meeting();
    let live = LiveTranscript::start(
        &meeting,
        &part,
        engines(Box::new(LengthAsr)),
        LiveOptions::default(),
    )
    .unwrap();

    play(&live, 0, TWO_SPEAKERS);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut segments = 0;
    while segments < 2 && Instant::now() < deadline {
        if let Ok(LiveEvent::Segment(_)) = live.events().recv_timeout(Duration::from_millis(50)) {
            segments += 1;
        }
    }
    assert_eq!(segments, 2);

    live.finish().unwrap();
}

/// A model that hasn't finished its first chunk.
struct Stuck {
    release: Receiver<()>,
}

impl Asr for Stuck {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        let _ = self.release.recv();
        LengthAsr.transcribe(clip, options)
    }
}

fn stuck() -> (Box<dyn Asr>, Sender<()>) {
    let (release_sender, release) = channel();
    (Box::new(Stuck { release }), release_sender)
}

/// Feeding is what the recording does. However far behind ASR is, this must never wait.
#[test]
fn a_stuck_model_never_holds_up_the_audio() {
    let (_dir, meeting, part) = new_meeting();
    let (asr, release) = stuck();
    let live =
        LiveTranscript::start(&meeting, &part, engines(asr), LiveOptions::default()).unwrap();

    let started = Instant::now();
    let feed = live.feed();
    for round in 0..40u64 {
        for n in 0..150 {
            feed.send(frame(round * 150 + n, if n < 50 { 0.5 } else { 0.0 }, 0.0))
                .unwrap();
        }
    }

    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    drop(feed);
    drop(release);
    live.finish().unwrap();
}

/// Past the limit audio is skipped rather than queued for ever, and the transcript says so.
#[test]
fn past_the_backlog_limit_chunks_are_dropped_with_a_visible_marker() {
    let (_dir, meeting, part) = new_meeting();
    let (asr, release) = stuck();
    let options = LiveOptions {
        max_backlog: Duration::from_secs(3),
        ..LiveOptions::default()
    };
    let live = LiveTranscript::start(&meeting, &part, engines(asr), options).unwrap();

    // Ten 2 s stretches of speech, each followed by 2 s of pause.
    let spec: Vec<(u64, f32, f32)> = (0..10)
        .flat_map(|_| [(50, 0.5, 0.0), (50, 0.0, 0.0)])
        .collect();
    play(&live, 0, &spec);
    drop(release);
    live.finish().unwrap();

    let transcript = meeting.transcript().unwrap();
    assert!(!transcript.segments.is_empty());
    assert!(transcript.segments.len() < 10, "{transcript:?}");
    assert!(!transcript.gaps.is_empty(), "{transcript:?}");
    assert!(
        transcript
            .gaps
            .iter()
            .all(|g| g.reason == GapReason::Behind)
    );
    assert!(transcript.gaps[0].start_ms > 0);
    assert!(transcript.gaps[0].end_ms > transcript.gaps[0].start_ms);
}

#[test]
fn a_gap_is_announced_too() {
    let (_dir, meeting, part) = new_meeting();
    let (asr, release) = stuck();
    let options = LiveOptions {
        max_backlog: Duration::from_secs(1),
        ..LiveOptions::default()
    };
    let live = LiveTranscript::start(&meeting, &part, engines(asr), options).unwrap();

    let spec: Vec<(u64, f32, f32)> = (0..6)
        .flat_map(|_| [(50, 0.5, 0.0), (50, 0.0, 0.0)])
        .collect();
    play(&live, 0, &spec);

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut announced = false;
    while !announced && Instant::now() < deadline {
        announced = matches!(
            live.events().recv_timeout(Duration::from_millis(50)),
            Ok(LiveEvent::Gap(_))
        );
    }
    assert!(announced);
    drop(release);
    live.finish().unwrap();
}

/// The recorder skips frames when this thread can't keep up. The clock must not slip, or every
/// later timestamp is wrong.
#[test]
fn frames_the_reader_missed_keep_their_place_on_the_clock() {
    let (_dir, meeting, part) = new_meeting();
    let live = LiveTranscript::start(
        &meeting,
        &part,
        engines(Box::new(LengthAsr)),
        LiveOptions::default(),
    )
    .unwrap();

    let next = play(&live, 0, &[(50, 0.0, 0.0)]);
    let after_jump = next + 250; // 10 s went missing
    play(&live, after_jump, &[(25, 0.5, 0.0), (50, 0.0, 0.0)]);
    live.finish().unwrap();

    let transcript = meeting.transcript().unwrap();
    assert_eq!(transcript.segments.len(), 1, "{transcript:?}");
    assert!(
        transcript.segments[0].start_ms.abs_diff(12_000) < 300,
        "{transcript:?}"
    );
    assert_eq!(transcript.gaps.len(), 2, "one per channel: {transcript:?}");
    assert_eq!(transcript.gaps[0].start_ms, 2_000);
    assert_eq!(transcript.gaps[0].end_ms, 12_000);
    assert_eq!(transcript.gaps[0].reason, GapReason::Behind);
}

#[test]
fn a_failed_chunk_leaves_a_marker_and_the_next_one_still_runs() {
    let (_dir, meeting, part) = new_meeting();
    let live = LiveTranscript::start(
        &meeting,
        &part,
        engines(Box::new(BrokenAsr)),
        LiveOptions::default(),
    )
    .unwrap();

    play(&live, 0, TWO_SPEAKERS);
    live.finish().unwrap();

    let transcript = meeting.transcript().unwrap();
    assert!(transcript.segments.is_empty());
    assert_eq!(transcript.gaps.len(), 2, "{transcript:?}");
    assert!(
        transcript
            .gaps
            .iter()
            .all(|g| g.reason == GapReason::Failed)
    );
}

#[test]
fn finishing_hands_the_engines_back_for_the_next_stage() {
    let (_dir, meeting, part) = new_meeting();
    let live = LiveTranscript::start(
        &meeting,
        &part,
        engines(Box::new(LengthAsr)),
        LiveOptions::default(),
    )
    .unwrap();
    play(&live, 0, TWO_SPEAKERS);

    let mut engines = live.finish().unwrap();

    let heard = engines
        .asr
        .transcribe(&AudioClip::new(vec![0.5; 160]), &AsrOptions::default())
        .unwrap();
    assert_eq!(heard.text, "heard 10 ms");
}
