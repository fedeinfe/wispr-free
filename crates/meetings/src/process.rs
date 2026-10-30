//! What happens after the recording stops. The stages run in order, each saving its result
//! before the next one starts, and each drops its model before the next one loads. A stage that
//! fails is recorded and the rest still run on what exists.

use std::sync::atomic::{AtomicBool, Ordering};

use wf_pipeline::{Asr, Diarizer, Llm, PipelineError, Vad};

use crate::Error;
use crate::diarize::stable_turns;
use crate::reader::MeetingAudio;
use crate::refine::{RefineOptions, fill_gaps, refine_transcript};
use crate::store::{Meeting, Stage, StageState};
use crate::summary::{SummaryMode, SummaryOptions, SummaryProgress, generate_summary};

/// Which stages run. Comes from the tier (spec §6) and the Meetings settings: `refine` and
/// `diarize` from `TierPreset::meeting_refine` and `::diarization`, `summarize` from whether the
/// tier has an LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeetingPlan {
    pub refine: bool,
    pub diarize: bool,
    pub summarize: bool,
    /// `false` deletes the audio as soon as the transcript is final.
    pub keep_audio: bool,
}

/// Loads a stage's model when the stage starts. Dropping what it returns must free the model, so
/// a shared `LlmHost` should be handed out as a clone.
pub trait MeetingEngines {
    fn asr(&mut self) -> Result<Box<dyn Asr>, PipelineError>;
    fn vad(&mut self) -> Result<Box<dyn Vad>, PipelineError>;
    fn diarizer(&mut self) -> Result<Box<dyn Diarizer>, PipelineError>;
    fn llm(&mut self) -> Result<Box<dyn Llm>, PipelineError>;
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProcessOptions {
    pub refine: RefineOptions,
    pub summary: SummaryOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessProgress {
    Stage(Stage),
    Refining { done_ms: u64, total_ms: u64 },
    Summarizing(SummaryProgress),
}

/// Runs what `plan` asks for and records how each stage went in the meeting's record. Blocking,
/// often for minutes: run it on a worker thread. Only a disk failure or `cancel` comes back as an
/// error. After a resumed recording, call it again.
pub fn finish_meeting(
    meeting: &mut Meeting,
    plan: MeetingPlan,
    engines: &mut dyn MeetingEngines,
    options: &ProcessOptions,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(ProcessProgress),
) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    // The audio is only read if a stage needs it: 170 MB for 30 minutes.
    let live = meeting.transcript()?;
    let needs_audio = plan.refine || plan.diarize || !live.gaps.is_empty();
    let audio = if meeting.record().audio_deleted || !needs_audio {
        None
    } else {
        match meeting.audio() {
            Ok(audio) => Some(audio),
            Err(error) => {
                let reason = error.to_string();
                meeting.set_stage(Stage::Diarization, failed(reason.clone()))?;
                meeting.set_stage(Stage::Refine, failed(reason))?;
                None
            }
        }
    };

    on_progress(ProcessProgress::Stage(Stage::Diarization));
    diarize(meeting, plan, engines, audio.as_ref())?;
    on_progress(ProcessProgress::Stage(Stage::Refine));
    refine(
        meeting,
        plan,
        engines,
        audio.as_ref(),
        options,
        cancel,
        on_progress,
    )?;
    drop(audio);

    on_progress(ProcessProgress::Stage(Stage::Summary));
    summarize(meeting, plan, engines, options, cancel, on_progress)?;

    if !plan.keep_audio && !meeting.record().audio_deleted && meeting.transcript_is_final()? {
        meeting.delete_audio()?;
    }
    Ok(())
}

fn skipped(reason: &str) -> StageState {
    StageState::Skipped {
        reason: reason.to_owned(),
    }
}

fn failed(reason: String) -> StageState {
    StageState::Failed { reason }
}

fn diarize(
    meeting: &mut Meeting,
    plan: MeetingPlan,
    engines: &mut dyn MeetingEngines,
    audio: Option<&MeetingAudio>,
) -> Result<(), Error> {
    if !plan.diarize {
        return meeting.set_stage(Stage::Diarization, skipped("off"));
    }
    let Some(audio) = audio else {
        if meeting.record().audio_deleted {
            return meeting.set_stage(Stage::Diarization, skipped("the audio was deleted"));
        }
        return Ok(());
    };
    let found = engines
        .diarizer()
        .and_then(|mut diarizer| diarizer.diarize(&audio.others));
    match found {
        Ok(found) => {
            let turns = stable_turns(&meeting.turns()?, &found);
            meeting.save_turns(&turns)?;
            meeting.set_stage(Stage::Diarization, StageState::done())
        }
        Err(error) => {
            tracing::warn!("diarization failed: {error}");
            meeting.set_stage(Stage::Diarization, failed(error.to_string()))
        }
    }
}

fn refine(
    meeting: &mut Meeting,
    plan: MeetingPlan,
    engines: &mut dyn MeetingEngines,
    audio: Option<&MeetingAudio>,
    options: &ProcessOptions,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(ProcessProgress),
) -> Result<(), Error> {
    let Some(audio) = audio else {
        if meeting.record().audio_deleted {
            return meeting.set_stage(Stage::Refine, skipped("the audio was deleted"));
        }
        if !plan.refine {
            return meeting.set_stage(Stage::Refine, skipped("off"));
        }
        return Ok(());
    };
    let live = meeting.read_live()?;
    let turns = meeting.turns()?;
    if plan.refine {
        let result =
            load_for_refine(engines)
                .map_err(Error::from)
                .and_then(|(mut asr, mut vad)| {
                    refine_transcript(
                        audio,
                        &turns,
                        &live,
                        (&mut *asr, &mut *vad),
                        &options.refine,
                        cancel,
                        &mut |done_ms, total_ms| {
                            on_progress(ProcessProgress::Refining { done_ms, total_ms })
                        },
                    )
                });
        return match result {
            Ok(refined) => {
                meeting.save_refined(&refined)?;
                meeting.set_stage(Stage::Refine, StageState::done())
            }
            Err(Error::Cancelled) => Err(Error::Cancelled),
            Err(Error::Engine(error)) => {
                tracing::warn!("the refine pass failed: {error}");
                meeting.set_stage(Stage::Refine, failed(error.to_string()))
            }
            Err(error) => Err(error),
        };
    }

    let reason = "off";
    let filled = if live.gaps.is_empty() {
        fill_gaps(audio, &turns, &live, None, &options.refine, cancel)
    } else {
        match load_for_refine(engines) {
            Ok((mut asr, mut vad)) => fill_gaps(
                audio,
                &turns,
                &live,
                Some((&mut *asr, &mut *vad)),
                &options.refine,
                cancel,
            ),
            Err(error) => {
                tracing::warn!("the gaps can't be filled: {error}");
                fill_gaps(audio, &turns, &live, None, &options.refine, cancel)
            }
        }
    };
    let filled = filled?;
    if filled.transcript != live {
        meeting.save_refined(&filled)?;
    }
    meeting.set_stage(Stage::Refine, skipped(reason))
}

fn load_for_refine(
    engines: &mut dyn MeetingEngines,
) -> Result<(Box<dyn Asr>, Box<dyn Vad>), PipelineError> {
    Ok((engines.asr()?, engines.vad()?))
}

fn summarize(
    meeting: &mut Meeting,
    plan: MeetingPlan,
    engines: &mut dyn MeetingEngines,
    options: &ProcessOptions,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(ProcessProgress),
) -> Result<(), Error> {
    if !plan.summarize {
        return meeting.set_stage(Stage::Summary, skipped("off"));
    }
    if meeting.transcript()?.segments.is_empty() {
        return meeting.set_stage(Stage::Summary, skipped("nothing was said"));
    }
    let mut llm = match engines.llm() {
        Ok(llm) => llm,
        Err(error) => {
            tracing::warn!("the summary model can't load: {error}");
            return meeting.set_stage(Stage::Summary, failed(error.to_string()));
        }
    };
    let generated = generate_summary(
        meeting,
        &mut *llm,
        SummaryMode::Update,
        &options.summary,
        cancel,
        &mut |progress| on_progress(ProcessProgress::Summarizing(progress)),
    );
    match generated {
        Ok(_) | Err(Error::Engine(_)) => Ok(()),
        Err(error) => Err(error),
    }
}
