# 07 ASR engines
Status: ready-for-agent
Blocked by: 01, 06

`Asr` trait with a whisper.cpp implementation (Metal/CUDA/Vulkan features) and a sherpa-onnx implementation (Parakeet-class transducers). Dictionary and context biasing (initial prompt / hotwords), language auto or fixed. Streaming chunk mode for meetings and long dictations.

Done when: the same WAV fixtures transcribe through both engines from a CLI, with real-time factor reported.

## Comments

**2026-10-30 · implementer (branch `t07-asr-engines`)**

Done (`crates/pipeline`, test-first; API and measurements are in the notes file `07-asr-engines.md`):
- `whisper::WhisperAsr` (feature `whisper`) and `sherpa::SherpaAsr` (feature `sherpa`, Parakeet-class NeMo transducers) behind `Asr`. `metal`, `cuda` and `vulkan` select the GPU backend. Default builds pull in neither engine.
- Dictionary and context biasing through the initial prompt (Whisper) and hotwords (Parakeet). Language is auto or fixed; Parakeet can only follow the speaker.
- Streaming chunk mode: `worker::AsrWorker` owns the engine on its own thread, so a slow model can't hold up VAD, and a meeting's two channels share one loaded model. `vad::LiveVad` now yields `LiveChunk { start, clip }` for `[mm:ss]` timestamps, which changes the type ticket 06 returned.
- CLI: `crates/pipeline/examples/asr.rs`.
- ggml: llama.cpp is now a shared library, plus linker flags in `.cargo/config.toml`. Decision and evidence in ADR-0003.

**Done when is met.** The same WAV fixtures transcribe through both engines from the CLI, with RTF reported (Apple M5 Max, warm):

| | Whisper small q8, Metal | Whisper small q8, CPU | Parakeet v3 int8, CPU |
|---|---|---|---|
| `speech_en.wav` 4.25 s | 0.031 | 0.199 | 0.041 |
| `speech_it.wav` 4.15 s | 0.032 | 0.204 | 0.041 |
| 150 s real speech, live path | 0.012–0.015 | 0.054–0.055 | 0.037–0.038 |

Problems found by testing on real audio, now fixed, each with a test seen failing first:
- **Linking both ggml copies statically is broken, not merely risky.** On macOS llama.cpp aborted, on Linux the link failed. Ticket 01's probe never ran inference. After the shared-library change, Linux still aborted until the executable's symbols were hidden, and the binary couldn't start outside `cargo run` without an rpath.
- **Parakeet silently lost speech:** 19 of 70 real clips came back empty with no error. `blank_penalty = 3.0` fixes it (0 of 70 empty). English word error against Whisper went from 17.7% to 9.6%.
- Parakeet aborted the whole process on clips under about 1 s. Whisper typed `[BLANK_AUDIO]` or "you" on silence, and failed on short clips and on English-only models.

Seams were chosen by the implementer since no human was available to confirm them: one `Asr` contract run against both real engines (`tests/asr_engines.rs`), every engine in one process (`tests/engines_together.rs`), `LiveVad` chunk positions, and `AsrWorker` with a fake engine (`tests/asr_worker.rs`). Real-model tests are ignored by default, so plain `cargo test` proves nothing about the engines. Commands are in the notes.

Verified on macOS: all tests with every engine feature and real models on Metal, including ticket 09's. On Linux arm64 in a container (CPU, GNU ld): the standalone probe ran real inference through all three engines with the committed linker flags and no library path. The workspace tests were not run on Linux.

Needs a decision:
- **Spec goal 4 says "one small native binary".** The app now also ships `libllama`, `libllama-common` and `libggml*` (15 MB on macOS). Ticket 18 must package them. On Linux they aren't copied next to the binary, so it won't start until they are. The alternative is running llama.cpp in a helper process.
- **Whisper invents text on noise with VAD off** ("Thank you for watching."), and no confidence signal separates it from speech. Spec §8 has Lite running with VAD off. Recommendation: keep VAD on whenever the engine is Whisper, in tickets 12 and 19. Parakeet stayed silent on the same audio.

Not verified / needs human verification:
- **Real microphone audio.** Everything ran on recordings. The Parakeet penalty and the silence filter are tuned on LibriVox speech and synthetic room tone; real rooms are the likely place for them to be wrong. The margin is narrow: at 4.5 Parakeet invents text on room tone.
- **Windows**: nothing has run there, and whether the shared-library fix links is unknown (`ggml.lib` would exist both as a static and an import library). Linux x86_64, CUDA and Vulkan were never built.
- CI doesn't build any engine feature, so a break in them won't show up there. Adding `--features whisper,sherpa` would also download models' native libraries at build time, so that was left for a human to decide.
- Accuracy numbers use Whisper as the reference, not a human transcript, so they are relative. Ticket 14 should measure real WER.
- `blank_penalty` can add a stray word or two at the end of a clip (`speech_en.wav` ends "… M. D., by the"). Whisper is unaffected.
- Mixed-language speech within one chunk loses text on both engines. Tested only on an artificial clip.
- `HOTWORD_SCORE`, `BLANK_PENALTY` and the 8-thread cap were tuned on one machine.
- Nothing is wired into the app (ticket 19).
