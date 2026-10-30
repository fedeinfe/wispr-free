# 19 Wire the dictation flow into the app
Status: ready-for-agent
Blocked by: 05, 07, 09, 10, 12, 15

Added during implementation: tickets 02–12 each built a stage in isolation, and `crates/app` calls none of them. Spec §8's first acceptance criterion and ticket 13's "try a dictation" step both need this.

Connect shortcut → state machine → capture → VAD → ASR → rule cleanup → LLM cleanup → local passes → insertion, with the pill showing state throughout and settings taking effect.

Loose ends from earlier tickets (details in each ticket's Comments and the shared notes):

- 02/06: expose `Recorder` pre-roll so `LiveVad` chunks keep the audio captured since key-down.
- 02: mic stream policy, decided 2026-10-30: the mic opens on key-down and closes when the dictation ends, so the OS indicator stays off while idle. Measure how much of the first syllable this loses.
- 07: VAD stays on whenever the engine is Whisper, whatever the setting says; show that in Settings. Decided 2026-10-30.
- 07: test one long clip with VAD off on Parakeet before allowing that combination.
- Models folder: build stores with `ModelStore::configured(settings.pipeline.models_folder, default)`, react to `on_change`, and turn `!available()` or a missing model into the pill's error state instead of a failed dictation. llama.cpp maps model files into memory by default, so unplugging an external drive under a loaded model can crash the app: check this and load into RAM from a chosen folder if so.
- 04: show the no-focus toast from the insertion outcome.
- 05: handle pill menu events (only Hide for 1 hour and Quit work today).
- 09: resolve the idle RSS problem after model unload (122 MB measured against the §8 limit of 40 MB).
- 10: add an Enter keystroke to all three OS backends so `press_enter` does something; run paste last and copy last end to end.
- 10/12: unify the duplicate `Style`, `Snippet`, `Replacement` and `HistoryRetention` types.
- 12: make Show pill, Sounds, Dock, shortcuts, microphone ranking and pipeline settings take effect.
- 15: add the meeting shortcut, tray item, pill state and opt-in suggestion card; call `repair_meeting` on leftover files at launch; detect and surface the denied-permission case, where the macOS tap records silence without an error.
- CI: build and test the `silero`, `llama` and ASR engine features, which no workflow covers today.

Done when: on macOS a shortcut dictates into a focused field through the real stages, and §8's "um so I I think…" and "red one… blue one" cases pass end to end from audio fixtures through to the insertion seam.
