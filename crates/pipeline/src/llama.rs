//! The llama.cpp engine behind [`Llm`], for GGUF instruct models. Run it through an
//! [`LlmHost`](crate::llm_host::LlmHost), which does the lazy loading and unloading.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::{LogOptions, send_logs_to_tracing};

use crate::{Llm, LlmPrompt, PipelineError};

/// Used when a prompt leaves `max_tokens` at 0.
const DEFAULT_MAX_TOKENS: u32 = 1024;

#[derive(Debug, Clone)]
pub struct LlamaConfig {
    pub model: PathBuf,
    /// Layers to offload to the GPU. Backends without a GPU ignore it.
    pub gpu_layers: u32,
    /// CPU threads for whatever stays off the GPU.
    pub threads: i32,
    /// Map the file rather than read it into memory. Loads faster, but every page has to stay
    /// reachable: a read from an unplugged drive is a bus error, which no caller can catch.
    pub use_mmap: bool,
}

impl LlamaConfig {
    pub fn new(model: impl Into<PathBuf>) -> Self {
        Self {
            model: model.into(),
            gpu_layers: u32::MAX,
            threads: default_threads(),
            use_mmap: true,
        }
    }
}

/// Half the logical cores, which skips hyperthreads and most efficiency cores. llama.cpp's own
/// default of 4 times out on CPU: with this prompt, 62 of 62 eval cases on a 12+6-core Mac.
fn default_threads() -> i32 {
    let logical = std::thread::available_parallelism().map_or(8, |n| n.get());
    (logical / 2).max(4) as i32
}

/// Whether a model reasons before it answers, from its chat template. Mentioning `</think>` isn't
/// enough: Qwen3 4B Instruct's template strips the tag from history and the model never writes it.
/// Handed an empty thought anyway it refuses or says nothing, which took it to 65% on the eval.
fn thinks(template: &str) -> bool {
    template.contains("enable_thinking")
}

/// llama.cpp allows one backend per process, and it holds no model memory, so it stays up.
fn backend() -> Result<&'static LlamaBackend, PipelineError> {
    static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| {
            send_logs_to_tracing(LogOptions::default());
            LlamaBackend::init().map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| PipelineError::Engine(error.clone()))
}

pub struct LlamaEngine {
    model: LlamaModel,
    template: LlamaChatTemplate,
    /// Opens the assistant turn. Reasoning models get an empty thought so they answer at once.
    reply_prefix: &'static str,
    threads: i32,
}

impl LlamaEngine {
    pub fn load(config: &LlamaConfig) -> Result<Self, PipelineError> {
        if !config.model.exists() {
            return Err(PipelineError::ModelMissing(
                config.model.display().to_string(),
            ));
        }
        let params = LlamaModelParams::default()
            .with_n_gpu_layers(config.gpu_layers)
            .with_use_mmap(config.use_mmap);
        let model =
            LlamaModel::load_from_file(backend()?, &config.model, &params).map_err(engine)?;
        let template = model.chat_template(None).map_err(engine)?;
        let reasoning = template.to_str().is_ok_and(thinks);
        Ok(Self {
            model,
            template,
            reply_prefix: if reasoning {
                "<think>\n\n</think>\n\n"
            } else {
                ""
            },
            threads: config.threads,
        })
    }

    fn render(&self, prompt: &LlmPrompt) -> Result<String, PipelineError> {
        let message = |role: &str, content: &str| {
            LlamaChatMessage::new(role.to_owned(), content.to_owned()).map_err(engine)
        };
        let mut chat = Vec::with_capacity(2 + 2 * prompt.examples.len());
        if !prompt.system.is_empty() {
            chat.push(message("system", &prompt.system)?);
        }
        for (user, assistant) in &prompt.examples {
            chat.push(message("user", user)?);
            chat.push(message("assistant", assistant)?);
        }
        chat.push(message("user", &prompt.user)?);
        let mut text = self
            .model
            .apply_chat_template(&self.template, &chat, true)
            .map_err(engine)?;
        text.push_str(self.reply_prefix);
        Ok(text)
    }
}

impl Llm for LlamaEngine {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        let vocab = self.model.vocab();
        let tokens = vocab.tokenize(self.render(prompt)?.as_bytes(), true, true);
        let limit = self.model.n_ctx_train();
        let prompt_len = tokens.len() as u32;
        if prompt_len >= limit {
            return Err(PipelineError::Engine(format!(
                "prompt of {prompt_len} tokens does not fit the model's {limit}"
            )));
        }
        let wanted = match prompt.max_tokens {
            0 => DEFAULT_MAX_TOKENS,
            n => n,
        };
        let n_ctx = prompt_len.saturating_add(wanted).min(limit);

        // A context sized to the prompt keeps memory low and is cheap next to a decode.
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            .with_n_batch(n_ctx)
            .with_n_threads(self.threads)
            .with_n_threads_batch(self.threads);
        let mut context = self.model.new_context(backend()?, params).map_err(engine)?;
        let mut batch = LlamaBatch::new(n_ctx as usize, 1);
        batch.add_sequence(&tokens, 0, false).map_err(engine)?;
        context.decode(&mut batch).map_err(engine)?;

        let mut sampler = LlamaSampler::greedy();
        let mut utf8 = Utf8Stream::default();
        let mut reply = String::new();
        for position in prompt_len..n_ctx {
            if prompt.deadline.is_some_and(|d| Instant::now() >= d) {
                return Err(PipelineError::Timeout);
            }
            let token = sampler.sample(&context, batch.n_tokens() - 1);
            sampler.accept(token);
            if vocab.is_eog(token) {
                break;
            }
            let piece = utf8.push(&vocab.token_to_piece(token, false, None));
            if !piece.is_empty() {
                on_token(&piece);
                reply.push_str(&piece);
            }
            batch.clear();
            batch
                .add(token, position as i32, &[0], true)
                .map_err(engine)?;
            context.decode(&mut batch).map_err(engine)?;
        }
        Ok(reply)
    }
}

fn engine(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Engine(error.to_string())
}

/// Token pieces can split a character, so bytes wait here until they form valid UTF-8.
#[derive(Default)]
struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(text) => text.len(),
            // `error_len` is `None` when the tail is a character still being completed.
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => {
                let text = String::from_utf8_lossy(&self.pending).into_owned();
                self.pending.clear();
                return text;
            }
        };
        let text = String::from_utf8_lossy(&self.pending[..valid]).into_owned();
        self.pending.drain(..valid);
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{Utf8Stream, thinks};

    #[test]
    fn only_a_template_that_can_turn_thinking_off_gets_an_empty_thought() {
        let reasoning = "{%- if enable_thinking is defined and enable_thinking is false %}\
                         {{- '<think>\\n\\n</think>\\n\\n' }}{%- endif %}";
        let strips_history = "{%- set content = message.content.split('</think>')[-1] %}";

        assert!(thinks(reasoning));
        assert!(!thinks(strips_history));
    }

    #[test]
    fn a_character_split_across_pieces_comes_out_whole() {
        let mut stream = Utf8Stream::default();
        let bytes = "è".as_bytes();

        assert_eq!(stream.push(&[b'p', bytes[0]]), "p");
        assert_eq!(stream.push(&bytes[1..]), "è");
    }

    #[test]
    fn invalid_bytes_do_not_block_the_stream() {
        let mut stream = Utf8Stream::default();

        assert_eq!(stream.push(&[0xff]), "\u{fffd}");
        assert_eq!(stream.push(b"ok"), "ok");
    }
}
