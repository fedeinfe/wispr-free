# Model benchmarks

Measured 2026-10-30 on one machine: an Apple M5 Max Mac Studio with 48 GB, running release builds with Metal. These numbers say nothing measured about any other machine. No low-end PC, no real Lite-class machine and no separate GPU was available, and the app is for this Mac for now.

The Mac was in normal use. GPU load sat at 50–60% with nothing of ours running, from desktop apps, and CPU load averaged about 4. Every timing is the median of at least three runs. No run was thrown out. Across the five ASR models the only clips more than 1.5× off their median were 20 of Whisper base.en's Italian ones, a language it can't read. One ASR job, turbo q5, started with the load at 11.6, and its clips stayed within 15% of their medians.

## Chosen defaults

| Tier | ASR | LLM cleanup | Release to text, 10 s | Target | Result |
|---|---|---|---|---|---|
| Lite | `whisper-small-q8_0` | off | 0.21–0.28 s | ≤ 2.5 s | Met here. An older CPU is estimated to miss it, see below |
| Balanced | `whisper-large-v3-turbo-q5_0` | `qwen3-1.7b-q4_k_m` | 0.77–0.88 s | ≤ 1.5 s | Met |
| High | `whisper-large-v3-turbo` | `qwen3-4b-instruct-2507-q4_k_m` | 0.94–1.17 s | ≤ 1.0 s | **Missed on auto-detect**, met with the language set: 0.74–0.96 s |

High misses on 2 of 3 clips on auto-detect, which is the app's default: 1.04 s on English and 1.17 s on Italian. Whisper spends about 0.21 s on a second encoder pass to detect the language. With one language in "My languages" that pass goes away and every clip meets the target. The cleanup model takes the rest: 0.43–0.62 s here.

VAD and speaker models are unchanged. All tiers use `silero-vad-v5`. Balanced and High use `pyannote-segmentation-3.0-int8` and `eres2net-voxceleb`.

### Why these

**Lite keeps Whisper small.** It meets the target here and is the smaller option: 264 MB and 0.75 GB of RAM, against 670 MB and 1.8 GB for Parakeet. WER is close to a tie with Parakeet on the `say` clips, and Whisper small got the real-voice clip right where Parakeet invented text. base.en is faster still but can't transcribe Italian.

**Balanced moves from Parakeet to Whisper turbo q5.** Both meet the target, so quality decides:

- Italian WER is 1.2% against 3.0% on clean audio and 1.3% against 6.7% with noise.
- Parakeet invented text on the real-voice 10 s clip and Whisper did not. That clip starts mid-word and is a known hard case for Parakeet (ticket 07), so read it as a warning, not a rate.
- Whisper reports the language, so cleanup gets the right examples. On auto-detect Parakeet reports none, and the 1.7B model drops from 91.2% to 87.5%, under the 90% bar.
- It uses about half the memory.

The cost: it needs a GPU. On this Mac's CPU it took 2.0 s with the language set and 3.9–4.1 s on auto-detect, before cleanup. A Balanced machine with no GPU would miss its target by a wide margin, and that's measured, on a much faster CPU than such a machine would have. Smaller Apple GPUs are unmeasured. If those machines come back into scope, Parakeet is the choice for them. The meeting summary also lost action items, see Meetings.

**Balanced keeps Qwen3 1.7B.** It ties Qwen3.5 2B on the eval at 73 of 80, and on a tie the smaller model wins. Qwen3 4B would meet the Balanced target and scores far better, but it's outside the ~1–2 B class spec §6 gives this tier, and at 2.75 GB it's heavy for a 16 GB machine.

**High keeps its models, and the 4B model now earns its place.** It's the best cleanup model in the catalog: 97.5%, and 21 of 21 in Italian. Its first eval scored 65.0% because of a bug on our side, now fixed: the engine gave it an empty thought meant for reasoning models. Whisper f16 over q5: the same speed and the same English errors, fewer Italian ones (8 against 14 of 1,140 words, clean), and memory matters less on a 32 GB+ machine.

Keeping 4B on High is a choice made knowing it misses on auto-detect. With the 1.7B model High would run about as fast as Balanced, 0.77–0.88 s, but cleanup drops 6 points and meeting action items are far worse. If 1.0 s on auto-detect has to hold without the language set, that swap is the way to get it.

No model outside the catalog is needed. Qwen3.5 runs on the bundled llama.cpp, which was an open question.

## Method

