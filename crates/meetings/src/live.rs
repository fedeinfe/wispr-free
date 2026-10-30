//! The live transcript (spec §4): each channel is cut into chunks by VAD and transcribed as they
//! close. This sits on the recording's timeline, so its timestamps match the file, and nothing
//! here can hold the recording up: frames arrive through a channel the recorder never waits on,
//! and ASR runs on its own thread. When ASR can't keep up the backlog grows to a limit and then
//! chunks are skipped, each leaving a [`Gap`] that the refine pass fills in.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wf_core::audio::{AudioClip, FRAME_MS, FRAME_SAMPLES};
use wf_pipeline::language::SETTLING_SPEECH;
use wf_pipeline::vad::{LiveChunk, LiveVad, VadOptions};
use wf_pipeline::worker::{AsrWorker, ChunkTranscript};
use wf_pipeline::{Asr, AsrOptions, PipelineError, Vad};

use crate::recorder::TimelineFrame;
use crate::store::{LiveLog, Meeting, NewPart};
use crate::transcript::{Gap, GapReason, Segment};
use crate::{Channel, Error};

/// About 10 s of frames, for when this thread is held up. Past that the recorder skips frames.
const TIMELINE_BUFFER: usize = 256;
/// [`LiveEvent`]s beyond this aren't delivered. Everything is in the log either way.
const EVENT_BUFFER: usize = 1_024;
/// Short enough to read as a live transcript and give citations a useful place to jump to.
const MIN_CHUNK: Duration = Duration::from_secs(6);
/// Three minutes of audio in memory, about 6 MB per channel. A live transcript this far behind
/// isn't live any more, and the refine pass redoes it all anyway.
const MAX_BACKLOG: Duration = Duration::from_secs(180);
/// A quiet channel shows what was said this long after it went quiet, however short it was.
const FLUSH_AFTER_SILENCE: Duration = Duration::from_secs(2);
/// How often results are collected when no audio is coming in.
const RESULT_POLL: Duration = Duration::from_millis(100);
/// How often to look at the backlog after the audio ended.
const DRAIN_POLL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, PartialEq)]
pub struct LiveOptions {
    pub vad: VadOptions,
    pub asr: AsrOptions,
    /// How far behind ASR may fall before chunks are skipped.
    pub max_backlog: Duration,
}

impl Default for LiveOptions {
    fn default() -> Self {
        Self {
            vad: VadOptions {
                min_chunk: MIN_CHUNK,
                // Here this doesn't stop anything: it's the signal to flush a quiet channel.
                auto_stop: Some(FLUSH_AFTER_SILENCE),
                ..VadOptions::default()
            },
            asr: AsrOptions::default(),
            max_backlog: MAX_BACKLOG,
        }
    }
}

/// The engines live transcription runs on, handed back by [`LiveTranscript::finish`].
pub struct LiveEngines {
    pub asr: Box<dyn Asr>,
    pub you_vad: Box<dyn Vad>,
    pub others_vad: Box<dyn Vad>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEvent {
    Segment(Segment),
    /// Audio that has no text, and why.
    Gap(Gap),
    /// The live log can't be written, or live transcription stopped. The recording goes on either
    /// way, and the refine pass doesn't need any of this.
    Failed(String),
}

/// The live transcript of one recording.
pub struct LiveTranscript {
    feed: SyncSender<TimelineFrame>,
    events: Receiver<LiveEvent>,
    thread: JoinHandle<Result<LiveEngines, Error>>,
}

impl LiveTranscript {
    /// Starts it on `part`. Pass [`feed`](Self::feed) as [`MeetingConfig::timeline`], and call
    /// [`finish`](Self::finish) once the recorder has stopped.
    ///
    /// [`MeetingConfig::timeline`]: crate::MeetingConfig::timeline
    pub fn start(
        meeting: &Meeting,
        part: &NewPart,
        engines: LiveEngines,
        options: LiveOptions,
    ) -> Result<Self, Error> {
        let log = LiveLog::open(meeting)?;
        let offset = part.offset;
        let (feed, frames) = mpsc::sync_channel(TIMELINE_BUFFER);
        let (events_tx, events) = mpsc::sync_channel(EVENT_BUFFER);
        let thread = thread::Builder::new()
            .name("wf-meeting-live".into())
            .spawn(move || {
                let events = events_tx.clone();
                let result = run(frames, engines, options, offset, log, events_tx);
                if let Err(error) = &result {
                    tracing::error!("the live transcript stopped: {error}");
                    let _ = events.try_send(LiveEvent::Failed(error.to_string()));
                }
                result
            })?;
        Ok(Self {
            feed,
            events,
            thread,
        })
    }

