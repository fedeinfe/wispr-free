# 16 Live transcript, refine, diarization, summary
Status: ready-for-agent
Blocked by: 07, 09, 15

VAD-chunked live transcription per channel with timestamps. Post-meeting refine pass + diarization of the Others channel, renameable speakers. Map-reduce summary into Title / TL;DR / Key points / Decisions / Action items.

Done when: the spec §8 30-minute meeting criterion passes on Balanced.

## Comments

**2026-10-30 · implementer (branch `t16-meeting-transcript-summary`)**

Done, headless, in `crates/meetings` (no changes under `crates/app`):
- Live VAD-chunked transcription per channel with `[mm:ss]` timestamps, on the aligned file timeline. Never holds up recording: ASR falling behind leaves a marked `Gap`, and refine fills it.
- Refine pass, sherpa-onnx diarization of Others (pyannote segmentation + ERes2Net), stable speaker ids, renames stored as data.
- Map-reduce summary: Title, TL;DR, Key points, Decisions, Action items with owner and due when stated. Transcript text is fenced and every reply parsed into fixed fields.
- Meeting record on disk, per-stage status, Regenerate, Update after a resumed recording, delete audio once final. A failed stage leaves earlier output usable.
- `crates/xtask`: scripted 30-minute fixture built with `say`, scored run, a holdout meeting, `llm.log`.
- Catalog: `pyannote-segmentation-3.0-int8` (MIT) and `eres2net-voxceleb` (Apache-2.0).

Verified on macOS, Balanced, real models:
- Spec §8's "a 30-minute meeting produces a timestamped transcript, a summary": ran at real speed, 2003.5 s, lag median 0.90 s, worst 3.28 s, 0 frames missed, 0 gaps, peak footprint 1.71 GB, max RSS 2.86 GB. Flat out 272.6 s.
- WER 2.7% You, 3.8% Others. 3 of 3 speakers, 98.0% of Others speech on the right voice.
- Summary: 7 of 8 decisions, 8 of 10 action items really right (the scorer says 10 found). 2 superseded decisions still listed, 28 of 46 lines match nothing known.
- Holdout, run once: 3 of 5 decisions, 5 of 5 actions found, 4 fully right, 0 stale. It reported a suggestion the speaker refused as a decision.
- An injected instruction doesn't change the format or reach the output as a line. A short Italian meeting gets a correct Italian summary.
- `cargo fmt`, clippy, `cargo test --workspace --locked` (617 passed, 0 failed, 5 ignored), real-model tests (184 pipeline, 2 meeting).

Not verified:
- Real speech, real rooms, real calls. All scores come from three clean synthetic voices, and the prompts and the 1.1 clustering threshold were tuned on them.
- More than three Others. Lite and High tiers. Long Italian meetings. French, German, Spanish and Portuguese summaries, which have no action example and are expected to lose action items.
- Live transcription fed by the real recorder and a real call: the harness feeds the fixture through `LiveTranscript`. Echo with no headphones can duplicate lines.
- Windows and Linux: parked by the user.

Open, for whoever picks this up:
- Superseded decisions and junk lines remain. Three ways of finding reversals with the small model failed: see the notes.
- Speaker models aren't on Settings → Models and `move_to` doesn't carry them until the app gets a `ModelKind` arm.
- ADR-0005 landed in `v1` meanwhile: diarization needs an engine host, and `crates/dictation/src/meeting.rs` doesn't use the meeting record yet. Details and API in `wispr-free-notes/16-meeting-transcript-summary.md`.
