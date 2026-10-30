//! The VAD stage against a fake detector: trimming, chunking at pauses, the ASR gate and
//! hands-free auto-stop.

use std::time::Duration;

use wf_core::audio::{AudioClip, FRAME_MS, FRAME_SAMPLES, SAMPLE_RATE_HZ};
use wf_pipeline::vad::{LiveVad, VadOptions, speech_chunks, transcribe_speech};
use wf_pipeline::{Asr, AsrOptions, PipelineError, SpeechSegment, Transcript, Vad};

const VOICE: f32 = 0.5;
const QUIET: f32 = 0.0;
/// How long the fake waits before calling a pause, like Silero's minimum silence.
const HANGOVER_MS: usize = 500;

fn samples(ms: usize) -> usize {
    ms * SAMPLE_RATE_HZ as usize / 1_000
}

/// Builds audio from `(level, milliseconds)` runs.
fn audio(runs: &[(f32, usize)]) -> Vec<f32> {
    runs.iter()
        .flat_map(|&(level, ms)| vec![level; samples(ms)])
        .collect()
}

fn clip(runs: &[(f32, usize)]) -> AudioClip {
    AudioClip::new(audio(runs))
}

fn duration_ms(chunk: &AudioClip) -> usize {
    chunk.samples().len() * 1_000 / SAMPLE_RATE_HZ as usize
}

/// Any non-zero sample is speech, so test audio describes its own ground truth.
#[derive(Default)]
struct LoudIsSpeech {
    position: usize,
    open_at: Option<usize>,
    last_loud: usize,
}

impl Vad for LoudIsSpeech {
    fn accept(&mut self, samples_in: &[f32]) -> Result<Vec<SpeechSegment>, PipelineError> {
        let mut closed = Vec::new();
        for &sample in samples_in {
            if sample != 0.0 {
                self.open_at.get_or_insert(self.position);
                self.last_loud = self.position + 1;
            } else if let Some(start) = self.open_at
                && self.position - self.last_loud >= samples(HANGOVER_MS)
            {
                closed.push(SpeechSegment {
                    start,
                    end: self.last_loud,
                });
                self.open_at = None;
            }
            self.position += 1;
        }
        Ok(closed)
    }

    fn is_speaking(&self) -> bool {
        self.open_at.is_some()
    }

    fn finish(&mut self) -> Result<Vec<SpeechSegment>, PipelineError> {
        let open = self.open_at.map(|start| SpeechSegment {
            start,
            end: self.last_loud,
        });
        *self = Self::default();
        Ok(open.into_iter().collect())
    }
}

fn chunks_of(recording: &AudioClip, options: &VadOptions) -> Vec<AudioClip> {
    speech_chunks(recording, Some(&mut LoudIsSpeech::default()), options).unwrap()
}

#[test]
fn flushing_closes_a_short_chunk_and_the_next_one_starts_after_it() {
    let mut vad = LoudIsSpeech::default();
    let mut live = LiveVad::new(&mut vad, VadOptions::default());
    let first = live
        .push(&audio(&[(VOICE, 1_000), (QUIET, 1_000)]))
        .unwrap();
    assert!(first.chunks.is_empty(), "shorter than the minimum");

    let flushed = live.flush().expect("a chunk was open");
    assert!(live.flush().is_none(), "nothing left");

    assert_eq!(flushed.start, Duration::ZERO);
    let next = live
        .push(&audio(&[(VOICE, 1_000), (QUIET, 1_000)]))
        .unwrap();
    assert!(next.chunks.is_empty());
    let rest = live.finish().unwrap();
    assert!(
        rest[0].start >= flushed.clip.duration(),
        "{:?}",
        rest[0].start
    );
}

#[test]
fn a_silent_recording_has_no_chunks() {
    let silent = clip(&[(QUIET, 5_000)]);

    assert!(chunks_of(&silent, &VadOptions::default()).is_empty());
}

#[test]
fn leading_and_trailing_silence_are_trimmed_down_to_the_padding() {
    let recording = clip(&[(QUIET, 2_000), (VOICE, 3_000), (QUIET, 2_000)]);
    let options = VadOptions {
        padding: Duration::from_millis(200),
        ..VadOptions::default()
    };

    let chunks = chunks_of(&recording, &options);

    assert_eq!(chunks.len(), 1);
    assert_eq!(
        chunks[0].samples(),
        audio(&[(QUIET, 200), (VOICE, 3_000), (QUIET, 200)])
    );
}

