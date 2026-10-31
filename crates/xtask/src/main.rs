//! `cargo xtask eval --model model.gguf` scores LLM cleanup against `eval/cleanup.toml`.
//! Flags: `--rules-only` scores rule cleanup alone, `--dev` skips the holdout cases, `--cpu` keeps
//! every layer off the GPU, and `--no-language` withholds each case's language, which is what
//! cleanup gets from Parakeet on auto-detect. `cargo xtask bundle` is in [`xtask::bundle`],
//! `cargo xtask bridge` in [`xtask::bridge`], and `cargo xtask shots` and `cargo xtask footprint`
//! in [`xtask::dev_app`].
//!
//! `cargo xtask-meeting meeting-fixture --out dir` builds the 30-minute meeting with macOS `say`.
//! `cargo xtask-meeting meeting-run --fixture dir --models dir --out dir` runs it through every
//! stage on the Balanced tier and scores the summary. Flags: `--realtime` paces the audio like
//! a real meeting, `--tier lite|balanced|high` picks another tier.
//! `meeting-summary` and `meeting-diarize` redo one stage on the newest meeting in `--out`, for
//! tuning. `meeting-diarize` takes `--threshold`, `--shift` and `--embedding`. `meeting-holdout`
//! summarizes a transcript the prompts were never tuned on. `meeting-chat` puts the chat's
//! questions to the newest meeting and scores where the citations land, and takes `--context`.
//! `meeting-chat-score` scores the answers saved with it, and needs no model.
//!
//! `cargo xtask-meeting speech-fixture --out dir` builds the ASR benchmark's clips with macOS `say`.
//! `cargo xtask-meeting asr-bench --fixture dir --models dir --model id --out file.json` times one
//! ASR model over them and scores its WER. Flags: `--runs n`, `--filter text` keeps the clips whose
//! id has it, `--mode auto|pinned` tries one way of choosing the language, `--threads n` runs on
//! the CPU with that many threads, to stand in for a machine with no GPU.

use std::process::ExitCode;

use anyhow::{Context as _, bail};
use xtask::eval::{self, Report, RulesOnly, TARGET};
use xtask::{bridge, bundle, dev_app};

fn main() -> anyhow::Result<ExitCode> {
    let mut args = std::env::args().skip(1);
    let Some(task) = args.next() else {
        bail!(
            "usage: cargo xtask eval [--model model.gguf] [--rules-only] [--dev] [--cpu] \
             [--no-language]"
        );
    };
    match task.as_str() {
        "eval" => {}
        "bundle" => return bundle::run(args).map(|()| ExitCode::SUCCESS),
        "bridge" => return bridge::run().map(|()| ExitCode::SUCCESS),
        "shots" => return dev_app::shots(args).map(|()| ExitCode::SUCCESS),
        "footprint" => return dev_app::footprint().map(|()| ExitCode::SUCCESS),
        "signing-identity" => return bundle::create_identity().map(|()| ExitCode::SUCCESS),
        "meeting-fixture" | "meeting-run" | "meeting-summary" | "meeting-diarize"
        | "meeting-holdout" | "meeting-chat" | "meeting-chat-score" | "speech-fixture"
        | "asr-bench" => {
            return meeting_task(&task, args);
        }
        _ => bail!("unknown task {task:?}"),
    }

    let mut model = std::env::var("WF_LLM_MODEL").ok();
    let (mut rules_only, mut dev, mut cpu, mut no_language) = (false, false, false, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => model = Some(args.next().context("--model needs a path")?),
            "--rules-only" => rules_only = true,
            "--dev" => dev = true,
            "--cpu" => cpu = true,
            "--no-language" => no_language = true,
            other => bail!("unknown flag {other:?}"),
        }
    }

    let mut cases = eval::cases()?;
    if dev {
        cases.retain(|case| !case.holdout);
    }
    // The report still splits by the language really spoken.
    let spoken: Vec<_> = cases.iter().map(|case| case.language.clone()).collect();
    if no_language {
        for case in &mut cases {
            case.language = None;
        }
    }
    let mut report = if rules_only {
        eval::run(&cases, &mut RulesOnly)
    } else {
        let model = model.context("set --model or WF_LLM_MODEL to a GGUF file")?;
        eval_with_model(&cases, &model, cpu)?
    };
    for (result, language) in report.results.iter_mut().zip(spoken) {
        result.language = language;
    }
    print!("{}", report.render());
    Ok(if report.score() >= TARGET {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn meeting_task(task: &str, mut args: impl Iterator<Item = String>) -> anyhow::Result<ExitCode> {
    let mut flags = std::collections::HashMap::new();
    let mut realtime = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--realtime" => realtime = true,
            "--fixture" | "--models" | "--out" | "--tier" | "--threshold" | "--shift"
            | "--embedding" | "--context" | "--model" | "--runs" | "--filter" | "--mode"
            | "--threads" => {
                let value = args
                    .next()
                    .with_context(|| format!("{arg} needs a value"))?;
                flags.insert(arg, value);
            }
            other => bail!("unknown flag {other:?}"),
        }
    }
    let out = flags.get("--out").context("set --out to a folder")?;
    let out = std::path::Path::new(out);
    if task == "meeting-fixture" {
        let script = xtask::meeting_eval::MeetingScript::bundled()?;
        let truth = xtask::meeting_fixture::build(&script, out)?;
        println!(
            "{} turns, {:.1} min, in {}",
            truth.turns.len(),
            truth.length_ms as f64 / 60_000.0,
            out.display()
        );
        return Ok(ExitCode::SUCCESS);
    }
    if task == "speech-fixture" {
        let script = xtask::speech_fixture::SpeechScript::bundled()?;
        let recordings =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pipeline/tests/fixtures");
        let clips = xtask::speech_fixture::build(&script, &recordings, out)?;
        println!("{} clips in {}", clips.len(), out.display());
        return Ok(ExitCode::SUCCESS);
    }
    meeting_run(task, &flags, realtime, out)
}

