# What the UI shows today and what it needs from the core

Answers ticket `issues/04-surface.md`. Read from the code at `ef09446`. `ui/` means `crates/app/ui`, `src/` means `crates/app/src`.

## Pill

- Window 320×120, always there, transparent. Polling 16 ms while animating or under the cursor, 40 ms otherwise, 250 ms when Hidden, displays every second.
- States and capsule sizes:

| State | Size | Does |
| --- | --- | --- |
| Hidden | none | Toasts can still show |
| Resting | 32×4 (4×32 on the sides) | Grows on hover |
| Ready | 42×18 | Click toggles hands-free, right click opens the menu |
| ListeningPtt | 60×20 | Bars |
| ListeningHandsFree | 100×24 | Cancel, bars, Stop |
| Processing | 42×18 | Ring, 1400 ms a turn |
| Error | Red | Tooltip, 6 s |
| Meeting | 50×18 | Green, bars at 0.85 |

- Precedence: recording, processing, error, blocked, meeting, resting.
- Drag past 4 pt, snap to the nearest edge, saved in `pill.toml`, Esc cancels the drag.
- Menu: Paste last, Recent ▸ (48 chars), Microphone ▸, Language ▸ (Auto, English, Italiano), Pipeline quality ▸, Meetings, Settings, Hide for 1 hour, Quit.
- Card 236×64, 30 s, shown even when the pill is off. Used for "take notes?" and "copy your text?".
- Toast 4 s, beats the error tooltip, shows when Hidden.
- Sounds are played by the app behind `prefs.sounds`.
- `wf_core::pill::Pill` is pure already: `tick(Tick) -> Frame` gives the capsule, the card, the move and whether the window catches the mouse.

## Menu bar icon

"Finish setup…" (bundle mode), Start or Stop meeting notes, Debug pill ▸ (debug builds), Meetings…, Settings…, Quit wispr-free. Quit runs `close_media(3 s)` and `close_meetings(5 s)`.

## Settings

880×640, minimum 760×480, 8 pages, a notice banner on top, no Save: every change commits at once.

- **General:** 6 shortcuts with Change and Reset, recording times out after 15 s idle. Microphone auto, ranked list, test. My languages, a grid of 15. App language (nothing reads it), Theme, Launch at login, Show pill, Show in Dock.
- **Models:** This computer, Models folder with its move, pending and missing flows, Downloaded, Available, `ModelLine`.
- **Pipeline:** tier, stages, models, Keep models loaded, per-stage `ModelState`.
- **Style & dictionary:** 4 styles, dictionary, auto-add, press enter, replacements, snippets.
- **Meetings, Sounds & media, Privacy & data** (context, history, export all, delete all with confirmation), **About**.

## Onboarding

760×620, 4 steps: Microphone, Keyboard and paste, Models, Try a dictation. Bundle mode only. The first run shows all steps, later runs only broken ones. Re-checks every 1 s open, 5 s closed. "One more thing" after 8 looks. Trial messages name the failed part. The Models step closes Settings, and opening Settings while onboarding is up raises onboarding.

## Meetings

980×680. Sidebar: start and stop, search (150 ms debounce), list with marks. Header: title (saves after 700 ms), Keep recording, Export, Delete, stage chips, Finish processing, progress, failures. Tabs: Summary, Notes, Transcript, Chat (streaming, `TimeChips`, suggestions, Stop).

## What the core offers

- `Dictation`: `start(Parts)`, `on_key`, `on_pill`, `settings_changed`, `set_blocked`, `paste_last`, `paste_recent`, `copy_text`, `toggle_meeting`, `resume_meeting`, `finish_meeting`, `summarize_meeting`, `cancel_meeting_work`, `start_trial`, `arm_trial`, `pause_keys`, `meetings_status`, `ask_meeting -> ChatRun`, `forget_history`, `close_meetings`, `close_media`.
- `Event`: Pill, Level, Toast, Recent, Trial, TrialFailed, Inserted, WordLearned, Unloaded, Meeting, SuggestMeeting, NoTextField, Meetings(`MeetingNews`: ListChanged, Recording, Stopped, Live, Working, WorkDone, Chat).
- `SettingsStore`: update takes a closure, so it can't cross the bridge as it is. `Settings` is plain serde data.
- `wf_models`: Catalog, ModelStore (`download`, `move_to`, `verify` are long, with cancel and progress), hardware detection.
- `wf_platform`: permissions, desktop, autostart. `keys::listen` runs on its own `wf-keys` thread and needs nothing from the main run loop. `paste_key_code` asks the main queue with a 100 ms timeout.
- `wf_audio`: devices, Capture plus LevelMeter, sounds.
- `wf_meetings`: the library is blocking disk work, ordered on one thread.
- Shortcut capture is `pause_keys(true)` plus a second `keys::listen`.

## Where the current app code goes

- **Toolkit-free logic, moves:** `onboarding/{steps,trial}.rs`, `settings_window/data.rs`, `prefs.rs`, `memory.rs`, `meetings_window/disk.rs`, the logic in `values`, `rows`, `system`, `host` and `dictation.rs`. The seams `Desktop`, `Machine` and `MeetingActions` already exist.
- **Slint glue, dies:** the rest, including `overlay::apply` and `raise`, and 3,932 lines of `.slint`.
- **Modes in `main.rs`:** `--version`, `--engine-host`, `--probe-clip`, `--pill-state`, `--debug-pill`, `--settings`.

## Tests

`crates/app` has 215 tests. 142 are window tests that need rewriting (settings 72, onboarding 38, meetings 32) and 17 are Slint-specific. Their labels are a usable accessibility contract. `settings_probe persist-*` is worth keeping. `real_dictation`, `real_meeting` and `engine_host` keep their meaning.

## What assumes Slint

`crates/app/build.rs` (the weak CoreAudio link stops mattering with a macOS 26 floor), `.cargo/config.toml` rpath, binary name and icon path in `bundle.rs`, the `--version` smoke test, `Info.plist`, CI, the About attribution, `README.md:28,30`, `.scratch/v1/spec.md:5,207`, and the own-window skip at `crates/platform/src/context/macos.rs:30`.

## Mismatches found

- `app_language` has no reader.
- System Audio isn't in onboarding.
- The pill's Language menu has 3 entries, Settings has 15, and choosing from the menu overwrites My languages.
- The tier crosses windows as a Debug string.
- The language and tier lists are duplicated.
- Model downloads are tied to the window.
- CI's Windows and Linux jobs and `crates/platform/src/overlay/{windows,x11}.rs` lose their consumer.
- Engine hosts need a helper binary.
