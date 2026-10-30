//! Runs the fixture through the real pipeline on the tier's real models: live transcription, as
//! fast as it can go or at real speed, then diarization, refine and summary. Reports what the
//! ticket asks for: wall-clock time, memory, and how much of the meeting the summary got right.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use wf_core::audio::FRAME_SAMPLES;
use wf_meetings::{
    Channel, LiveEngines, LiveEvent, LiveLog, LiveOptions, LiveTranscript, Meeting, MeetingEngines,
    MeetingPlan, MeetingStore, MeetingWriter, ProcessOptions, ProcessProgress, Segment, SpeakerId,
    Stage, StereoFrame, Summary, SummaryMode, SummaryOptions, TimelineFrame, Transcript, Turn,
    finish_meeting, generate_summary, stable_turns,
};
use wf_models::{Catalog, ModelEntry, ModelFile, Tier};
use wf_pipeline::diarization::{DiarizationConfig, SherpaDiarizer};
use wf_pipeline::llama::{LlamaConfig, LlamaEngine};
use wf_pipeline::llm_host::{KeepLoaded, LlmHost};
use wf_pipeline::sherpa::SherpaAsr;
use wf_pipeline::silero::SileroVad;
use wf_pipeline::whisper::WhisperAsr;
use wf_pipeline::{Asr, Diarizer, Llm, LlmPrompt, PipelineError, Vad};

use crate::meeting_eval::{MeetingScript, Truth, score_summary, word_error_rate};
use crate::meeting_fixture::read_wav;

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub fixture: PathBuf,
    /// A folder with the tier's models: `<id>/<file>` or plain `<file>`.
    pub models: PathBuf,
    pub out: PathBuf,
    pub tier: Tier,
    /// Pace the audio like a real meeting. Takes as long as the fixture.
    pub realtime: bool,
}

/// The tier's real engines, each loaded when its stage starts.
pub struct RealEngines {
    models: PathBuf,
    catalog: Catalog,
    tier: Tier,
}

impl RealEngines {
    pub fn new(models: &Path, tier: Tier) -> Self {
        Self {
            models: models.to_owned(),
            catalog: Catalog::bundled(),
            tier,
        }
    }

    fn file<K>(&self, model: &ModelEntry<K>, file: &ModelFile) -> PathBuf {
        let nested = self.models.join(&model.id).join(&file.name);
        if nested.exists() {
            nested
        } else {
            self.models.join(&file.name)
        }
    }

    pub fn llm_file(&self) -> Result<PathBuf, PipelineError> {
        let preset = self.catalog.preset(self.tier);
        let id = preset
            .llm_model
            .as_deref()
            .ok_or_else(|| missing("no LLM"))?;
        let model = self.catalog.model(id).ok_or_else(|| missing(id))?;
        Ok(self.file(model, &model.files[0]))
    }
}

fn missing(what: &str) -> PipelineError {
    PipelineError::ModelMissing(what.to_owned())
}

impl MeetingEngines for RealEngines {
    fn asr(&mut self) -> Result<Box<dyn Asr>, PipelineError> {
        let id = &self.catalog.preset(self.tier).asr_model;
        let model = self.catalog.model(id).ok_or_else(|| missing(id))?;
        match model.engine {
            wf_models::Engine::SherpaOnnx => {
                Ok(Box::new(SherpaAsr::new(&self.models.join(&model.id))?))
            }
            _ => Ok(Box::new(WhisperAsr::new(
                &self.file(model, &model.files[0]),
            )?)),
        }
    }

    fn vad(&mut self) -> Result<Box<dyn Vad>, PipelineError> {
        let id = self.catalog.preset(self.tier).vad_model.as_deref();
        let id = id.ok_or_else(|| missing("no VAD"))?;
        let model = self.catalog.model(id).ok_or_else(|| missing(id))?;
        Ok(Box::new(SileroVad::new(
            &self.file(model, &model.files[0]),
        )?))
    }

    fn diarizer(&mut self) -> Result<Box<dyn Diarizer>, PipelineError> {
        let models = self.catalog.speaker_models_for_tier(self.tier);
        let [segmentation, embedding] = models.as_slice() else {
            return Err(missing("speaker models"));
        };
        let config = DiarizationConfig::new(
            self.file(segmentation, &segmentation.files[0]),
            self.file(embedding, &embedding.files[0]),
        );
        Ok(Box::new(SherpaDiarizer::new(&config)?))
    }

