//! The refine pass (spec §4): the whole meeting transcribed again, with the time to do it well.
//! Chunks are cut at speaker changes on the Others channel, so each segment has one voice.
//! A gap in the live transcript disappears here without being looked at.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use wf_core::audio::{AudioClip, FRAME_SAMPLES, SAMPLE_RATE_HZ};
use wf_pipeline::language::SteadyLanguage;
use wf_pipeline::vad::{LiveVad, VadOptions};
use wf_pipeline::{Asr, AsrOptions, PipelineError, Vad};

use crate::diarize::{Turn, assign_speakers, split_at_turns};
use crate::reader::MeetingAudio;
use crate::store::RefinedTranscript;
use crate::transcript::{Gap, GapReason, Segment, SpeakerId, Transcript};
use crate::{Channel, Error};

/// Shorter than live: citations get a finer place to land, and nobody is waiting on a pause.
const MIN_CHUNK: Duration = Duration::from_secs(4);
/// How much of the meeting's speech decides its language. Whisper reads no further than this.
const LANGUAGE_SPEECH: Duration = Duration::from_secs(30);
/// A pause this long ends a chunk however short it is. Without it one person's turns would join
/// up across whatever happened in between, and the text would land at the wrong time.
const CHUNK_PAUSE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq)]
pub struct RefineOptions {
    pub vad: VadOptions,
    pub asr: AsrOptions,
}

impl Default for RefineOptions {
    fn default() -> Self {
        Self {
            vad: VadOptions {
                min_chunk: MIN_CHUNK,
                // Here this doesn't stop anything: it's the signal to close the chunk.
                auto_stop: Some(CHUNK_PAUSE),
                ..VadOptions::default()
            },
            asr: AsrOptions::default(),
        }
    }
}

/// A stretch of one channel to transcribe, on the meeting's timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Job {
    channel: Channel,
    start_ms: u64,
    end_ms: u64,
    speaker: Option<SpeakerId>,
}

/// Transcribes both channels again. Text the model loses on a chunk comes from `live`. Fails
/// only if the engine can't do any of it, so a broken model can't replace a good transcript
/// with an empty one.
pub fn refine_transcript(
    audio: &MeetingAudio,
    turns: &[Turn],
    live: &Transcript,
    engines: (&mut dyn Asr, &mut dyn Vad),
    options: &RefineOptions,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<RefinedTranscript, Error> {
    let (asr, vad) = engines;
    let total_ms = duration_ms(audio);
    let mut jobs = speech_jobs(&audio.you, 0, Channel::You, &[], vad, options)?;
    jobs.extend(speech_jobs(
        &audio.others,
        0,
        Channel::Others,
        turns,
        vad,
        options,
    )?);
    jobs.sort_by_key(|job| (job.start_ms, job.channel));

    let mut transcript = Transcript::default();
    let mut failure = None;
    let mut failed = 0;
    let mut language = SteadyLanguage::default();
    settle_language(audio, &jobs, asr, options, &mut language);
    let job_count = jobs.len();
    for job in jobs {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        match transcribe(audio, job, asr, options, &mut language) {
            Ok(Some(segment)) => transcript.segments.push(segment),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!("a refine chunk failed: {error}");
                failed += 1;
                failure = Some(error);
                keep_live(job, live, &mut transcript);
            }
        }
        on_progress(job.end_ms.min(total_ms), total_ms);
    }
    if let Some(error) = failure
        && failed == job_count
    {
        return Err(error.into());
    }
    on_progress(total_ms, total_ms);
    transcript.sort();
    Ok(RefinedTranscript {
        covers_ms: total_ms,
        transcript,
    })
}

/// What a tier with no refine pass does: the live text stays, what it skipped is transcribed now
/// with the same model, and speakers go on by overlap if there are any.
pub fn fill_gaps(
    audio: &MeetingAudio,
    turns: &[Turn],
    live: &Transcript,
    engines: Option<(&mut dyn Asr, &mut dyn Vad)>,
    options: &RefineOptions,
    cancel: &AtomicBool,
) -> Result<RefinedTranscript, Error> {
    let mut transcript = live.clone();
    if let Some((asr, vad)) = engines {
        transcript.gaps.clear();
        let mut language = SteadyLanguage::default();
        for gap in &live.gaps {
            let clip = match gap.channel {
                Channel::You => &audio.you,
                Channel::Others => &audio.others,
            };
            let part = slice(clip, gap.start_ms, gap.end_ms);
            let jobs = speech_jobs(&part, gap.start_ms, gap.channel, turns, vad, options)?;
            settle_language(audio, &jobs, asr, options, &mut language);
            for job in jobs {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                match transcribe(audio, job, asr, options, &mut language) {
                    Ok(Some(segment)) => transcript.segments.push(segment),
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!("a gap could not be filled: {error}");
                        transcript.gaps.push(Gap {
                            reason: GapReason::Failed,
                            start_ms: job.start_ms,
                            end_ms: job.end_ms,
                            channel: job.channel,
                        });
                    }
                }
            }
        }
    }
    assign_speakers(&mut transcript.segments, turns);
    transcript.sort();
    Ok(RefinedTranscript {
        covers_ms: duration_ms(audio),
        transcript,
    })
}

