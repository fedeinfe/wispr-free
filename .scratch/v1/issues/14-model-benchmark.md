# 14 Benchmark + pick default models per tier
Status: ready-for-human
Blocked by: 07, 09

Measure WER and latency (EN + IT at minimum) and cleanup eval scores for the candidate ASR and LLM models on representative Lite / Balanced / High machines. Write the results to `docs/benchmarks.md` and set the manifest defaults. Human: needs access to real low-end and GPU machines.

Done when: each tier has chosen defaults that meet the spec §3 latency targets.

## Comments

### 2026-10-30

Scope changed by the user: this Mac only, an Apple M5 Max Mac Studio with 48 GB. No real Lite-class or separate GPU machine was used. Results and reasons are in `docs/benchmarks.md`.

Chosen defaults, with release to text on a 10 s clip:

| Tier | ASR | LLM | Auto-detect | Language set | Target |
|---|---|---|---|---|---|
| Lite | `whisper-small-q8_0` | off | 0.21–0.28 s | 0.16–0.23 s | ≤ 2.5 s, met |
| Balanced | `whisper-large-v3-turbo-q5_0` | `qwen3-1.7b-q4_k_m` | 0.77–0.88 s | 0.54–0.68 s | ≤ 1.5 s, met |
| High | `whisper-large-v3-turbo` | `qwen3-4b-instruct-2507-q4_k_m` | 0.94–1.17 s | 0.74–0.96 s | ≤ 1.0 s, **missed on auto-detect** |

The "done when" isn't fully met. High misses on auto-detect, the default, on 2 of 3 clips: 1.04 s English, 1.17 s Italian. It meets the target on all three with one language in "My languages". Keeping the 4B model anyway was a choice: 97.5% against 91.2% on the eval, and 10 of 10 meeting action items fully right against 6 of 10.

Only Balanced changed a model, from Parakeet to Whisper turbo q5.

Fixed along the way:
- **The 4B model scored 65.0% because of a bug on our side.** The engine read `</think>` in its template as a reasoning model and opened every reply with an empty thought. It now looks for `enable_thinking`. Score 97.5%, Italian 21 of 21.
- `cargo xtask-meeting` built Whisper without Metal, so a High run would have measured the CPU.
- The spec's own sentence came out "I meant the blue one." on the 4B model. One more worked example fixed that. It also made the model drop "No," from "no I think the blue one looks better", so the eval stayed at 78 of 80.

Verified on this Mac:
- ASR: 5 models, 48 `say` clips per language plus 3 real recordings, 3 runs each, clean and noisy. LLM: 5 models, 80 cases, 3 runs each with identical output, plus 3 each with no language known.
- End to end through `real_dictation`'s real child processes: 5 setups in the final set and 3 more before the last prompt change, 5 dictations each.
- High end to end, for the first time. Spec §8's sentences pass on High and Balanced, and the filler sentence on Lite.
- Meetings on the 30-minute fixture on Balanced and High, the summary holdout on both, and chat on both. High chat at 8,192 tokens gave the same scores, slower.
- Onboarding: someone on Balanced with the old models keeps the tier, and the Models step asks for the new model (unit test). Settings shows VAD locked on for Balanced, as spec §3 requires with Whisper (test).
- `cargo fmt --all --check` clean. Clippy clean on the workspace and on xtask's `meeting` feature. `cargo test --workspace --locked`: 970 passed, 0 failed, 75 ignored, in 92 test binaries.
- Real-model tests: `real_dictation` 13 of 13, `real_meeting` 2 of 2, `asr_engines` 38, `engines_together` 1, `llama_engine` 5 on 1.7B and 5 on 4B, `silero_vad` 13, `meeting_real` 2. All passed. The largest job reached 5.1 GB of RSS, and the 16 GB watchdog never fired.

Not verified:
- Any real person at a microphone. WER comes from `say` voices, which flatter it, plus 38 words of real recordings.
- Any other machine. The Lite figures are this CPU with an assumed slowdown, and they are labelled as estimates.
- Smaller Apple GPUs. Balanced needs a GPU now, and how it runs on an M1 is unknown.
- The paste itself. Timing stops at the insertion seam.
- Loading from a cold disk. Model files were in the OS cache, so cold runs matched warm ones.
- Balanced meetings at real speed. Those runs were flat out, plus the 15× real path.
- Italian meetings of any length on either tier. Long dictations with the new Balanced ASR.

Open or weak:
- **Moving Balanced costs a download.** Anyone on Balanced can't dictate until they fetch 574 MB. Missing ASR refuses at key-down, and the Models step reopens. The settings file isn't touched.
- **Balanced can't run without a GPU.** On this Mac's CPU it took 2.0–4.1 s before cleanup. `recommended_tier` still sends CPU-only machines to Balanced. Those are parked, but Parakeet is the choice when they return.
- **The Balanced summary lost ground on the new transcript:** 8 of 10 action items found and 6 fully right, from 10 and 8. Decisions went from 7 of 8 to 8 of 8. The chat named 7 of 10 known actions, from 8.
- **`real_meeting`'s injection check is looser now.** On Whisper's transcript the Balanced chat quotes the sentence instead of saying "That isn't in this meeting." That's true, cited and doesn't obey it, so the test accepts either.
- Lite on an older CPU is estimated to miss with the default, on auto-detect at ×2 and with either setting at ×4.
- Parakeet's language gap stays: on auto-detect cleanup gets no language, and the 1.7B model drops to 87.5%. Custom setups with Parakeet still hit it.
- The holdout cases were not looked at when the last worked example went in, but the score includes them, as before.
