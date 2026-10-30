//! The refine pass (spec §4): the whole meeting transcribed again, with the time to do it well.
//! Chunks are cut at speaker changes on the Others channel, so each segment has one voice.
//! A gap in the live transcript disappears here without being looked at.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};
use wf_pipeline::vad::{LiveVad, VadOptions};
use wf_pipeline::{Asr, AsrOptions, PipelineError, Vad};

use crate::diarize::{Turn, assign_speakers, split_at_turns};
use crate::reader::MeetingAudio;
use crate::store::RefinedTranscript;
use crate::transcript::{Gap, GapReason, Segment, SpeakerId, Transcript};
use crate::{Channel, Error};

/// Shorter than live: citations get a finer place to land, and nobody is waiting on a pause.
const MIN_CHUNK: Duration = Duration::from_secs(4);
/// Language detection is unreliable on less audio than this, so a chunk this short uses the
/// language of the last longer one. A bilingual meeting still switches per chunk.
const MIN_DETECT: Duration = Duration::from_secs(3);
/// VAD reads the recording in blocks of this many samples: 10 s. In one go, sherpa would hold
/// every segment it closed until the end.
const VAD_BLOCK: usize = 160_000;

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
    let mut language = None;
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
        let mut language = None;
        for gap in &live.gaps {
            let clip = match gap.channel {
                Channel::You => &audio.you,
                Channel::Others => &audio.others,
            };
            let part = slice(clip, gap.start_ms, gap.end_ms);
            let jobs = speech_jobs(&part, gap.start_ms, gap.channel, turns, vad, options)?;
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
    for block in clip.samples().chunks(VAD_BLOCK) {
        chunks.extend(live.push(block)?.chunks);
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

fn transcribe(
    audio: &MeetingAudio,
    job: Job,
    asr: &mut dyn Asr,
    options: &RefineOptions,
    language: &mut Option<String>,
) -> Result<Option<Segment>, PipelineError> {
    let clip = match job.channel {
        Channel::You => slice(&audio.you, job.start_ms, job.end_ms),
        Channel::Others => slice(&audio.others, job.start_ms, job.end_ms),
    };
    let short = clip.duration() < MIN_DETECT;
    let transcript = if short && options.asr.language.is_none() {
        let pinned = AsrOptions {
            language: language.clone(),
            ..options.asr.clone()
        };
        asr.transcribe(&clip, &pinned)?
    } else {
        asr.transcribe(&clip, &options.asr)?
    };
    if !short && transcript.language.is_some() {
        language.clone_from(&transcript.language);
    }
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
