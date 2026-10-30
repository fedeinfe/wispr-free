# ADR-0003: Local inference stack

Status: accepted · 2026-10-30

## Context

Wispr Flow runs transcription, cleanup, summaries and chat in its cloud. We must reach comparable output fully offline, across hardware from 8 GB laptops to GPU workstations, with each heavy stage optional.

## Decision

| Stage | Engine | Rust binding (verified 2026-10-30) |
|---|---|---|
| Capture | cpal + rubato (resample) | `cpal` 0.18.2, `rubato` 5.0.1 |
| Denoise (opt.) | RNNoise | `nnnoiseless` 0.5.2 (pure Rust) |
| VAD | Silero VAD (ONNX) | `sherpa-onnx` 2.0.1 |
| ASR (transducer, e.g. Parakeet) | sherpa-onnx | `sherpa-onnx` 2.0.1 (official k2-fsa crate) |
| ASR (Whisper family) | whisper.cpp (Metal / CUDA / Vulkan / CPU) | `whisper-rs` 0.16.0 |
| Diarization (meetings) | sherpa-onnx speaker segmentation + embeddings | `sherpa-onnx` 2.0.1 |
| LLM cleanup, summaries, chat | llama.cpp (GGUF) | `llama-cpp-2` 0.1.160 |
| Storage | SQLite | `rusqlite` 0.41.1 (`bundled`) |

Versions are pinned once in the root `Cargo.toml` `[workspace.dependencies]`. Each crate opts in when its ticket lands, so the scaffold builds without any C++ toolchain work.

All engines sit behind small traits (`Vad`, `Asr`, `Llm` in `wf-pipeline`), so models and backends can be swapped. Model choice lives in a versioned manifest, not in code.

Refinements made while verifying:

- **VAD uses sherpa-onnx, not `ort`.** sherpa-onnx already ships Silero VAD and links its own onnxruntime. Adding `ort` as well would put a second onnxruntime in the binary.
- **Prefer the official `sherpa-onnx` crate over the community `sherpa-rs`.** It downloads prebuilt static libs from the k2-fsa GitHub releases at build time. Set `SHERPA_ONNX_LIB_DIR` or `SHERPA_ONNX_ARCHIVE_DIR` for offline or CI-cached builds.
- **`llama-cpp-2` is used with `default-features = false`.** Its defaults pull OpenMP and Android options we don't need. GPU backends are features on both ggml crates: `metal`, `cuda`, `vulkan`.

## Consequences

- Native C/C++ dependencies (whisper.cpp, llama.cpp, onnxruntime) complicate cross-compiling. CI builds on each OS natively.
- GPU backends are cargo features. Release builds: Metal on macOS; CPU + Vulkan on Windows and Linux, with CUDA as an optional build.
- `whisper-rs-sys` and `llama-cpp-sys-2` each vendor and statically link their own ggml, so the same `ggml_*` symbols appear twice. On macOS the linker keeps one copy silently (verified: a probe binary linking all three engines builds and runs). This risks a ggml version mismatch at runtime and may be a hard "multiple definition" error with GNU ld. Tickets 07 and 09 must resolve it: build both against one ggml (`llama-cpp-2` has `system-ggml`), keep the ggml revisions aligned, or run one engine out of process.