#[cfg(feature = "meeting")]
fn meeting_run(
    task: &str,
    flags: &std::collections::HashMap<String, String>,
    realtime: bool,
    out: &std::path::Path,
) -> anyhow::Result<ExitCode> {
    use wf_models::Tier;
    use xtask::asr_bench::{self, Bench};
    use xtask::meeting_run::{RunOptions, chat, holdout, rediarize, resummarize, run};

    let path = |flag: &str| -> anyhow::Result<std::path::PathBuf> {
        Ok(flags
            .get(flag)
            .with_context(|| format!("set {flag} to a folder"))?
            .into())
    };
    let tier = match flags.get("--tier").map(String::as_str) {
        None | Some("balanced") => Tier::Balanced,
        Some("lite") => Tier::Lite,
        Some("high") => Tier::High,
        Some(other) => bail!("unknown tier {other:?}"),
    };
    let options = RunOptions {
        fixture: flags.get("--fixture").map(Into::into).unwrap_or_default(),
        models: path("--models")?,
        out: out.to_owned(),
        tier,
        realtime,
    };
    let number = |flag: &str| -> anyhow::Result<Option<f32>> {
        Ok(flags.get(flag).map(|text| text.parse()).transpose()?)
    };
    let report = match task {
        "asr-bench" => asr_bench::run(&Bench {
            fixture: options.fixture.clone(),
            models: options.models.clone(),
            model: flags.get("--model").context("set --model")?.clone(),
            out: out.to_owned(),
            runs: flags.get("--runs").map_or(Ok(3), |n| n.parse())?,
            filter: flags.get("--filter").cloned(),
            mode: flags.get("--mode").cloned(),
            cpu_threads: flags.get("--threads").map(|n| n.parse()).transpose()?,
        })?,
        "meeting-summary" => resummarize(&options)?,
        "meeting-holdout" => holdout(&options)?,
        // Scoring what's saved needs no model.
        "meeting-chat" | "meeting-chat-score" => {
            let context = flags
                .get("--context")
                .map(|text| text.parse())
                .transpose()?;
            chat(&options, task == "meeting-chat", context)?
        }
        "meeting-diarize" => rediarize(
            &options,
            number("--threshold")?,
            number("--shift")?,
            flags.get("--embedding").map(Into::into),
        )?,
        _ => run(&options)?,
    };
    println!("{report}");
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(feature = "meeting"))]
fn meeting_run(
    _task: &str,
    _flags: &std::collections::HashMap<String, String>,
    _realtime: bool,
    _out: &std::path::Path,
) -> anyhow::Result<ExitCode> {
    bail!("built without the `meeting` feature; run it through `cargo xtask-meeting`")
}

#[cfg(feature = "llama")]
fn eval_with_model(cases: &[eval::Case], model: &str, cpu: bool) -> anyhow::Result<Report> {
    use std::time::Instant;

    use wf_pipeline::llama::{LlamaConfig, LlamaEngine};
    use wf_pipeline::llm_host::{KeepLoaded, LlmHost};
    use wf_pipeline::{Llm, LlmPrompt};

    let mut config = LlamaConfig::new(model);
    if cpu {
        config.gpu_layers = 0;
    }
    let mut host = LlmHost::spawn(
        move || Ok(Box::new(LlamaEngine::load(&config)?) as Box<dyn Llm>),
        KeepLoaded::Always,
    );

    // Loads the model up front: load time is a separate cost from the per-dictation budget.
    let started = Instant::now();
    let warm_up = LlmPrompt {
        user: "Hello".into(),
        max_tokens: 1,
        ..LlmPrompt::default()
    };
    host.generate(&warm_up, &mut |_| {})?;
    println!("model {model}\nloaded in {:?}", started.elapsed());

    Ok(eval::run(cases, &mut host))
}

#[cfg(not(feature = "llama"))]
fn eval_with_model(_cases: &[eval::Case], _model: &str, _cpu: bool) -> anyhow::Result<Report> {
    bail!("built without the `llama` feature; run it through `cargo xtask`")
}
