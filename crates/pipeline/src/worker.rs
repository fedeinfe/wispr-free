//! Streaming chunk mode (spec §3, §4). VAD closes chunks while the recording runs, and this
//! transcribes them on the engine's own thread, so a slow model never holds up the audio. A
//! meeting's two channels share one worker, and so one loaded model.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::vad::LiveChunk;
use crate::{Asr, AsrOptions, PipelineError, Transcript};

/// The words in one chunk. `source` is whatever the caller labelled the chunk with.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkTranscript<S> {
    pub source: S,
    pub start: Duration,
    pub end: Duration,
    pub transcript: Transcript,
}

type ChunkResult<S> = Result<ChunkTranscript<S>, PipelineError>;
/// The results no poll took, and the engine, ready for the next recording.
type Finished<S> = (Vec<ChunkResult<S>>, Box<dyn Asr>);

/// Owns an engine for one recording. Chunks run one at a time, in the order submitted. Chunks
/// with no words produce no result; a failed chunk produces an error and the next one still runs.
pub struct AsrWorker<S> {
    jobs: Sender<(S, LiveChunk)>,
    results: Receiver<ChunkResult<S>>,
    cancelled: Arc<AtomicBool>,
    thread: JoinHandle<Box<dyn Asr>>,
}

impl<S: Send + 'static> AsrWorker<S> {
    pub fn spawn(mut asr: Box<dyn Asr>, options: AsrOptions) -> Self {
        let (jobs, queue) = mpsc::channel::<(S, LiveChunk)>();
        let (publish, results) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancelled);
        let thread = thread::spawn(move || {
            for (source, chunk) in queue {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let end = chunk.start + chunk.clip.duration();
                let result = match asr.transcribe(&chunk.clip, &options) {
                    Ok(transcript) if transcript.text.trim().is_empty() => continue,
                    Ok(transcript) => Ok(ChunkTranscript {
                        source,
                        start: chunk.start,
                        end,
                        transcript,
                    }),
                    Err(error) => Err(error),
                };
                // Nobody is listening once the worker is dropped.
                if publish.send(result).is_err() {
                    break;
                }
            }
            asr
        });
        Self {
            jobs,
            results,
            cancelled,
            thread,
        }
    }

    /// Queues a chunk and returns at once.
    pub fn submit(&self, source: S, chunk: LiveChunk) -> Result<(), PipelineError> {
        self.jobs.send((source, chunk)).map_err(|_| stopped())
    }

    /// The chunks finished since the last poll. Never waits.
    pub fn poll(&self) -> Vec<ChunkResult<S>> {
        self.results.try_iter().collect()
    }

    /// Transcribes everything queued, then hands back the rest of the results and the engine.
    pub fn finish(self) -> Result<Finished<S>, PipelineError> {
        let Self {
            jobs,
            results,
            thread,
            ..
        } = self;
        drop(jobs);
        let asr = thread.join().map_err(|_| stopped())?;
        Ok((results.try_iter().collect(), asr))
    }

    /// Cancel: drops what is queued and returns the engine once the running chunk ends.
    pub fn cancel(self) -> Result<Box<dyn Asr>, PipelineError> {
        self.cancelled.store(true, Ordering::Relaxed);
        drop(self.jobs);
        self.thread.join().map_err(|_| stopped())
    }
}

fn stopped() -> PipelineError {
    PipelineError::Engine("the ASR worker stopped".into())
}