fn chunking_after(seconds: u64) -> VadOptions {
    VadOptions {
        min_chunk: Duration::from_secs(seconds),
        padding: Duration::from_millis(200),
        ..VadOptions::default()
    }
}

#[test]
fn a_short_dictation_stays_whole_across_its_pauses() {
    let recording = clip(&[(VOICE, 2_000), (QUIET, 1_000), (VOICE, 2_000)]);

    let chunks = chunks_of(&recording, &chunking_after(15));

    assert_eq!(chunks.len(), 1);
    assert_eq!(duration_ms(&chunks[0]), 5_000);
}

#[test]
fn a_long_dictation_splits_at_the_first_pause_past_the_minimum() {
    let recording = clip(&[
        (VOICE, 6_000),
        (QUIET, 1_000),
        (VOICE, 2_000),
        (QUIET, 1_000),
        (VOICE, 6_000),
    ]);

    let chunks = chunks_of(&recording, &chunking_after(5));

    assert_eq!(
        chunks.iter().map(duration_ms).collect::<Vec<_>>(),
        // 6 s + trailing padding; then padding + 2 s + 1 s + 6 s.
        vec![6_200, 9_200]
    );
}

#[test]
fn speech_with_no_pause_is_never_cut() {
    let recording = clip(&[(VOICE, 40_000)]);

    let chunks = chunks_of(&recording, &chunking_after(5));

    assert_eq!(chunks.len(), 1);
    assert_eq!(duration_ms(&chunks[0]), 40_000);
}

#[test]
fn padding_never_repeats_audio_when_a_pause_is_shorter_than_both_paddings() {
    let recording = clip(&[(VOICE, 6_000), (QUIET, 600), (VOICE, 6_000)]);
    let options = VadOptions {
        min_chunk: Duration::from_secs(5),
        padding: Duration::from_millis(400),
        ..VadOptions::default()
    };

    let chunks = chunks_of(&recording, &options);

    assert_eq!(
        chunks.iter().map(duration_ms).collect::<Vec<_>>(),
        vec![6_400, 6_200]
    );
}

#[test]
fn with_vad_off_the_whole_recording_passes_through() {
    let recording = clip(&[(QUIET, 1_000), (VOICE, 1_000), (QUIET, 1_000)]);

    let chunks = speech_chunks(&recording, None, &VadOptions::default()).unwrap();

    assert_eq!(chunks, vec![recording]);
}

#[test]
fn with_vad_off_an_empty_recording_still_has_no_chunks() {
    let chunks = speech_chunks(&AudioClip::default(), None, &VadOptions::default()).unwrap();

    assert!(chunks.is_empty());
}

#[test]
fn the_toggle_is_an_optional_boxed_engine() {
    let recording = clip(&[(QUIET, 1_000), (VOICE, 1_000), (QUIET, 1_000)]);
    let mut engine: Option<Box<dyn Vad>> = Some(Box::new(LoudIsSpeech::default()));

    let trimmed = speech_chunks(&recording, engine.as_deref_mut(), &VadOptions::default());
    engine = None;
    let whole = speech_chunks(&recording, engine.as_deref_mut(), &VadOptions::default());

    assert_eq!(duration_ms(&trimmed.unwrap()[0]), 1_400);
    assert_eq!(whole.unwrap(), vec![recording]);
}

/// A meeting's `[mm:ss]` comes from here.
#[test]
fn live_chunks_know_where_they_start_in_the_recording() {
    let mut vad = LoudIsSpeech::default();
    let options = VadOptions {
        min_chunk: Duration::from_secs(1),
        ..VadOptions::default()
    };
    let recording = audio(&[
        (QUIET, 3_000),
        (VOICE, 2_000),
        (QUIET, 4_000),
        (VOICE, 2_000),
    ]);
    let mut live = LiveVad::new(&mut vad, options);

    let mut chunks = Vec::new();
    for frame in recording.chunks(FRAME_SAMPLES) {
        chunks.extend(live.push(frame).unwrap().chunks);
    }
    chunks.extend(live.finish().unwrap());

    let starts: Vec<_> = chunks.iter().map(|chunk| chunk.start).collect();
    assert_eq!(
        starts,
        [Duration::from_millis(2_800), Duration::from_millis(8_800)]
    );
}