    fn llm(&mut self) -> Result<Box<dyn Llm>, PipelineError> {
        let config = LlamaConfig::new(self.llm_file()?);
        Ok(Box::new(LlmHost::spawn(
            move || Ok(Box::new(LlamaEngine::load(&config)?) as Box<dyn Llm>),
            KeepLoaded::Always,
        )))
    }
}

/// Watches the process's resident memory from a thread of its own.
struct MemoryWatch {
    peak_kb: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl MemoryWatch {
    fn start() -> Self {
        let peak_kb = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (peak, stopped) = (peak_kb.clone(), stop.clone());
        std::thread::spawn(move || {
            let pid = std::process::id().to_string();
            while !stopped.load(Ordering::Relaxed) {
                let output = std::process::Command::new("ps")
                    .args(["-o", "rss=", "-p", &pid])
                    .output();
                if let Ok(output) = output
                    && let Ok(rss) = String::from_utf8_lossy(&output.stdout).trim().parse()
                {
                    peak.fetch_max(rss, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        });
        Self { peak_kb, stop }
    }

    /// The peak since the last call, in MB.
    fn take_mb(&self) -> u64 {
        self.peak_kb.swap(0, Ordering::Relaxed) / 1_024
    }
}

impl Drop for MemoryWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct LiveReport {
    wall: Duration,
    drain: Duration,
    frames_missed: u64,
    worst_lag: Duration,
    lags: Vec<Duration>,
}

/// Plays the fixture into a recording and a live transcript, like `MeetingRecorder` does.
fn record(
    meeting: &mut Meeting,
    engines: &mut RealEngines,
    you: &[f32],
    others: &[f32],
    realtime: bool,
) -> anyhow::Result<LiveReport> {
    let part = meeting.begin_part()?;
    let live_engines = LiveEngines {
        asr: engines.asr()?,
        you_vad: engines.vad()?,
        others_vad: engines.vad()?,
    };
    // Flat out, the audio outruns any model, which says nothing about a real meeting.
    let live_options = LiveOptions {
        max_backlog: if realtime {
            LiveOptions::default().max_backlog
        } else {
            Duration::MAX
        },
        ..LiveOptions::default()
    };
    let live = LiveTranscript::start(meeting, &part, live_engines, live_options)?;
    let mut writer = MeetingWriter::create(&part.path)?;
    let feed = live.feed();

    let mut report = LiveReport::default();
    let started = Instant::now();
    let frames = you.len().min(others.len()) / FRAME_SAMPLES;
    for n in 0..frames {
        let at = n * FRAME_SAMPLES;
        let frame = StereoFrame {
            you: you[at..at + FRAME_SAMPLES].to_vec(),
            others: others[at..at + FRAME_SAMPLES].to_vec(),
        };
        writer.write_frame(&frame.you, &frame.others)?;
        let frame = TimelineFrame {
            index: n as u64,
            frame,
        };
        if realtime {
            // The recorder never waits for this channel.
            if feed.try_send(frame).is_err() {
                report.frames_missed += 1;
            }
            let due = started + Duration::from_millis(40 * (n as u64 + 1));
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
            for event in live.events().try_iter() {
                if let LiveEvent::Segment(segment) = event {
                    let heard_at = Duration::from_millis(segment.end_ms);
                    let lag = started.elapsed().saturating_sub(heard_at);
                    report.worst_lag = report.worst_lag.max(lag);
                    report.lags.push(lag);
                }
            }
        } else {
            feed.send(frame).context("the live transcript stopped")?;
        }
    }
    let length = writer.finish()?;
    report.wall = started.elapsed();
    drop(feed);
    let draining = Instant::now();
    live.finish()?;
    report.drain = draining.elapsed();
    meeting.end_part(length)?;
    Ok(report)
}

/// Which voice each diarized speaker is, by overlap with the truth, and how much of the Others
/// channel's speech that places correctly.
fn speaker_match(
    transcript: &Transcript,
    truth: &Truth,
    you: &str,
) -> (BTreeMap<SpeakerId, String>, f64) {
    let mut overlap: BTreeMap<(SpeakerId, &str), u64> = BTreeMap::new();
    let mut total = 0;
    for segment in transcript
        .segments
        .iter()
        .filter(|s| s.channel == Channel::Others)
    {
        for turn in truth.turns.iter().filter(|t| t.who != you) {
            let shared = turn
                .end_ms
                .min(segment.end_ms)
                .saturating_sub(turn.start_ms.max(segment.start_ms));
            if shared > 0 {
                total += shared;
                if let Some(speaker) = segment.speaker {
                    *overlap.entry((speaker, turn.who.as_str())).or_default() += shared;
                }
            }
        }
    }
    let mut best: BTreeMap<SpeakerId, (&str, u64)> = BTreeMap::new();
    for (&(speaker, who), &time) in &overlap {
        let entry = best.entry(speaker).or_insert((who, 0));
        if time > entry.1 {
            *entry = (who, time);
        }
    }
    let right: u64 = best.values().map(|&(_, time)| time).sum();
    let names = best
        .into_iter()
        .map(|(speaker, (who, _))| (speaker, who.to_owned()))
        .collect();
    (names, right as f64 / total.max(1) as f64)
}

fn channel_text(segments: &[Segment], channel: Channel) -> String {
    let lines: Vec<&str> = segments
        .iter()
        .filter(|s| s.channel == channel)
        .map(|s| s.text.as_str())
        .collect();
    lines.join(" ")
}

fn wer_line(label: &str, transcript: &Transcript, truth: &Truth, you: &str) -> String {
    let said = |mine: bool| -> String {
        let lines: Vec<&str> = truth
            .turns
            .iter()
            .filter(|t| (t.who == you) == mine)
            .map(|t| t.text.as_str())
            .collect();
        lines.join(" ")
    };
    format!(
        "{label}: {} segments, WER you {:.1}%, others {:.1}%, gaps {}",
        transcript.segments.len(),
        100.0
            * word_error_rate(
                &said(true),
                &channel_text(&transcript.segments, Channel::You)
            ),
        100.0
            * word_error_rate(
                &said(false),
                &channel_text(&transcript.segments, Channel::Others)
            ),
        transcript.gaps.len(),
    )
}

/// Records every prompt and reply, to read while tuning prompts.
struct LoggedLlm {
    inner: Box<dyn Llm>,
    path: PathBuf,
}

impl Llm for LoggedLlm {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        let started = Instant::now();
        let reply = self.inner.generate(prompt, on_token);
        let mut log = format!(
            "\n===== USER ({:?})\n{}\n===== REPLY\n",
            started.elapsed(),
            prompt.user
        );
        match &reply {
            Ok(text) => log.push_str(text),
            Err(error) => log.push_str(&error.to_string()),
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path);
        if let Ok(mut file) = file {
            let _ = std::io::Write::write_all(&mut file, log.as_bytes());
        }
        reply
    }
}

/// Regenerates the newest meeting's summary and scores it, for tuning the prompts.
pub fn resummarize(options: &RunOptions) -> anyhow::Result<String> {
    let script = MeetingScript::bundled()?;
    let store = MeetingStore::new(&options.out);
    let id = store
        .list()?
        .first()
        .context("no meeting there")?
        .id
        .clone();
    let mut meeting = store.open(&id)?;
    let mut engines = RealEngines::new(&options.models, options.tier);
    let path = options.out.join(&id).join("llm.log");
    let _ = std::fs::remove_file(&path);
    let mut llm = LoggedLlm {
        inner: engines.llm()?,
        path,
    };
    let started = Instant::now();
    let summary = generate_summary(
        &mut meeting,
        &mut llm,
        SummaryMode::Regenerate,
        &SummaryOptions::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )?;
    let summary = meeting.summary()?.unwrap_or(summary);
    Ok(render_summary(&summary, &script, started.elapsed()))
}

/// Summarizes the holdout transcript. There's no audio, so it goes straight into a live log with
/// the labels diarization would have left, 8 s to a turn.
pub fn holdout(options: &RunOptions) -> anyhow::Result<String> {
    let script = MeetingScript::holdout()?;
    let store = MeetingStore::new(&options.out);
    let mut meeting = store.create()?;
    let mut log = LiveLog::open(&meeting)?;
    for (n, (who, text)) in script.turns().iter().enumerate() {
        let start_ms = n as u64 * 8_000;
        let speaker = who.strip_prefix('S').and_then(|n| n.parse().ok());
        log.segment(&Segment {
            channel: if speaker.is_some() {
                Channel::Others
            } else {
                Channel::You
            },
            speaker: speaker.map(SpeakerId),
            start_ms,
            end_ms: start_ms + 7_000,
            text: text.clone(),
        })?;
    }
    let mut engines = RealEngines::new(&options.models, options.tier);
    let id = meeting.record().id.clone();
    let mut llm = LoggedLlm {
        inner: engines.llm()?,
        path: options.out.join(&id).join("llm.log"),
    };
    let started = Instant::now();
    let summary = generate_summary(
        &mut meeting,
        &mut llm,
        SummaryMode::Regenerate,
        &SummaryOptions::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )?;
    Ok(render_summary(&summary, &script, started.elapsed()))
}

fn render_summary(summary: &Summary, script: &MeetingScript, spent: Duration) -> String {
    let mut report = format!(
        "{:.1} s, partial {}\nTITLE {:?}\nTLDR {}\n",
        spent.as_secs_f64(),
        summary.partial,
        summary.title,
        summary.tldr
    );
    for item in &summary.key_points {
        let _ = writeln!(report, "key point: {}", item.text);
    }
    report.push_str(&score_summary(script, summary).render());
    report
}

/// Runs diarization alone on the newest meeting's audio, for choosing its settings.
pub fn rediarize(
    options: &RunOptions,
    threshold: Option<f32>,
    shift: Option<f32>,
    embedding: Option<PathBuf>,
) -> anyhow::Result<String> {
    let script = MeetingScript::bundled()?;
    let truth: Truth = serde_json::from_slice(&std::fs::read(options.fixture.join("truth.json"))?)?;
    let store = MeetingStore::new(&options.out);
    let id = store
        .list()?
        .first()
        .context("no meeting there")?
        .id
        .clone();
    let audio = store.open(&id)?.audio()?;
    let engines = RealEngines::new(&options.models, options.tier);
    let models = engines.catalog.speaker_models_for_tier(options.tier);
    let mut config = DiarizationConfig::new(
        engines.file(models[0], &models[0].files[0]),
        engines.file(models[1], &models[1].files[0]),
    );
    if let Some(threshold) = threshold {
        config.cluster_threshold = threshold;
    }
    if let Some(shift) = shift {
        config.window_shift = shift;
    }
    if let Some(embedding) = embedding {
        config.embedding = embedding;
    }
    let started = Instant::now();
    let turns = SherpaDiarizer::new(&config)?.diarize(&audio.others)?;
    let spent = started.elapsed();
    let turns = stable_turns(&[], &turns);
    let (speakers, placed) = turn_match(&turns, &truth, &script.you);
    Ok(format!(
        "threshold {} shift {} embedding {}: {:.1} s, {} turns, {speakers} speakers, {:.1}% right",
        config.cluster_threshold,
        config.window_shift,
        config.embedding.display(),
        spent.as_secs_f64(),
        turns.len(),
        100.0 * placed
    ))
}

/// How many speakers came out, and how much of the Others channel's true speech is on the right
/// one, each cluster counting as the voice it overlaps most.
fn turn_match(turns: &[Turn], truth: &Truth, you: &str) -> (usize, f64) {
    let mut overlap: BTreeMap<(SpeakerId, &str), u64> = BTreeMap::new();
    let mut total = 0;
    for turn in truth.turns.iter().filter(|t| t.who != you) {
        total += turn.end_ms - turn.start_ms;
        for found in turns {
            let shared = turn
                .end_ms
                .min(found.end_ms)
                .saturating_sub(turn.start_ms.max(found.start_ms));
            if shared > 0 {
                *overlap
                    .entry((found.speaker, turn.who.as_str()))
                    .or_default() += shared;
            }
        }
    }
    let mut best: BTreeMap<SpeakerId, u64> = BTreeMap::new();
    for (&(speaker, _), &time) in &overlap {
        let entry = best.entry(speaker).or_default();
        *entry = (*entry).max(time);
    }
    let right: u64 = best.values().sum();
    (best.len(), right as f64 / total.max(1) as f64)
}

pub fn run(options: &RunOptions) -> anyhow::Result<String> {
    let script = MeetingScript::bundled()?;
    let truth: Truth = serde_json::from_slice(&std::fs::read(options.fixture.join("truth.json"))?)?;
    let you = read_wav(&options.fixture.join("you.wav"))?;
    let others = read_wav(&options.fixture.join("others.wav"))?;

    let catalog = Catalog::bundled();
    let preset = catalog.preset(options.tier);
    let plan = MeetingPlan {
        refine: preset.meeting_refine,
        diarize: preset.diarization,
        summarize: preset.llm_model.is_some(),
        keep_audio: true,
    };

    let mut report = format!(
        "tier {:?}: ASR {}, LLM {:?}, audio {:.1} min\n",
        options.tier,
        preset.asr_model,
        preset.llm_model,
        truth.length_ms as f64 / 60_000.0
    );
    let store = MeetingStore::new(&options.out);
    let mut meeting = store.create()?;
    let mut engines = RealEngines::new(&options.models, options.tier);
    let memory = MemoryWatch::start();
    let everything = Instant::now();

    let live = record(&mut meeting, &mut engines, &you, &others, options.realtime)?;
    drop((you, others));
    let _ = writeln!(
        report,
        "live ({}): {:.1} s + {:.2} s draining, peak RSS {} MB",
        if options.realtime {
            "real time"
        } else {
            "flat out"
        },
        live.wall.as_secs_f64(),
        live.drain.as_secs_f64(),
        memory.take_mb()
    );
    if options.realtime {
        let mut lags = live.lags.clone();
        lags.sort();
        let middle = lags.get(lags.len() / 2).copied().unwrap_or_default();
        let _ = writeln!(
            report,
            "live lag (speech end to text shown): median {:.2} s, worst {:.2} s, frames missed {}",
            middle.as_secs_f64(),
            live.worst_lag.as_secs_f64(),
            live.frames_missed
        );
    }
    let _ = writeln!(
        report,
        "{}",
        wer_line("live", &meeting.read_live()?, &truth, &script.you)
    );

    let mut stage_started = Instant::now();
    let mut current: Option<Stage> = None;
    let mut timings = Vec::new();
    let cancel = AtomicBool::new(false);
    finish_meeting(
        &mut meeting,
        plan,
        &mut engines,
        &ProcessOptions::default(),
        &cancel,
        &mut |progress| {
            if let ProcessProgress::Stage(stage) = progress {
                if let Some(previous) = current.replace(stage) {
                    timings.push((previous, stage_started.elapsed(), memory.take_mb()));
                }
                stage_started = Instant::now();
            }
        },
    )?;
    if let Some(last) = current {
        timings.push((last, stage_started.elapsed(), memory.take_mb()));
    }
    for (stage, time, peak) in &timings {
        let _ = writeln!(
            report,
            "{stage:?}: {:.1} s, peak RSS {peak} MB",
            time.as_secs_f64()
        );
    }
    let _ = writeln!(report, "stages: {:?}", meeting.record().stages);
    let _ = writeln!(report, "total: {:.1} s", everything.elapsed().as_secs_f64());

    let transcript = meeting.transcript()?;
    let _ = writeln!(
        report,
        "{}",
        wer_line("final", &transcript, &truth, &script.you)
    );
    let (names, placed) = speaker_match(&transcript, &truth, &script.you);
    let _ = writeln!(
        report,
        "speakers: {:?}, {:.1}% of Others speech on the right voice",
        names,
        100.0 * placed
    );
    for (speaker, name) in &names {
        meeting.rename_speaker(*speaker, name)?;
    }

    let id = meeting.record().id.clone();
    let names = &meeting.record().speakers;
    std::fs::write(
        options.out.join(&id).join("transcript.txt"),
        transcript.lines(names).join("\n"),
    )?;
    match meeting.summary()? {
        Some(summary) => {
            let _ = writeln!(
                report,
                "\nTITLE {:?}\nTLDR {}\npartial {}\n",
                summary.title, summary.tldr, summary.partial
            );
            for item in &summary.key_points {
                let _ = writeln!(report, "key point: {}", item.text);
            }
            let _ = writeln!(report);
            report.push_str(&score_summary(&script, &summary).render());
        }
        None => report.push_str("no summary\n"),
    }
    let _ = writeln!(report, "\nmeeting {}", options.out.join(&id).display());
    Ok(report)
}
