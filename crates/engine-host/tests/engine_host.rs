//! The engine host binary, in a real child process.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use wf_dictation::Engines;
use wf_dictation::plan::{AsrModel, DiarizerModel, LlmModel};
use wf_dictation::remote::ChildEngines;
use wf_pipeline::PipelineError;

fn engines() -> ChildEngines {
    ChildEngines::new(PathBuf::from(env!("CARGO_BIN_EXE_wf-engine-host")))
}

#[test]
fn a_missing_model_comes_back_as_model_missing_from_every_engine() {
    let engines = engines();
    let gone = Path::new("/no/such/model");

    assert!(matches!(
        engines.vad(gone).map(|_| ()),
        Err(PipelineError::ModelMissing(_))
    ));
    assert!(matches!(
        engines.asr(&AsrModel::Whisper(gone.to_owned())).map(|_| ()),
        Err(PipelineError::ModelMissing(_))
    ));
    assert!(matches!(
        engines.asr(&AsrModel::Sherpa(gone.to_owned())).map(|_| ()),
        Err(PipelineError::ModelMissing(_))
    ));
    let model = LlmModel {
        file: gone.to_owned(),
        in_ram: false,
    };
    assert!(matches!(
        engines.llm(&model).map(|_| ()),
        Err(PipelineError::ModelMissing(_))
    ));
    let model = DiarizerModel {
        segmentation: gone.to_owned(),
        embedding: gone.to_owned(),
    };
    assert!(matches!(
        engines.diarizer(&model).map(|_| ()),
        Err(PipelineError::ModelMissing(_))
    ));
}

#[test]
fn a_host_that_cannot_start_is_an_error() {
    let engines = ChildEngines::new(PathBuf::from("/no/such/binary"));

    assert!(matches!(
        engines.vad(Path::new("model")).map(|_| ()),
        Err(PipelineError::Engine(_))
    ));
}

#[test]
fn a_host_starts_quickly_enough_to_run_on_key_down() {
    let started = Instant::now();

    let _ = engines().vad(Path::new("/no/such/model")).map(|_| ());

    assert!(started.elapsed() < Duration::from_secs(5));
}
