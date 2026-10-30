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
- **llama.cpp is built as a shared library (`llama-cpp-2/dynamic-link`); whisper.cpp stays static.** `whisper-rs-sys` vendors ggml 0.9.5 and `llama-cpp-sys-2` vendors ggml 0.26.0, about 530 duplicate symbols with different enums and struct layouts, and neither crate offers a way to share one copy (`whisper-rs-sys` doesn't expose `WHISPER_USE_SYSTEM_GGML`). Linked statically they fail: macOS `ld` keeps one copy of each symbol and llama.cpp aborts on its first context (`GGML_ASSERT(a->op == GGML_OP_FLASH_ATTN_EXT)`), and GNU `ld` refuses with `multiple definition`. Ticket 01's probe linked and started but never ran inference, so it missed this.
- Each engine now has its own ggml in one process, which needs two linker settings in `.cargo/config.toml`. On Linux `-Wl,--exclude-libs,ALL` keeps the executable's static ggml out of its dynamic symbol table; without it `libllama.so` binds to the older copy and aborts the same way, even though it links cleanly. An rpath to the executable's folder lets the binary start outside `cargo run`, which hides a missing one by setting the library path.
- **The app is no longer a single file** (spec goal 4). `libllama`, `libllama-common` and `libggml*` (15 MB on macOS with Metal) ship beside the binary, or in `Contents/Frameworks` with a matching rpath in the app bundle. Ticket 18 packages them.

| Linked with both engines | macOS arm64 (Metal) | Linux arm64 (CPU, GNU ld) | Windows |
|---|---|---|---|
| Both static | links, llama.cpp aborts | link error | not tested |
| llama.cpp shared | works | links, llama.cpp aborts | not tested |
| llama.cpp shared + `.cargo/config.toml` | **works** | **works** | not tested |

"Works" means real inference through whisper.cpp, llama.cpp and sherpa-onnx in one process, in both orders, with text output checked. Windows is the open risk: `ggml.lib` would exist both as a static library and as an import library, so check it the first time Windows builds both engines. If the shared library ever stops being enough, the fallback is running llama.cpp in a helper process.