#[test]
fn a_boxed_engine_serves_one_live_dictation_after_another() {
    let mut engine: Box<dyn Vad> = Box::new(LoudIsSpeech::default());

    let first = run_live(
        engine.as_mut(),
        &audio(&[(VOICE, 1_000)]),
        VadOptions::default(),
    );
    let second = run_live(
        engine.as_mut(),
        &audio(&[(VOICE, 2_000)]),
        VadOptions::default(),
    );

    assert_eq!(duration_ms(&first.chunks[0].1), 1_000);
    assert_eq!(duration_ms(&second.chunks[0].1), 2_000);
}

/// Replies with one canned text per call and counts the calls.
struct ScriptedAsr {
    replies: Vec<&'static str>,
    calls: usize,
}

impl ScriptedAsr {
    fn new(replies: &[&'static str]) -> Self {
        Self {
            replies: replies.to_vec(),
            calls: 0,
        }
    }
}

impl Asr for ScriptedAsr {
    fn transcribe(
        &mut self,
        _clip: &AudioClip,
        _options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        self.calls += 1;
        Ok(Transcript {
            text: self.replies[self.calls - 1].to_owned(),
            language: Some("it".into()),
        })
    }
}

#[test]
fn a_silent_recording_makes_no_asr_call_and_no_text() {
    let silent = clip(&[(QUIET, 5_000)]);
    let mut asr = ScriptedAsr::new(&[]);

    let chunks = chunks_of(&silent, &VadOptions::default());
    let transcript = transcribe_speech(chunks, &mut asr, &AsrOptions::default()).unwrap();

    assert_eq!(asr.calls, 0);
    assert_eq!(transcript, None);
}

#[test]
fn chunk_transcripts_join_in_order_into_one() {
    let chunks = vec![clip(&[(VOICE, 100)]), clip(&[(VOICE, 100)])];
    let mut asr = ScriptedAsr::new(&["ciao a tutti", "buona giornata"]);

    let transcript = transcribe_speech(chunks, &mut asr, &AsrOptions::default()).unwrap();

    assert_eq!(asr.calls, 2);
    assert_eq!(
        transcript,
        Some(Transcript {
            text: "ciao a tutti buona giornata".into(),
            language: Some("it".into()),
        })
    );
}

#[test]
fn an_engine_that_hears_no_words_gives_no_text() {
    let chunks = vec![clip(&[(VOICE, 100)])];
    let mut asr = ScriptedAsr::new(&["  "]);

    let transcript = transcribe_speech(chunks, &mut asr, &AsrOptions::default()).unwrap();

    assert_eq!(transcript, None);
}

/// What came out of a live recording, fed in capture frames.
#[derive(Default)]
struct LiveRun {
    /// Each chunk with the time it came out, in ms from the start of the recording.
    chunks: Vec<(usize, AudioClip)>,
    auto_stop_at_ms: Option<usize>,
}

fn run_live(vad: &mut dyn Vad, audio: &[f32], options: VadOptions) -> LiveRun {
    let mut live = LiveVad::new(vad, options);
    let mut run = LiveRun::default();
    for (index, frame) in audio.chunks(FRAME_SAMPLES).enumerate() {
        let now_ms = (index + 1) * FRAME_MS as usize;
        let update = live.push(frame).unwrap();
        run.chunks
            .extend(update.chunks.into_iter().map(|chunk| (now_ms, chunk.clip)));
        if update.auto_stop && run.auto_stop_at_ms.is_none() {
            run.auto_stop_at_ms = Some(now_ms);
        }
    }
    let end_ms = audio.len() * 1_000 / SAMPLE_RATE_HZ as usize;
    run.chunks.extend(
        live.finish()
            .unwrap()
            .into_iter()
            .map(|chunk| (end_ms, chunk.clip)),
    );
    run
}

#[test]
fn a_silent_live_recording_produces_no_chunks() {
    let run = run_live(
        &mut LoudIsSpeech::default(),
        &audio(&[(QUIET, 5_000)]),
        VadOptions::default(),
    );

    assert!(run.chunks.is_empty());
}

#[test]
fn a_long_dictation_hands_chunks_to_asr_before_the_user_stops() {
    let recording = audio(&[(VOICE, 6_000), (QUIET, 1_000), (VOICE, 2_000)]);

    let run = run_live(&mut LoudIsSpeech::default(), &recording, chunking_after(5));

    let times: Vec<_> = run.chunks.iter().map(|(at_ms, _)| *at_ms).collect();
    // The first chunk leaves with the capture frame in which the hangover confirms the pause
    // (6.5 s falls inside the frame ending at 6.52 s), the rest when the recording ends.
    assert_eq!(times, vec![6_520, 9_000]);
}

#[test]
fn live_chunks_match_the_chunks_of_the_finished_recording() {
    let recording = clip(&[
        (QUIET, 1_000),
        (VOICE, 6_000),
        (QUIET, 1_000),
        (VOICE, 2_000),
        (QUIET, 1_000),
        (VOICE, 6_000),
        (QUIET, 1_000),
        (VOICE, 1_000),
        (QUIET, 2_000),
    ]);

    let run = run_live(
        &mut LoudIsSpeech::default(),
        recording.samples(),
        chunking_after(5),
    );

    let live: Vec<_> = run.chunks.into_iter().map(|(_, chunk)| chunk).collect();
    assert_eq!(live.len(), 3);
    assert_eq!(live, chunks_of(&recording, &chunking_after(5)));
}

fn auto_stop_after(seconds: u64) -> VadOptions {
    VadOptions {
        auto_stop: Some(Duration::from_secs(seconds)),
        ..VadOptions::default()
    }
}

#[test]
fn auto_stop_is_off_by_default() {
    let run = run_live(
        &mut LoudIsSpeech::default(),
        &audio(&[(VOICE, 1_000), (QUIET, 60_000)]),
        VadOptions::default(),
    );

    assert_eq!(run.auto_stop_at_ms, None);
}

#[test]
fn auto_stop_fires_after_that_much_silence_following_speech() {
    let run = run_live(
        &mut LoudIsSpeech::default(),
        &audio(&[(VOICE, 1_000), (QUIET, 10_000)]),
        auto_stop_after(3),
    );

    assert_eq!(run.auto_stop_at_ms, Some(4_000));
}

#[test]
fn speech_restarts_the_auto_stop_silence() {
    let run = run_live(
        &mut LoudIsSpeech::default(),
        &audio(&[
            (VOICE, 1_000),
            (QUIET, 2_000),
            (VOICE, 1_000),
            (QUIET, 2_000),
            (VOICE, 1_000),
            (QUIET, 10_000),
        ]),
        auto_stop_after(3),
    );

    assert_eq!(run.auto_stop_at_ms, Some(10_000));
}

#[test]
fn auto_stop_also_ends_a_dictation_nobody_spoke_in() {
    let run = run_live(
        &mut LoudIsSpeech::default(),
        &audio(&[(QUIET, 10_000)]),
        auto_stop_after(3),
    );

    assert_eq!(run.auto_stop_at_ms, Some(3_000));
    assert!(run.chunks.is_empty());
}

#[test]
fn a_cancelled_recording_leaves_nothing_behind_for_the_next_one() {
    let mut vad = LoudIsSpeech::default();
    let mut cancelled = LiveVad::new(&mut vad, VadOptions::default());
    cancelled.push(&audio(&[(VOICE, 1_000)])).unwrap();
    cancelled.finish().unwrap();

    let run = run_live(
        &mut vad,
        &audio(&[(QUIET, 1_000), (VOICE, 1_000)]),
        VadOptions::default(),
    );

    assert_eq!(run.chunks.len(), 1);
    assert_eq!(
        run.chunks[0].1.samples(),
        audio(&[(QUIET, 200), (VOICE, 1_000)])
    );
}

/// Deterministic xorshift so failures reproduce.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

fn random_recording(rng: &mut Rng) -> Vec<f32> {
    let runs: Vec<_> = (0..1 + rng.below(12))
        .flat_map(|_| {
            [
                (QUIET, rng.below(3_000)),
                (VOICE, 50 + rng.below(8_000)),
                (QUIET, rng.below(1_500)),
            ]
        })
        .collect();
    audio(&runs)
}

fn voiced(chunks: &[AudioClip]) -> usize {
    chunks
        .iter()
        .flat_map(|chunk| chunk.samples())
        .filter(|&&sample| sample != QUIET)
        .count()
}

#[test]
fn chunks_keep_all_the_speech_once_and_live_agrees_with_finished() {
    let mut rng = Rng(0x5EED_CAFE);
    let options = chunking_after(5);

    for _ in 0..100 {
        let recording = AudioClip::new(random_recording(&mut rng));

        let chunks = chunks_of(&recording, &options);
        let run = run_live(
            &mut LoudIsSpeech::default(),
            recording.samples(),
            options.clone(),
        );

        assert_eq!(voiced(&chunks), voiced(std::slice::from_ref(&recording)));
        assert!(chunks.iter().all(|chunk| !chunk.samples().is_empty()));
        let live: Vec<_> = run.chunks.into_iter().map(|(_, chunk)| chunk).collect();
        assert_eq!(live, chunks);
    }
}
