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

## Comments

**2026-10-30 · implementer (branch `t19-dictation-wiring`)**

Scope changed during the work: the user parked Windows and Linux and the CI engine coverage. Done and verified means macOS only. Nothing was pushed, and `main` and `v1` are untouched.

Done:
- New crate `wf-dictation`: the whole flow, with no Slint, driven through seams for mic, engines, context, insertion, clock and the meeting OS calls. `crates/app/src/dictation.rs` only connects it to the pill, tray, keys and settings.
- Each engine runs in its own child process (ADR-0005). The brief said worker threads hold the engines. They hold proxies instead, because in-process unload left 67 to 452 MB behind.

Loose ends:
- 02/06 pre-roll: closed by construction. Every frame from the moment the mic opens reaches `LiveVad`, and frames wait in a queue while engines load. No `Recorder` pre-roll was needed.
- 02 mic policy: done, opens on key-down and closes when the dictation ends. **The time to the first frame is not measured: this Mac Studio has no input device.** `cargo run -p wf-audio --release --example first_frame` gives it on a real mic. What a late mic costs was measured by cutting the start of one `say` sentence: Balanced keeps every word up to 200 ms late and loses "Please" at 300 ms; Lite keeps every word up to 150 ms and loses it at 200 ms.
- 07 VAD forced for Whisper: done, shown in Settings.
- 07 Parakeet with VAD off: tested and left allowed. 9.1 min clip, all 1757 words returned, recall 0.978, the end not lost, 20 to 25 s from release to paste, ASR host peak 2.4 GB against 1.9 GB on a short clip.
- Models folder: done, including changes without a restart. llama.cpp reads the model into memory when a folder was chosen, and the engines' processes keep any bus error away from the app. Real test: models on a disk image force-detached under loaded engines gave the pill error "Models folder not connected: ...", no engine died, and dictation worked again after reattaching.
- 04 no-focus toast: done.
- 05 pill menu: done.
- 09 idle memory: done, see the numbers below.
- 10 Enter keystroke: macOS verified at the seam only, because no real keystroke may be sent here. The Windows and Linux backends were already written before they were parked and are neither run nor verified. Paste last and copy last run end to end to the insertion seam.
- 10/12 type unification: done.
- 12 settings: done. Dock was already live.
- 15: done: shortcut, tray item, pill state, opt-in suggestion card, repair at launch, denied-permission detection (silence on the system channel for 5 s while another app plays).
- CI: **parked by the user.** `.github/workflows/ci.yml` is unchanged. See the risk below.
- Windows and Linux: **parked by the user.**

Measured (Apple M5 Max, release build, shared machine):
- Release to paste, 10 s clip: Balanced 0.90 to 1.31 s warm, 1.55 s cold. Lite 0.27 to 0.35 s warm, 1.37 s cold.
- `phys_footprint` of the app, the metric tickets 01, 05 and 12 use for the 40 MB target: 28 to 30 MB idle after start, 29 to 32 MB after unload, 30.3 MB after 30 dictations with Balanced. `ps` RSS reads 70 to 105 MB because it counts shared frameworks, so the literal "RSS < 40 MB" in §8 is not met and the footprint one is.
- Footprint reads about 226 MB in the seconds after start and after each dictation, while the pill draws, and settles within 20 s. The cause was not investigated. The engine hosts are gone by then.
- Models loaded and kept (Balanced): VAD 40 MB, ASR 1.7 to 1.9 GB, LLM 1.2 GB.
- llama.cpp load: 268 to 309 ms into memory, 142 ms with mmap once cached.
- Real binary: 38 MB, starts from `/tmp` with no library path, and `cargo run -p wispr-free -- --version` works. Engine hosts exit when the app is force-killed. The frontmost app did not change during the runs.
- Killed hosts: with all three killed under a running app, the next dictation shows "Couldn't detect speech: the voice detection engine stopped", the one after pastes rule-cleaned text because the LLM host is only then found dead, and the third is back to normal.

Verified: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --locked --no-fail-fast`: 603 passed, 0 failed, 66 ignored; `real_dictation` 10 of 10 with real models, including both §8 sentences on Balanced and the filler sentence on Lite; pipeline real-model tests 51 of 51; `cargo xtask eval` 73/80 = 91.2%.

Not verified, needs a human on macOS:
- A real shortcut, a real mic and a real paste into a real app. Nothing here pressed a key or used a mic. The `--probe-clip` run replaces all three and goes through the rest of the real app.
- Time to the first frame on a real mic, and whether the last syllable survives release (there is no post-roll).
- A real meeting: `MeetingRecorder` through `SystemMeetingRoom` was not started, since it could raise a permission prompt. `other_app_playing` was checked against `afplay` and the card's buttons through Slint's test platform, but nobody has looked at the card on screen.
- High tier: its models aren't in the test folder.

Not done, and why:
- `media.while_dictating`, `style.auto_add_corrections` and `meetings.max_length_minutes` are saved by Settings but do nothing yet. Each needs work beyond this ticket: pausing other apps' media, watching edits after a paste, and a timer that ends a meeting.
- A meeting still running at Quit may not finish cleanly, because nothing waits for it. Repair at the next launch covers that file. This path was reasoned through, not tested.

Risks:
- CI builds the engines on all three OSes now, because `wf-dictation` turns the features on and workspace builds unify them. The workflow wasn't touched, so the Linux and Windows jobs may fail or run much longer.
- ADR-0003's Windows `ggml.lib` link risk is still untested.
- `wf-platform` still passes cross-target clippy for Windows and Linux. `wf-meetings` can't be checked that way without stubbing its native builds, so its two small `other_app_playing` stubs have not been compiled.
- `ClipRun` adds a hidden `--probe-clip` flag to the shipped binary. It replaces the mic, keys and paste with a WAV and a log. Everything else is the normal app, so history goes to the normal data folder unless `HOME` is set to a temp folder.
- Finished meetings go to `<data dir>/meetings/<unix time>.ogg`. Ticket 16 should check that this is the location it expects.
- Secure Input can leave keys looking held, and on Windows a new push-to-talk held while the last paste fires gives Ctrl+Win+V. Neither is handled.
- `Ah,` opening a sentence is now removed as a filler.
- LLM cleanup can drop a word from a fragment. Depending on pauses in the audio, Balanced turned the fixture's "C. Fillingham, Coxwell, M. D." into "C. Fillingham, M. D." with no fallback. The eval is at 91.2%, so this is known territory, but it is a lost word.