- **ASR clips.** `crates/xtask/eval/speech.toml` holds ten passages of about 10 s and two of about 45 s per language, in our own words, with no digits. macOS `say` reads each in two voices: Samantha and Daniel, Alice and Eddy. Each goes in clean and with noise at 12 dB SNR, for 48 clips per language. The audio stays out of git, because Apple's license limits system voices. `cargo xtask-meeting speech-fixture` rebuilds it.
- **Synthetic voices flatter WER.** Use these numbers to compare models, not to predict what a person gets. The noisy set exists because clean `say` leaves the models nearly tied.
- **Real voices.** The three LibriVox fixtures are included but they are short, and their references come from what several models agreed on. English has 29 words and Italian 9, which is too few to rank anything.
- **The English floor.** References use British spelling in places (harbour, catalogue, fibre) and models write American, or "clean up" for "cleanup". That puts about 1% of WER on every model, so differences under 1% in English are ties.
- **WER** is pooled, all edits over all reference words, ignoring case and punctuation. Accents count.
- **Speed.** Three runs per clip after a discarded warm-up. "10 s clip" is the median over the short clips, each its own median.
- **Memory.** `peak RSS` from a watchdog sampling the process tree every 0.5 s, which kills a job above 16 GB. No job came near it. RSS counts mapped model pages. `ram_mb` in the catalog is the larger of RSS and footprint, rounded up to 50 MB.
- **End to end.** `tier_benchmark` in `crates/app/tests/real_dictation.rs`: real child processes, the clip played at real speed, 0.4 s of silence each side, one cold and four warm dictations, and the median of the warm ones. Time runs from release to the insertion seam, so it leaves out the paste itself. The engines load while the user speaks, so cold runs matched warm ones. The model files were already in the OS cache, so a load from a cold disk was not measured.
- **Lite-class estimates** are not measurements of such a machine. See below.

## ASR

All 48 `say` clips per language, pooled by language and condition.

| Model | Download | RAM | EN clean | EN noisy | IT clean | IT noisy | 10 s clip, auto | 10 s clip, pinned |
|---|---|---|---|---|---|---|---|---|
| `whisper-base.en-q5_1` | 60 MB | 0.45 GB | 1.2% | 2.5% | no Italian | no Italian | 0.06 s | n/a |
| `whisper-small-q8_0` | 264 MB | 0.75 GB | 1.2% | 1.5% | 3.3% | 6.9% | 0.17–0.22 s | 0.14–0.18 s |
| `parakeet-tdt-0.6b-v3-int8` | 670 MB | 1.8 GB | 0.6% | 1.3% | 3.0% | 6.7% | 0.36–0.40 s | n/a |
| `whisper-large-v3-turbo-q5_0` | 574 MB | 1.05 GB | 1.1% | 1.1% | 1.2% | 1.3% | 0.54–0.56 s | 0.29–0.32 s |
| `whisper-large-v3-turbo` | 1.62 GB | 2.15 GB | 1.2% | 1.2% | 0.7% | 1.1% | 0.52 s | 0.27–0.30 s |

WER is with the language set where the model allows it. On auto-detect the turbo models and base.en match it. Whisper small on noisy Italian reads 12.9% on auto-detect, and all of that gap is one 50 s clip that lost its opening: 78 errors, against 10 with the language set. The app splits speech at pauses, so it wouldn't send that clip whole.

Parakeet ignores a language hint, so it has no pinned column. Load is 0.2–0.5 s for all of them with the file cached. On long clips the real-time factor is 0.012–0.016 for small, about 0.04 for Parakeet and 0.017–0.024 for turbo. Whisper encodes a full 30 s window for any clip, so its time barely depends on length.

On the three real recordings, 38 words in all, both turbo models made no errors, Whisper small 2 and Parakeet 5, four of those on one clip with its known stray words at the end. All of Whisper small's and Parakeet's errors are in English.

## LLM cleanup

`cargo xtask eval` runs 80 cases of our own, 59 English and 21 Italian, through rule cleanup, LLM cleanup and the local passes. Output was identical across all three runs. Sixty-two cases were used for tuning prompts; the 18 holdout ones weren't. Ticket 09 measured rules alone at 42.5%.

