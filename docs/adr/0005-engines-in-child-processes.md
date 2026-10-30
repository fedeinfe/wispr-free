# ADR-0005: Engines run in child processes

Status: accepted · 2026-10-30

## Context

Spec goal 4 asks for idle memory under 40 MB after a model has been used, with models unloading after an idle timeout. Dropping an engine doesn't get there. Measured with `phys_footprint` on Apple silicon, the process that had loaded a model kept 107 MB after Whisper, 452 MB after Parakeet, 67 MB after llama.cpp and 435 MB after VAD, Parakeet and llama.cpp together. Most of that is Metal, ggml and onnxruntime state that their APIs give no way to release.

Three more problems point the same way. A read from a model mapped from an unplugged drive is a bus error, which no caller can catch. An onnxruntime or ggml assertion aborts whatever process it runs in. And Cancel has to stop a call that has already started, which the engine traits have no way to do.

## Decision

Each of VAD, ASR and LLM runs in its own child process, the app's own binary started with a hidden `--engine-host` argument. The dictation flow holds proxies that implement the same `Vad`, `Asr` and `Llm` traits, so nothing above `wf_dictation::remote` knows.

- **Unload is killing the process.** The OS reclaims everything.
- **Cancel kills a busy host.** It costs a reload on the next dictation, and it stops any engine mid-call.
- **A crash is an error.** A dead host fails its call, the flow shows the pill's error state or falls back to rule-cleaned text, and the next dictation starts a new one.
- **Messages go over stdin and stdout**: two lengths, a JSON header, then raw `f32` samples. No sockets, so ADR-0004's "no network" stays easy to check.

## Consequences

- Idle footprint after a dictation with everything unloaded is about 30 MB, the same as before the first dictation.
- The app's main process never loads an engine, so whisper.cpp's and llama.cpp's ggml never run in the same process. They still link into one binary, so ADR-0003's linker settings stay.
- A model loaded from a chosen folder can't take the app down if its drive goes. llama.cpp also reads such a model into memory instead of mapping it, because the drive can go while a dictation runs.
- Loading costs a process start on top of the model load.
- Engine hosts inherit the app's permissions and its signature, so packaging (ticket 18) has nothing extra to sign. Their stdout is the protocol, so nothing in an engine host may print to it.
- If the user force-kills the app, a host sees its stdin close and exits.
