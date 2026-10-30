# ADR-0003: Local inference stack

Status: proposed · 2026-10-30

## Context

Wispr Flow runs transcription, cleanup, summaries and chat in its cloud. We must reach comparable output fully offline, across hardware from 8 GB laptops to GPU workstations, with each heavy stage optional.

## Decision

| Stage | Engine | Rust binding |
|---|---|---|
| Capture | cpal + rubato (resample) | native crates |
| Denoise (opt.) | RNNoise | `nnnoiseless` (pure Rust) |
| VAD | Silero VAD (ONNX) | sherpa-onnx or `ort` |
| ASR (transducer, e.g. Parakeet) | sherpa-onnx | sherpa-onnx Rust bindings |
| ASR (Whisper family) | whisper.cpp (Metal / CUDA / Vulkan / CPU) | `whisper-rs` |
| Diarization (meetings) | sherpa-onnx speaker segmentation + embeddings | same as ASR |
| LLM cleanup, summaries, chat | llama.cpp (GGUF) | `llama-cpp-2` |
| Storage | SQLite | `rusqlite` |

All engines sit behind small traits (`Vad`, `Asr`, `Llm`), so models and backends can be swapped. Model choice lives in a versioned manifest, not in code.

## Consequences

- Native C/C++ dependencies (whisper.cpp, llama.cpp, onnxruntime) complicate cross-compiling. CI builds on each OS natively.
- GPU backends are cargo features. Release builds: Metal on macOS; CPU + Vulkan on Windows and Linux, with CUDA as an optional build.
- Binding crate names and versions must be verified when the workspace is scaffolded (ticket 01). Mark this ADR accepted then.
