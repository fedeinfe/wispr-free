# 09 LLM cleanup
Status: ready-for-agent
Blocked by: 08

llama.cpp behind the `Llm` trait. Our own system prompt implementing spec §3 rules + strength + style. Guards: no preamble, 0.5–1.3× length check, 2–3 s timeout → rule-cleaned fallback. Lazy load and unload after idle. Prompt-injection test (dictated 'ignore previous instructions…' stays as text).

Done when: an eval set (≥ 50 cases: self-corrections, lists, numbers, styles) scores ≥ 90% on Balanced, run via `cargo xtask eval`.

## Comments

**2026-10-30 · implementer (branch `t09-llm-cleanup`)**

**Measured: 73/80 = 91.2% on `cargo xtask eval`** with Qwen3-1.7B Q4_K_M (unsloth GGUF, Apache-2.0) on an M5 Max with Metal: 57/62 on the cases used for prompt tuning and 16/18 (88.9%) on the holdout cases the prompt never saw. So the bar is met overall, but the holdout alone sits just under 90% on a small sample. Output repeats exactly between runs. For comparison: Qwen2.5-1.5B Q4_K_M 69/80 = 86.2% (17/18 holdout), rules and local passes with no LLM 34/80 = 42.5%. A perfect model scores 100% in the eval's own tests. The eval runs the real path (rule cleanup → LLM cleanup → local passes), and a fallback scores what the user would get.

Done:
- `wf_pipeline::cleanup::llm_cleanup(llm, request, timeout)`: our own rules prompt plus worked examples per language (en, it), strength and style. It always returns usable text, and `outcome.fallback` says why the model's reply wasn't used. Default timeout 2.5 s.
- Guards: no preamble, 0.5×–1.3× length, timeout, plus two the spec doesn't list: `NewContent` (Light only, over a third of the words never spoken) and `Answered` (a dictated question no longer opens the same way). Dictated "ignore previous instructions…" stays as text: the transcript travels in tags it can't close, the guards reject a model that obeys, and all 8 injection eval cases pass on the real model.
- `wf_pipeline::llm_host::LlmHost`: owns the model on its own thread, loads on first use, unloads per `KeepLoaded`, and returns `Timeout` on time even if the engine is stuck. It implements `Llm`, so meetings can share one loaded model.
- `wf_pipeline::llama::LlamaEngine` behind cargo feature `llama`, off by default. `LlmPrompt` gained `examples` and `deadline`.
- `cargo xtask eval` with 80 cases of our own in `crates/xtask/eval/cleanup.toml`: self-corrections, lists, numbers, styles, injection, dictionary, en and it.
- Tests, test-first: `llm_cleanup.rs` and `llm_host.rs` with fakes, `eval_scoring.rs` for the scorer, `llama_engine.rs` (ignored, needs a model). As a sub-agent run, the seams were chosen without asking: the public cleanup function, the host through `Llm`, and the scorer.

Decisions to review:
- **Spec conflict:** §3's 0.5× limit rejects §8's own example ("I like the blue one." is 0.42× its input). A reply may now go down to 0.25× if the input has a revision marker and at most 1 word in 5 is new. The spec text is unchanged; please confirm or amend it.
- **Rule cleanup (08) changed**, because the eval showed it destroying data before the LLM saw it: "compra due biglietti, anzi tre" became "Compra due tre." and "five five five one two" became "five one two". A number now only swaps a number, repeated numbers are kept, and "i meant" is a marker. Golden cases added.
- Style and email formatting come from ticket 10's `local_passes`; the model only gets a style hint.

Needs human verification / follow-ups:
- **Latency on real Balanced hardware.** ~160 ms p50 is an M5 Max. Every dictation sends a ~980-token prompt, cheap on Metal and costly elsewhere: CPU-only here was 2.1–2.3 s p50 and fell to 59–65/80 as cases hit the timeout. Ticket 14 should measure an M1 and a mid GPU. Caching the shared prompt prefix is the obvious fix and is not done.
- **Long dictations get rule cleanup only.** On this Mac 280 words took 1.7 s and 560 words timed out, so past roughly 2–3 minutes of speech the 2.5 s budget is never enough. Chunking or a bigger budget is a product decision.
- **Memory after unload.** RSS goes 7 → 1189 → 122 MB. Unloading works, but ~115 MB stays, which may break the 40 MB idle target once a model has been used. Check with `footprint`.
- **ggml skew is larger than ADR-0003 assumes:** llama-cpp-sys-2 vendors ggml 0.26.0, whisper-rs-sys 0.15 vendors 0.9.5. Not solved and not made worse here, since `llama` is off by default and whisper-rs isn't in the build. Ticket 07 should not ship both in one binary without proving the ABI or going out of process.
- The app doesn't call any of this yet. The `llama` feature has only been built on macOS: not Windows, Linux, Vulkan or CUDA, and CI doesn't build it.
- Medium and High strength have no eval cases. Eval inputs are clean text, not real ASR output. Italian is 21 of the 80 cases.
- The default model pick belongs to ticket 14.
