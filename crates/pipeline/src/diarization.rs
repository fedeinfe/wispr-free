//! Speaker diarization through sherpa-onnx (ADR-0003): pyannote segmentation finds the turns,
//! a speaker embedding model tells the voices apart, and clustering groups them. Behind the
//! `diarization` feature because sherpa-onnx fetches prebuilt native libraries at build time.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractorConfig,
};
use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};

use crate::{Diarizer, PipelineError, SpeakerTurn};

/// Cosine distance under which two turns are the same voice. Measured on ERes2Net with three
/// voices over 30 minutes: 0.5 finds 26 speakers, 0.7 finds 12, 0.9 finds 5, 1.1 finds the 3.
const CLUSTER_THRESHOLD: f32 = 1.1;
/// How far each 10 s window moves, as a fraction of the window.
const WINDOW_SHIFT: f32 = 0.1;
/// Turns shorter than this are dropped: too little audio to tell a voice from.
const MIN_TURN_SECONDS: f32 = 0.3;
/// A pause shorter than this doesn't end a turn.
const MIN_PAUSE_SECONDS: f32 = 0.5;

/// Where the two models are, as the catalog installs them.
#[derive(Debug, Clone)]
pub struct DiarizationConfig {
    pub segmentation: PathBuf,
    pub embedding: PathBuf,
    /// Bigger is faster and coarser.
    pub window_shift: f32,
    /// Bigger merges more voices.
    pub cluster_threshold: f32,
}

impl DiarizationConfig {
    pub fn new(segmentation: impl Into<PathBuf>, embedding: impl Into<PathBuf>) -> Self {
        Self {
            segmentation: segmentation.into(),
            embedding: embedding.into(),
            window_shift: WINDOW_SHIFT,
            cluster_threshold: CLUSTER_THRESHOLD,
        }
    }
}

pub struct SherpaDiarizer {
    engine: OfflineSpeakerDiarization,
}

impl SherpaDiarizer {
    pub fn new(config: &DiarizationConfig) -> Result<Self, PipelineError> {
        let threads = std::thread::available_parallelism().map_or(4, |cores| cores.get().min(8));
        let engine = OfflineSpeakerDiarization::create(&OfflineSpeakerDiarizationConfig {
            segmentation: OfflineSpeakerSegmentationModelConfig {
                pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                    model: Some(model_path(&config.segmentation)?),
                    window_shift_ratio: config.window_shift,
                },
                num_threads: threads as i32,
                ..OfflineSpeakerSegmentationModelConfig::default()
            },
            embedding: SpeakerEmbeddingExtractorConfig {
                model: Some(model_path(&config.embedding)?),
                num_threads: threads as i32,
                ..SpeakerEmbeddingExtractorConfig::default()
            },
            clustering: FastClusteringConfig {
                threshold: config.cluster_threshold,
                ..FastClusteringConfig::default()
            },
            min_duration_on: MIN_TURN_SECONDS,
            min_duration_off: MIN_PAUSE_SECONDS,
        })
        .ok_or_else(|| PipelineError::Engine("the diarization models failed to load".into()))?;
        if engine.sample_rate() != SAMPLE_RATE_HZ as i32 {
            return Err(PipelineError::Engine(format!(
                "diarization runs at {} Hz, not {SAMPLE_RATE_HZ}",
                engine.sample_rate()
            )));
        }
        Ok(Self { engine })
    }
}

impl Diarizer for SherpaDiarizer {
    fn diarize(&mut self, clip: &AudioClip) -> Result<Vec<SpeakerTurn>, PipelineError> {
        if clip.samples().is_empty() {
            return Ok(Vec::new());
        }
        let result = self
            .engine
            .process(clip.samples())
            .ok_or_else(|| PipelineError::Engine("diarization returned no result".into()))?;
        Ok(result
            .sort_by_start_time()
            .iter()
            .map(|segment| SpeakerTurn {
                start: Duration::from_secs_f32(segment.start.max(0.0)),
                end: Duration::from_secs_f32(segment.end.max(0.0)),
                speaker: segment.speaker.max(0) as u32,
            })
            .collect())
    }
}

fn model_path(path: &Path) -> Result<String, PipelineError> {
    if !path.is_file() {
        return Err(PipelineError::ModelMissing(path.display().to_string()));
    }
    Ok(path.to_string_lossy().into_owned())
}