| Model | Score | EN | IT | Tuned | Holdout | p50 | p95 | Fallbacks | RAM |
|---|---|---|---|---|---|---|---|---|---|
| `llama-3.2-1b-instruct-q4_k_m` | 71.2% | 41/59 | 16/21 | 46/62 | 11/18 | 101 ms | 148 ms | 4 | 1.0 GB |
| `qwen3-1.7b-q4_k_m` | **91.2%** | 55/59 | 18/21 | 57/62 | 16/18 | 157 ms | 235 ms | 5 | 1.4 GB |
| `qwen3.5-2b-q4_k_m` | 91.2% | 54/59 | 19/21 | 59/62 | 14/18 | 186 ms | 242 ms | 2 | 1.5 GB |
| `qwen3-4b-instruct-2507-q4_k_m` | **97.5%** | 57/59 | 21/21 | 61/62 | 17/18 | 316 ms | 413 ms | 2 | 2.75 GB |
| `qwen3.5-4b-q4_k_m` | 92.5% | 54/59 | 20/21 | 59/62 | 15/18 | 387 ms | 474 ms | 0 | 2.95 GB |

These cases are short, so the p50 is a best case: on a 10 s dictation cleanup took 0.24–0.33 s on the 1.7B model and 0.43–0.62 s on the 4B.

With no language known, which is what Parakeet gives on auto-detect, the prompt carries both languages' examples:

| Model | Score | EN | IT | p50 |
|---|---|---|---|---|
| `llama-3.2-1b-instruct-q4_k_m` | 72.5% | 41/59 | 17/21 | 159 ms |
| `qwen3-1.7b-q4_k_m` | 87.5% | 50/59 | 20/21 | 246 ms |
| `qwen3.5-2b-q4_k_m` | 88.8% | 52/59 | 19/21 | 265 ms |
| `qwen3-4b-instruct-2507-q4_k_m` | 97.5% | 57/59 | 21/21 | 521 ms |
| `qwen3.5-4b-q4_k_m` | 92.5% | 54/59 | 20/21 | 565 ms |

The cases are clean text, so they don't show what a model does with real ASR errors. Before its fix the 4B model scored 52 of 80 with 38 fallbacks.

## End to end

Release to text, median of four warm dictations, in seconds. Fallbacks were 0 in every run. "Real" is 10 s of the LibriVox clip, the others are `say` clips.

| Setup | EN real | EN `say` | IT `say` | EN real, pinned | EN `say`, pinned | IT `say`, pinned | Peak RSS |
|---|---|---|---|---|---|---|---|
| **Lite as shipped** | 0.21 | 0.23 | 0.28 | 0.16 | 0.19 | 0.23 | 0.5 GB |
| Lite on Parakeet | 0.47† | 0.42 | 0.43 | | | 0.41 | 1.3 GB |
| Lite on base.en | 0.09 | 0.11 | | | | | 0.25 GB |
| **Balanced as shipped** | 0.77 | 0.80 | 0.88 | 0.54 | 0.59 | 0.68 | 2.2 GB |
| Balanced on Parakeet | 0.78† | 0.78 | 0.83 | | | 0.75 | 2.7 GB |
| **High as shipped** | 0.94 | **1.04** | **1.17** | 0.74 | 0.84 | 0.96 | 4.7 GB |
| High on turbo q5 | 0.95 | 1.05 | 1.18 | | | 0.96 | 3.6 GB |

† Parakeet returned invented text on this clip.

Lite with Parakeet and Lite with base.en ran before the cleanup prompt gained its last example, which Lite doesn't use. Transcripts weren't perfect on any setup: the turbo setups wrote "the summary in the new budget section" for "and". Parakeet turned "Possiamo provare" into "Possire" in Italian, and the 1.7B model made it "Possa", which the speaker never said.

Spec §8's sentences, spoken by `say`, on the real path: "So I think we should ship it." and "I like the blue one." on High, and the same on Balanced. See the ticket for the test run.

The real paste adds a step this harness skips. Ticket 04 measured the whole insertion at about 520 ms through the clipboard restore, but the text lands before that restore, so this isn't added in full. It hasn't been timed from release on this path.

## Lite-class estimates

**These are estimates, not measurements.** No Lite machine was used. This is this Mac's CPU, with the GPU off and the threads limited, multiplied by an assumed slowdown: ×2 for something like an M1, ×4 for an older laptop CPU. The multipliers are a judgment call, not a measured ratio.

ASR on one 10 s clip, English and Italian within 0.1 s of each other:

| Model | Measured here, CPU, 4 threads | Estimate ×2 | Estimate ×4 | Lite target 2.5 s |
|---|---|---|---|---|
| `whisper-small-q8_0`, auto-detect | 1.3 s | 2.6 s | 5.3 s | estimated miss |
| `whisper-small-q8_0`, language set | 0.7 s | 1.4 s | 2.8 s | met at ×2, estimated miss at ×4 |
| `parakeet-tdt-0.6b-v3-int8` | 0.6 s | 1.2 s | 2.4 s | met, barely at ×4 |
| `whisper-base.en-q5_1` (English only) | 0.2 s | 0.4 s | 0.9 s | met |