/// Finds the speech in `clip`, which starts at `base_ms` on the meeting's timeline, and cuts it
/// at speaker changes.
fn speech_jobs(
    clip: &AudioClip,
    base_ms: u64,
    channel: Channel,
    turns: &[Turn],
    vad: &mut dyn Vad,
    options: &RefineOptions,
) -> Result<Vec<Job>, PipelineError> {
    let mut live = LiveVad::new(vad, options.vad.clone());
    let mut chunks = Vec::new();
    for frame in clip.samples().chunks(FRAME_SAMPLES) {
        let update = live.push(frame)?;
        chunks.extend(update.chunks);
        if update.auto_stop {
            chunks.extend(live.flush());
        }
    }
    chunks.extend(live.finish()?);

    let mut jobs = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let start_ms = base_ms + chunk.start.as_millis() as u64;
        let end_ms = start_ms + chunk.clip.duration().as_millis() as u64;
        for (start_ms, end_ms, speaker) in split_at_turns(start_ms, end_ms, turns) {
            jobs.push(Job {
                channel,
                start_ms,
                end_ms,
                speaker,
            });
        }
    }
    Ok(jobs)
}

/// Decides the language from the opening speech, so the first chunks don't have to guess. Failing
/// here leaves them to detect it themselves, and they'll report what's wrong.
fn settle_language(
    audio: &MeetingAudio,
    jobs: &[Job],
    asr: &mut dyn Asr,
    options: &RefineOptions,
    language: &mut SteadyLanguage,
) {
    let limit = (LANGUAGE_SPEECH.as_secs_f64() * f64::from(SAMPLE_RATE_HZ)) as usize;
    let mut speech = Vec::new();
    for job in jobs {
        if speech.len() >= limit {
            break;
        }
        speech.extend_from_slice(job_clip(audio, *job).samples());
    }
    speech.truncate(limit);
    if speech.is_empty() {
        return;
    }
    if let Err(error) = language.settle(asr, &AudioClip::new(speech), &options.asr) {
        tracing::warn!("couldn't tell the meeting's language: {error}");
    }
}

fn job_clip(audio: &MeetingAudio, job: Job) -> AudioClip {
    let clip = match job.channel {
        Channel::You => &audio.you,
        Channel::Others => &audio.others,
    };
    slice(clip, job.start_ms, job.end_ms)
}

fn transcribe(
    audio: &MeetingAudio,
    job: Job,
    asr: &mut dyn Asr,
    options: &RefineOptions,
    language: &mut SteadyLanguage,
) -> Result<Option<Segment>, PipelineError> {
    let transcript = language.transcribe(asr, &job_clip(audio, job), &options.asr)?;
    let text = transcript.text.trim();
    Ok((!text.is_empty()).then(|| Segment {
        channel: job.channel,
        speaker: job.speaker,
        start_ms: job.start_ms,
        end_ms: job.end_ms,
        text: text.to_owned(),
    }))
}

/// Falls back on what the live pass heard in the job's stretch, or marks the hole.
fn keep_live(job: Job, live: &Transcript, transcript: &mut Transcript) {
    let mut kept = false;
    for segment in &live.segments {
        let middle = segment.start_ms.midpoint(segment.end_ms);
        if segment.channel == job.channel && (job.start_ms..job.end_ms).contains(&middle) {
            transcript.segments.push(Segment {
                speaker: job.speaker,
                ..segment.clone()
            });
            kept = true;
        }
    }
    if !kept {
        transcript.gaps.push(Gap {
            channel: job.channel,
            start_ms: job.start_ms,
            end_ms: job.end_ms,
            reason: GapReason::Failed,
        });
    }
}

fn duration_ms(audio: &MeetingAudio) -> u64 {
    audio
        .you
        .duration()
        .max(audio.others.duration())
        .as_millis() as u64
}

fn slice(clip: &AudioClip, start_ms: u64, end_ms: u64) -> AudioClip {
    let samples = clip.samples();
    let at = |ms: u64| ((ms * u64::from(SAMPLE_RATE_HZ) / 1_000) as usize).min(samples.len());
    AudioClip::new(samples[at(start_ms)..at(end_ms)].to_vec())
}
