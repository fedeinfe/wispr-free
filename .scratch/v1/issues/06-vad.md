# 06 Silero VAD stage
Status: ready-for-agent
Blocked by: 02

Silero VAD behind the `Vad` trait. Trim leading/trailing silence, chunk long dictations at pauses, optional hands-free auto-stop. Toggleable.

Done when: silence-only recordings produce no ASR call and no text.

## Comments

**2026-10-30 · implementer (branch `t06-vad`)**

Done (`crates/pipeline`, test-first):
- `Vad` is now a stream: `accept` returns segments as they close, `is_speaking`, `finish` (returns what's left and resets). `speech_segments(clip)` stays as a provided method. Silero needs this: chunks and auto-stop must work during the recording, and `detected()` alone is too imprecise to trim with (about 0.3 s late at onset, holds on for the minimum silence after).
- `vad::speech_chunks(clip, vad, options)`: trims to the speech plus 200 ms padding, and closes a chunk at the first pause after 15 s. Speech is never cut mid-way, so a chunk can run long if nobody pauses. Passing `None` as `vad` is VAD off: the whole recording goes through.
- `vad::LiveVad`: the same logic during a recording. It hands out chunks as pauses confirm them, and `LiveUpdate::auto_stop` fires after `VadOptions::auto_stop` of silence. Off by default; set it to 3 s in hands-free only, per spec.
- `vad::transcribe_speech(chunks, asr, options)`: the ASR gate. No chunks means no `Asr` call and `Ok(None)`, and an engine that hears no words also gives `None`.
- `silero::SileroVad` behind the `silero` feature (sherpa-onnx), so default builds don't pull it in. It loads a model path; a missing file is `ModelMissing`, a corrupt one is `Engine`.
- Fixtures in `crates/pipeline/tests/fixtures/`: human speech in English and Italian from public-domain LibriVox recordings, plus synthesized room tone. Sources and commands are in its README.

Decisions:
- Auto-stop counts silence from the start of the dictation, before the first word too. That is the literal spec, and it stops a forgotten hands-free dictation holding the mic open. The cost: someone who thinks for 3 s before speaking gets stopped, with no text.
- Fixtures don't use macOS `say`: Apple's license text limits system voices to personal, non-commercial use, and this repo is public under MIT.
- Tests sit at four seams, chosen by the implementer since no human was available to confirm them: `speech_chunks`, `LiveVad`, `transcribe_speech` (all against a fake `Vad`) and `SileroVad` (real model).

Verified on macOS (Apple Silicon) with the real Silero v5 model (SHA-256 matches ticket 11's catalog):
- **Done when is met for this stage.** Digital silence and room tone, run through `SileroVad`, `speech_chunks` and `transcribe_speech`, give 0 ASR calls and no text, and the English and Italian speech fixtures give exactly 1 call. A silent hands-free dictation auto-stops, also with 0 calls. These tests fail when the engine is made to report a segment that isn't there.
- Trimming keeps every sample above −34 dBFS, including with room tone mixed in and when stopped mid-sentence. A real pause splits the chunk, and an abandoned recording leaves no state behind. Each behaviour was mutated to confirm a test catches it.
- Run the real-model tests with `WF_SILERO_MODEL=<path>/silero_vad.onnx cargo test -p wf-pipeline --features silero -- --include-ignored`. They're ignored otherwise, so plain `cargo test` proves nothing about Silero.

Not verified / needs human verification:
- **Real microphone audio.** "Silence" here is digital silence and synthesized room tone, because agent shells only get zeros from the mic (ticket 02). Check breathing, keyboard clicks, fans and speech in a noisy room. These are the cases most likely to reach ASR.
- **Nothing is wired into the app.** No dictation runs through this stage yet, and no real ASR ran here (ticket 07 is not done): ASR was a counting fake. The toggle is the `Option`; the settings UI and tier wiring are ticket 12's.
- **`Recorder` doesn't expose its pre-roll,** and `LiveVad` needs to be fed it first. Whoever wires live chunking must add that or lose the first 300 ms from live chunks. `speech_chunks` on the finished clip has no such issue.
- **Windows and Linux.** Nothing has run there. CI doesn't build the `silero` feature or run the model tests, so the feature can break without CI noticing.
- `min_chunk` (15 s) and the 0.5 s pause are judgement calls. Tune them once ticket 14's benchmark measures ASR on real chunks.