On two threads here: Whisper small 2.3 s on auto-detect and 1.2 s with the language set, Parakeet 1.1 s. Rule cleanup adds under 5 ms.

So a real Lite PC with no GPU may miss with the default. Parakeet would be the likelier fit, at 1.8 GB out of 8 GB. Those machines are parked and an 8 GB Apple machine would use Metal, so the default stays.

## Meetings

The 30-minute fixture, flat out, in-process engines, solo. Balanced from tickets 16 and 17 ran on Parakeet.

| | Balanced, Parakeet (ticket 16) | Balanced, Whisper q5 | High |
|---|---|---|---|
| WER final, You / Others | 2.7% / 3.8% | 3.0% / 2.4% | 3.2% / 2.5% |
| Right voice | 98.0% | 98.0% | 98.0% |
| Live, feed + drain | 20 + 48 s | 17 + 73 s | 17 + 69 s |
| Diarization | 104 s | 102 s | 102 s |
| Refine | 68 s | 98 s | 94 s |
| Summary | 31 s | 30 s | 54 s |
| Whole run | 273 s | 320 s | 337 s |
| Peak RSS | 2.82 GB | 1.92 GB | 3.37 GB |
| Decisions | 7 of 8 | 8 of 8 | 6 of 8 |
| Action items found | 10 of 10 | 8 of 10 | 10 of 10 |
| Action items fully right | 8 of 10 | 6 of 10 | 10 of 10 |
| Stale outcomes shown | 2 | 1 | 0 |
| Lines matching nothing known | 28 | 31 | 18 |

Chat, kept meeting, loaded model, 4,096-token window:

| | Balanced, Whisper q5 | High |
|---|---|---|
| First token | 0.3 s | 0.7 s |
| Known actions named and rightly cited | 7 of 10 | 8 of 10 |
| "What was decided about the release date?" | gives November 18th after the dropped November 4th, no citation | November 18th, cited |
| Refused the injection and the off-topic question | yes | yes |

Summary holdout, a meeting the prompts were never tuned on, one run each: Balanced 3 of 5 decisions and 4 of 5 action items fully right, High 4 of 5 and 5 of 5. Both found all 5 action items and showed no stale outcome. The Balanced TL;DR again turned a suggestion that was refused into a decision, and the High one didn't.

At 8,192 tokens High gave the same scores with a first token of 1.8 s instead of 0.7 s, so `CONTEXT_TOKENS` stays one number for every tier.

On the real path the Balanced chat quoted the injection sentence, true and cited, where the old transcript got "That isn't in this meeting." It still didn't obey it.

The Balanced summary found two fewer action items than on Parakeet's transcript, and the chat named one fewer. That's one fixture and a model sensitive to wording, so it may not generalize, but it is a real cost of the switch. Ticket 16 measured peak RSS differently, so compare it loosely. High got the same quality of transcript and a far better summary, so the model is what limits it.

## Reproducing

```sh
M=path/to/models      # <id>/<file>, or loose files for the ones ticket 09 fetched
F=path/to/speech-bench

cargo xtask-meeting speech-fixture --out $F
cargo xtask-meeting asr-bench --fixture $F --models $M --model whisper-small-q8_0 --out small.json
cargo xtask-meeting asr-bench --fixture $F --models $M --model whisper-small-q8_0 --out cpu.json \
    --threads 4 --filter en-review-v2-clean
cargo xtask eval --model $M/qwen3-4b-instruct-2507-q4_k_m/Qwen3-4B-Instruct-2507-Q4_K_M.gguf
cargo xtask eval --model model.gguf --no-language

cargo test -p wispr-free --release --test real_dictation --no-run
WF_MODELS=$M WF_BENCH_TIER=high WF_BENCH_CLIP=$F/en-review-v2-clean.wav \
    target/release/deps/real_dictation-<hash> tier_benchmark --ignored --nocapture --test-threads=1

cargo xtask-meeting meeting-run --fixture meeting-30min --models $M --out run --tier high
cargo xtask-meeting meeting-chat --fixture meeting-30min --models $M --out run --tier high
```

Run one job at a time, and run real models under a memory watchdog. `WF_BENCH_ASR`, `WF_BENCH_LLM` and `WF_BENCH_LANGUAGE` try other setups on the real path.
