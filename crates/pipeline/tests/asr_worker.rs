//! Streaming chunk mode against a fake engine: chunks queue while a slow engine works, and the
//! results come back in order with their place in the recording.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};
use wf_pipeline::vad::LiveChunk;
use wf_pipeline::worker::{AsrWorker, ChunkTranscript, FailedChunk};
use wf_pipeline::{Asr, AsrOptions, PipelineError, Transcript};

fn chunk(start_ms: u64, ms: usize, level: f32) -> LiveChunk {
    LiveChunk {
        start: Duration::from_millis(start_ms),
        clip: AudioClip::new(vec![level; ms * SAMPLE_RATE_HZ as usize / 1_000]),
    }
}

/// Says back the clip's level, so each result can be traced to its chunk.
struct LevelEcho;

impl Asr for LevelEcho {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        let level = clip.samples()[0];
        if level < 0.0 {
            return Err(PipelineError::Engine("bad chunk".into()));
        }
        if level == 0.0 {
            return Ok(Transcript::default());
        }
        Ok(Transcript {
            text: format!("level {level}"),
            language: options.language.clone(),
        })
    }
}

/// Doesn't finish a chunk until the test says so, or drops its end of the gate.
struct Gated {
    started: Sender<()>,
    release: Receiver<()>,
}

impl Asr for Gated {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        self.started.send(()).unwrap();
        let _ = self.release.recv();
        LevelEcho.transcribe(clip, options)
    }
}

fn gated() -> (Box<dyn Asr>, Receiver<()>, Sender<()>) {
    let (started, on_start) = channel();
    let (release_sender, release) = channel();
    (
        Box::new(Gated { started, release }),
        on_start,
        release_sender,
    )
}

fn texts<S>(results: Vec<Result<ChunkTranscript<S>, PipelineError>>) -> Vec<String> {
    results
        .into_iter()
        .map(|result| result.unwrap().transcript.text)
        .collect()
}

#[test]
fn results_keep_the_order_and_place_of_their_chunks() {
    let worker = AsrWorker::spawn(Box::new(LevelEcho), AsrOptions::default());
    worker.submit((), chunk(1_000, 2_000, 0.25)).unwrap();
    worker.submit((), chunk(9_500, 500, 0.5)).unwrap();

    let (results, _asr) = worker.finish().unwrap();

    let results: Vec<_> = results.into_iter().map(Result::unwrap).collect();
    assert_eq!(results[0].transcript.text, "level 0.25");
    assert_eq!(results[0].start, Duration::from_millis(1_000));
    assert_eq!(results[0].end, Duration::from_millis(3_000));
    assert_eq!(results[1].transcript.text, "level 0.5");
    assert_eq!(results[1].start, Duration::from_millis(9_500));
    assert_eq!(results[1].end, Duration::from_millis(10_000));
}

#[test]
fn every_chunk_gets_the_workers_options() {
    let options = AsrOptions {
        language: Some("it".into()),
        ..AsrOptions::default()
    };
    let worker = AsrWorker::spawn(Box::new(LevelEcho), options);
    worker.submit((), chunk(0, 100, 0.5)).unwrap();

    let (results, _asr) = worker.finish().unwrap();

    let transcript = results.into_iter().next().unwrap().unwrap().transcript;
    assert_eq!(transcript.language.as_deref(), Some("it"));
}

/// The caller feeds audio in real time and must not wait for a slow model.
#[test]
fn submitting_never_waits_for_the_engine() {
    let (asr, on_start, release) = gated();
    let worker = AsrWorker::spawn(asr, AsrOptions::default());

    worker.submit((), chunk(0, 100, 0.25)).unwrap();
    worker.submit((), chunk(100, 100, 0.5)).unwrap();
    on_start.recv().unwrap();

    assert!(worker.poll().is_empty());
    release.send(()).unwrap();
    release.send(()).unwrap();
    let (results, _asr) = worker.finish().unwrap();
    assert_eq!(texts(results), ["level 0.25", "level 0.5"]);
}

#[test]
fn polling_hands_out_each_finished_chunk_once() {
    let (asr, on_start, release) = gated();
    let worker = AsrWorker::spawn(asr, AsrOptions::default());
    worker.submit((), chunk(0, 100, 0.25)).unwrap();
    worker.submit((), chunk(100, 100, 0.5)).unwrap();

    on_start.recv().unwrap();
    release.send(()).unwrap();
    // The engine starting on the next chunk means the first one is published.
    on_start.recv().unwrap();
    let first = worker.poll();

    release.send(()).unwrap();
    let (rest, _asr) = worker.finish().unwrap();
    assert_eq!(texts(first), ["level 0.25"]);
    assert_eq!(texts(rest), ["level 0.5"]);
}

/// A meeting's two channels share one loaded model.
#[test]
fn sources_share_the_engine_and_stay_labelled() {
    #[derive(Debug, PartialEq)]
    enum Channel {
        You,
        Others,
    }
    let worker = AsrWorker::spawn(Box::new(LevelEcho), AsrOptions::default());
    worker.submit(Channel::You, chunk(0, 100, 0.25)).unwrap();
    worker.submit(Channel::Others, chunk(0, 100, 0.5)).unwrap();

    let (results, _asr) = worker.finish().unwrap();

    let labelled: Vec<_> = results
        .into_iter()
        .map(Result::unwrap)
        .map(|result| (result.source, result.transcript.text))
        .collect();
    assert_eq!(
        labelled,
        [
            (Channel::You, "level 0.25".to_owned()),
            (Channel::Others, "level 0.5".to_owned())
        ]
    );
}

