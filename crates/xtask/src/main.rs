//! `cargo xtask eval --model model.gguf` scores LLM cleanup against `eval/cleanup.toml`.
//! Flags: `--rules-only` scores rule cleanup alone, `--dev` skips the holdout cases, and
//! `--cpu` keeps every layer off the GPU. `cargo xtask bundle` is in [`xtask::bundle`].

use std::process::ExitCode;

use anyhow::{Context as _, bail};
use xtask::bundle;
use xtask::eval::{self, Report, RulesOnly, TARGET};

fn main() -> anyhow::Result<ExitCode> {
    let mut args = std::env::args().skip(1);
    let Some(task) = args.next() else {
        bail!("usage: cargo xtask eval [--model model.gguf] [--rules-only] [--dev] [--cpu]");
    };
    if task == "bundle" {
        return bundle::run(args).map(|()| ExitCode::SUCCESS);
    }
    if task == "signing-identity" {
        return bundle::create_identity().map(|()| ExitCode::SUCCESS);
    }
    if task != "eval" {
        bail!("unknown task {task:?}");
    }

    let mut model = std::env::var("WF_LLM_MODEL").ok();
    let (mut rules_only, mut dev, mut cpu) = (false, false, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => model = Some(args.next().context("--model needs a path")?),
            "--rules-only" => rules_only = true,
            "--dev" => dev = true,
            "--cpu" => cpu = true,
            other => bail!("unknown flag {other:?}"),
        }
    }

    let mut cases = eval::cases()?;
    if dev {
        cases.retain(|case| !case.holdout);
    }
    let report = if rules_only {
        eval::run(&cases, &mut RulesOnly)
    } else {
        let model = model.context("set --model or WF_LLM_MODEL to a GGUF file")?;
        eval_with_model(&cases, &model, cpu)?
    };
    print!("{}", report.render());
    Ok(if report.score() >= TARGET {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
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
