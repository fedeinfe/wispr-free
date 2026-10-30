//! The VAD stage (spec §3): decides which audio reaches ASR. Everything here is engine-free,
//! so it runs against any [`Vad`].

use std::ops::Range;
use std::time::Duration;

use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};

use crate::{Asr, AsrOptions, PipelineError, SpeechSegment, Transcript, Vad};

#[derive(Debug, Clone, PartialEq)]
pub struct VadOptions {
    /// Silence kept on each side of the speech, so soft word edges survive the trim.
    pub padding: Duration,
    /// A chunk closes at the first pause after it reaches this length. Speech is never cut
    /// mid-way, so a chunk can run longer when the speaker doesn't pause.
    pub min_chunk: Duration,
    /// Hands-free only: stop the dictation after this much silence. Silence before the first
    /// word counts too, so a dictation nobody speaks in doesn't hold the mic open.
    pub auto_stop: Option<Duration>,
}

impl Default for VadOptions {
    fn default() -> Self {
        Self {
            padding: Duration::from_millis(200),
            min_chunk: Duration::from_secs(15),
            auto_stop: None,
        }
    }
}

/// What ASR should hear from a finished recording. A silent recording has no chunks.
/// `None` is VAD switched off: the whole recording goes through.
pub fn speech_chunks(
    clip: &AudioClip,
    vad: Option<&mut (dyn Vad + '_)>,
    options: &VadOptions,
) -> Result<Vec<AudioClip>, PipelineError> {
    let samples = clip.samples();
    let Some(vad) = vad else {
        return Ok(if samples.is_empty() {
            Vec::new()
        } else {
            vec![clip.clone()]
        });
    };
    let mut chunker = Chunker::new(options);
    let mut ranges = Vec::new();
    for segment in vad.speech_segments(clip)? {
        ranges.extend(chunker.add(segment, samples.len()));
    }
    ranges.extend(chunker.close(samples.len()));
    Ok(ranges
        .into_iter()
        .map(|range| AudioClip::new(samples[range].to_vec()))
        .collect())
}

/// [`speech_chunks`] while the recording is still running, so ASR can start before the user
/// stops. Cancel = [`finish`](Self::finish) and drop the chunks.
pub struct LiveVad<'a> {
    vad: &'a mut dyn Vad,
    chunker: Chunker,
    auto_stop: Option<usize>,
    /// The recording from `pending_start` on: everything no chunk has taken yet.
    pending: Vec<f32>,
    pending_start: usize,
    received: usize,
    last_speech: usize,
}

/// A chunk of a running recording. `start` is where it begins, so a meeting can timestamp it.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveChunk {
    pub start: Duration,
    pub clip: AudioClip,
}

/// What one [`LiveVad::push`] produced.
#[derive(Debug, Default, PartialEq)]
pub struct LiveUpdate {
    /// Chunks ready for ASR, in order.
    pub chunks: Vec<LiveChunk>,
    /// The silence reached [`VadOptions::auto_stop`]. Stays set until speech resumes.
    pub auto_stop: bool,
}

impl<'a> LiveVad<'a> {
    /// The recording starts at the first [`push`](Self::push), so push the pre-roll first.
    pub fn new(vad: &'a mut dyn Vad, options: VadOptions) -> Self {
        Self {
            vad,
            chunker: Chunker::new(&options),
            auto_stop: options.auto_stop.map(to_samples),
            pending: Vec::new(),
            pending_start: 0,
            received: 0,
            last_speech: 0,
        }
    }

    pub fn push(&mut self, samples: &[f32]) -> Result<LiveUpdate, PipelineError> {
        self.pending.extend_from_slice(samples);
        self.received += samples.len();
        let mut chunks = Vec::new();
        for segment in self.vad.accept(samples)? {
            // Rewinds over the pause the detector needed to close the segment.
            self.last_speech = segment.end;
            if let Some(range) = self.chunker.add(segment, self.received) {
                chunks.push(self.take(range));
            }
        }
        if self.vad.is_speaking() {
            self.last_speech = self.received;
        }
        let silence = self.received - self.last_speech;
        Ok(LiveUpdate {
            chunks,
            auto_stop: self.auto_stop.is_some_and(|limit| silence >= limit),
        })
    }

    /// Closes the chunk being built, however short. For when the speaker has gone quiet and the
    /// words shouldn't wait for more speech to join them.
    pub fn flush(&mut self) -> Option<LiveChunk> {
        let range = self.chunker.close(self.received)?;
        Some(self.take(range))
    }

    /// Ends the recording and returns the chunks still to transcribe.
    pub fn finish(mut self) -> Result<Vec<LiveChunk>, PipelineError> {
        let mut chunks = Vec::new();
        for segment in self.vad.finish()? {
            if let Some(range) = self.chunker.add(segment, self.received) {
                chunks.push(self.take(range));
            }
        }
        if let Some(range) = self.chunker.close(self.received) {
            chunks.push(self.take(range));
        }
        Ok(chunks)
    }

    fn take(&mut self, range: Range<usize>) -> LiveChunk {
        let start = range.start - self.pending_start;
        let end = range.end - self.pending_start;
        let clip = AudioClip::new(self.pending[start..end].to_vec());
        self.pending.drain(..end);
        self.pending_start = range.end;
        LiveChunk {
            start: Duration::from_secs_f64(range.start as f64 / f64::from(SAMPLE_RATE_HZ)),
            clip,
        }
    }
}

/// Runs ASR over each chunk in order. `None` means nothing was said, so there is nothing to
/// clean or insert: no chunks (ASR never ran) or the engine heard no words.
pub fn transcribe_speech(
    chunks: impl IntoIterator<Item = AudioClip>,
    asr: &mut dyn Asr,
    options: &AsrOptions,
) -> Result<Option<Transcript>, PipelineError> {
    let mut text = String::new();
    let mut language = None;
    for chunk in chunks {
        let transcript = asr.transcribe(&chunk, options)?;
        let words = transcript.text.trim();
        if words.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(words);
        language = language.or(transcript.language);
    }
    Ok((!text.is_empty()).then_some(Transcript { text, language }))
}

/// Groups speech segments into chunks, as sample ranges of the recording.
struct Chunker {
    padding: usize,
    min_chunk: usize,
    open: Option<Range<usize>>,
    /// Where the last chunk ended; padding never reaches back past it.
    floor: usize,
}

impl Chunker {
    fn new(options: &VadOptions) -> Self {
        Self {
            padding: to_samples(options.padding),
            min_chunk: to_samples(options.min_chunk),
            open: None,
            floor: 0,
        }
    }

    /// Takes the next segment. Returns a chunk if this pause closes one; `available` is how
    /// much of the recording exists so far.
    fn add(&mut self, segment: SpeechSegment, available: usize) -> Option<Range<usize>> {
        let start = match &self.open {
            Some(open) => open.start,
            None => segment.start.saturating_sub(self.padding).max(self.floor),
        };
        let end = segment.end + self.padding;
        self.open = Some(start..end);
        if end - start >= self.min_chunk {
            self.close(available)
        } else {
            None
        }
    }

    fn close(&mut self, available: usize) -> Option<Range<usize>> {
        let open = self.open.take()?;
        let end = open.end.min(available);
        self.floor = end;
        Some(open.start..end)
    }
}

fn to_samples(duration: Duration) -> usize {
    (duration.as_secs_f64() * f64::from(SAMPLE_RATE_HZ)).round() as usize
}