#[test]
fn a_chunk_with_no_words_is_not_a_result() {
    let worker = AsrWorker::spawn(Box::new(LevelEcho), AsrOptions::default());
    worker.submit((), chunk(0, 100, 0.0)).unwrap();
    worker.submit((), chunk(100, 100, 0.5)).unwrap();

    let (results, _asr) = worker.finish().unwrap();

    assert_eq!(texts(results), ["level 0.5"]);
}

#[test]
fn a_failed_chunk_is_reported_and_the_next_one_still_runs() {
    let worker = AsrWorker::spawn(Box::new(LevelEcho), AsrOptions::default());
    worker.submit((), chunk(0, 100, -1.0)).unwrap();
    worker.submit((), chunk(100, 100, 0.5)).unwrap();

    let (results, _asr) = worker.finish().unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_ref().unwrap_err().to_string(), "bad chunk");
    assert_eq!(results[1].as_ref().unwrap().transcript.text, "level 0.5");
}

/// Reloading a model between dictations would cost seconds.
#[test]
fn finishing_hands_the_engine_back_for_the_next_recording() {
    let worker = AsrWorker::spawn(Box::new(LevelEcho), AsrOptions::default());
    worker.submit((), chunk(0, 100, 0.25)).unwrap();
    let (_, asr) = worker.finish().unwrap();

    let worker = AsrWorker::spawn(asr, AsrOptions::default());
    worker.submit((), chunk(0, 100, 0.5)).unwrap();
    let (results, _asr) = worker.finish().unwrap();

    assert_eq!(texts(results), ["level 0.5"]);
}

#[test]
fn cancelling_skips_what_is_queued_and_keeps_the_engine() {
    let (asr, on_start, release) = gated();
    let worker = AsrWorker::spawn(asr, AsrOptions::default());
    worker.submit((), chunk(0, 100, 0.25)).unwrap();
    worker.submit((), chunk(100, 100, 0.5)).unwrap();
    worker.submit((), chunk(200, 100, 0.75)).unwrap();
    on_start.recv().unwrap();

    // The first chunk holds the worker until cancel has had time to take effect. Releasing it
    // any earlier lets the next chunk start, which cancel doesn't interrupt.
    let cancelling = thread::spawn(move || worker.cancel());
    thread::sleep(Duration::from_millis(200));
    drop(release);
    let mut asr = cancelling.join().unwrap().unwrap();

    assert_eq!(on_start.try_iter().count(), 0);
    let transcript = asr
        .transcribe(&chunk(0, 100, 0.5).clip, &AsrOptions::default())
        .unwrap();
    assert_eq!(transcript.text, "level 0.5");
}

struct Panics;

impl Asr for Panics {
    fn transcribe(
        &mut self,
        _clip: &AudioClip,
        _options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        panic!("engine bug");
    }
}

#[test]
fn a_dead_engine_is_an_error_not_a_hang() {
    let worker = AsrWorker::spawn(Box::new(Panics), AsrOptions::default());
    worker.submit((), chunk(0, 100, 0.5)).unwrap();

    let error = worker.finish().err().unwrap();

    assert!(matches!(error, PipelineError::Engine(_)), "{error}");
}

/// A meeting has to know how far behind the model is, to stop queueing before memory runs out.
#[test]
fn the_backlog_counts_audio_until_its_chunk_is_done() {
    let (asr, on_start, release) = gated();
    let worker = AsrWorker::spawn(asr, AsrOptions::default());
    assert_eq!(worker.backlog(), Duration::ZERO);

    worker.submit((), chunk(0, 2_000, 0.25)).unwrap();
    worker.submit((), chunk(2_000, 3_000, 0.5)).unwrap();
    on_start.recv().unwrap();
    assert_eq!(worker.backlog(), Duration::from_secs(5));

    release.send(()).unwrap();
    on_start.recv().unwrap();
    assert_eq!(worker.backlog(), Duration::from_secs(3));

    release.send(()).unwrap();
    let (_results, _asr) = worker.finish().unwrap();
}

#[test]
fn a_chunk_with_no_words_still_leaves_the_backlog() {
    let (asr, on_start, release) = gated();
    let worker = AsrWorker::spawn(asr, AsrOptions::default());

    worker.submit((), chunk(0, 1_000, 0.0)).unwrap();
    on_start.recv().unwrap();
    release.send(()).unwrap();
    worker.submit((), chunk(1_000, 1_000, 0.5)).unwrap();
    on_start.recv().unwrap();

    assert_eq!(worker.backlog(), Duration::from_secs(1));
    release.send(()).unwrap();
    let (_results, _asr) = worker.finish().unwrap();
}

/// An error alone doesn't say where the hole in the transcript is.
#[test]
fn a_failed_chunk_reports_its_label_and_place() {
    let worker = AsrWorker::spawn(Box::new(LevelEcho), AsrOptions::default());
    worker.submit("system", chunk(2_000, 500, -1.0)).unwrap();
    worker.submit("mic", chunk(3_000, 500, 0.5)).unwrap();

    while worker.backlog() > Duration::ZERO {
        thread::yield_now();
    }

    assert_eq!(
        worker.failed(),
        [FailedChunk {
            source: "system",
            start: Duration::from_millis(2_000),
            end: Duration::from_millis(2_500),
        }]
    );
    assert!(worker.failed().is_empty(), "each failure comes out once");
    let (_results, _asr) = worker.finish().unwrap();
}