    /// Where the audio goes.
    pub fn feed(&self) -> SyncSender<TimelineFrame> {
        self.feed.clone()
    }

    /// Optional, for showing the transcript as it grows. Nothing waits for this to be read.
    pub fn events(&self) -> &Receiver<LiveEvent> {
        &self.events
    }

    /// Waits for the audio to end and the backlog to be transcribed, and returns the engines.
    /// Drop every [`feed`](Self::feed) first: stopping the recorder does that for its own.
    pub fn finish(self) -> Result<LiveEngines, Error> {
        let Self { feed, thread, .. } = self;
        drop(feed);
        thread
            .join()
            .map_err(|_| PipelineError::Engine("the live transcript thread panicked".into()))?
    }
}

const FRAME_MS_U64: u64 = FRAME_MS as u64;

fn run(
    frames: Receiver<TimelineFrame>,
    engines: LiveEngines,
    options: LiveOptions,
    offset: Duration,
    log: LiveLog,
    events: SyncSender<LiveEvent>,
) -> Result<LiveEngines, Error> {
    let LiveEngines {
        asr,
        mut you_vad,
        mut others_vad,
    } = engines;
    let mut you = LiveVad::new(&mut *you_vad, options.vad.clone());
    let mut others = LiveVad::new(&mut *others_vad, options.vad.clone());
    let mut output = Output {
        log,
        events,
        offset,
    };
    let mut gate = LanguageGate::new(&options.asr);
    let worker = AsrWorker::spawn(asr, options.asr);
    let silence = vec![0.0; FRAME_SAMPLES];
    let mut next_index = 0;

    loop {
        let TimelineFrame { index, frame } = match frames.recv_timeout(RESULT_POLL) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => {
                output.collect(&worker);
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        // Frames the recorder had to skip still happened. Silence keeps the clock right.
        if index > next_index {
            let (start, end) = (next_index * FRAME_MS_U64, index * FRAME_MS_U64);
            for channel in [Channel::You, Channel::Others] {
                output.gap(channel, start, end, GapReason::Behind);
            }
            for _ in next_index..index {
                you.push(&silence)?;
                others.push(&silence)?;
            }
        }
        next_index = index + 1;

        let closed = [
            (Channel::You, ready(&mut you, &frame.you)?),
            (Channel::Others, ready(&mut others, &frame.others)?),
        ];
        for (channel, chunks) in closed {
            for chunk in chunks {
                gate.submit(&worker, channel, chunk, options.max_backlog, &mut output)?;
            }
        }
        output.collect(&worker);
    }

    let closed = [
        (Channel::You, you.finish()?),
        (Channel::Others, others.finish()?),
    ];
    for (channel, chunks) in closed {
        for chunk in chunks {
            gate.submit(&worker, channel, chunk, options.max_backlog, &mut output)?;
        }
    }
    gate.release(&worker, options.max_backlog, &mut output)?;
    // `AsrWorker::finish` drops the failure list, so wait for it to empty first.
    while worker.backlog() > Duration::ZERO && worker.is_running() {
        output.collect(&worker);
        thread::sleep(DRAIN_POLL);
    }
    output.collect(&worker);
    let (rest, asr) = worker.finish()?;
    output.publish(rest);
    Ok(LiveEngines {
        asr,
        you_vad,
        others_vad,
    })
}

type Worker = AsrWorker<Channel>;

type ChunkResult = Result<ChunkTranscript<Channel>, PipelineError>;

/// Feeds one frame and returns the chunks it closed.
fn ready(vad: &mut LiveVad, samples: &[f32]) -> Result<Vec<LiveChunk>, PipelineError> {
    let update = vad.push(samples)?;
    let mut chunks = update.chunks;
    if update.auto_stop {
        chunks.extend(vad.flush());
    }
    Ok(chunks)
}

/// Queues a chunk, or skips it if ASR is too far behind.
fn submit(
    worker: &Worker,
    channel: Channel,
    chunk: LiveChunk,
    max_backlog: Duration,
    output: &mut Output,
) -> Result<(), Error> {
    if worker.backlog() >= max_backlog {
        let start = chunk.start.as_millis() as u64;
        let end = (chunk.start + chunk.clip.duration()).as_millis() as u64;
        output.gap(channel, start, end, GapReason::Behind);
        return Ok(());
    }
    Ok(worker.submit(channel, chunk)?)
}

/// Holds the first chunks back until there's enough speech to tell the language from. Their text
/// shows a little late, which beats showing it in the wrong language. Both channels count, since a
/// call has one language far more often than two.
struct LanguageGate {
    held: Vec<(Channel, LiveChunk)>,
    speech: Duration,
    open: bool,
}

impl LanguageGate {
    fn new(options: &AsrOptions) -> Self {
        Self {
            held: Vec::new(),
            speech: Duration::ZERO,
            open: options.language.is_some(),
        }
    }

    fn submit(
        &mut self,
        worker: &Worker,
        channel: Channel,
        chunk: LiveChunk,
        max_backlog: Duration,
        output: &mut Output,
    ) -> Result<(), Error> {
        if self.open {
            return submit(worker, channel, chunk, max_backlog, output);
        }
        self.speech += chunk.clip.duration();
        self.held.push((channel, chunk));
        if self.speech >= SETTLING_SPEECH {
            self.release(worker, max_backlog, output)?;
        }
        Ok(())
    }

    /// Lets everything through. The end of the audio calls this too, however little was said.
    fn release(
        &mut self,
        worker: &Worker,
        max_backlog: Duration,
        output: &mut Output,
    ) -> Result<(), Error> {
        self.open = true;
        let held = std::mem::take(&mut self.held);
        if held.is_empty() {
            return Ok(());
        }
        let speech = held.iter().flat_map(|(_, chunk)| chunk.clip.samples());
        worker.settle_language(AudioClip::new(speech.copied().collect()))?;
        for (channel, chunk) in held {
            submit(worker, channel, chunk, max_backlog, output)?;
        }
        Ok(())
    }
}

/// Where results go: the log first, then whoever is watching.
struct Output {
    log: LiveLog,
    events: SyncSender<LiveEvent>,
    offset: Duration,
}

impl Output {
    fn collect(&mut self, worker: &Worker) {
        self.publish(worker.poll());
        for chunk in worker.failed() {
            let (start, end) = (chunk.start.as_millis() as u64, chunk.end.as_millis() as u64);
            self.gap(chunk.source, start, end, GapReason::Failed);
        }
    }

    fn publish(&mut self, results: Vec<ChunkResult>) {
        for result in results {
            let chunk = match result {
                Ok(chunk) => chunk,
                Err(error) => {
                    // The failed chunk's place arrives through `failures`.
                    tracing::warn!("a live chunk failed: {error}");
                    continue;
                }
            };
            let segment = Segment {
                channel: chunk.source,
                speaker: None,
                start_ms: self.timeline_ms(chunk.start),
                end_ms: self.timeline_ms(chunk.end),
                text: chunk.transcript.text,
            };
            let written = self.log.segment(&segment);
            self.report(written);
            self.announce(LiveEvent::Segment(segment));
        }
    }

    /// `start_ms` and `end_ms` are on this recording's clock.
    fn gap(&mut self, channel: Channel, start_ms: u64, end_ms: u64, reason: GapReason) {
        let offset = self.offset.as_millis() as u64;
        let gap = Gap {
            channel,
            start_ms: offset + start_ms,
            end_ms: offset + end_ms,
            reason,
        };
        let written = self.log.gap(&gap);
        self.report(written);
        self.announce(LiveEvent::Gap(gap));
    }

    fn timeline_ms(&self, time: Duration) -> u64 {
        (self.offset + time).as_millis() as u64
    }

    fn report(&self, written: Result<(), Error>) {
        if let Err(error) = written {
            tracing::error!("the live transcript can't be saved: {error}");
            self.announce(LiveEvent::Failed(error.to_string()));
        }
    }

    fn announce(&self, event: LiveEvent) {
        if let Err(TrySendError::Disconnected(_)) = self.events.try_send(event) {
            tracing::debug!("nobody is watching the live transcript");
        }
    }
}
