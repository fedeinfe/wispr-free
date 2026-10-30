# 12 Settings window (Slint)
Status: ready-for-agent
Blocked by: 03, 11

All pages in spec §5, including the shortcut recorder with the validation rules in §2. Created on open, dropped on close. Light/dark/system theme. Own palette and icon set.

Done when: every setting persists across restarts and closing the window releases its memory (measured).

## Comments

**2026-10-30 · implementer (branch `t12-settings-window`)**

Done:
- `wf-core::settings`: `Settings` for everything in spec §5 and `SettingsStore` (TOML, atomic write, missing fields default, a damaged file is set aside as `settings.toml.bad`).
- `wf-core::shortcut::recorder`: the capture loop as pure logic: commit on release, Esc alone aborts, reset to default. It reuses `validate`. Keys now have stable file names and per-OS labels. A recording ends after 15 s without a key, because it swallows all typing.
- `crates/app`: all 8 pages, light/dark/system theme, own palette and icons, "Settings…" in the tray and the pill menu, created on open and dropped on close (`SettingsHost`). The crate now has a lib target so tests and examples can reach the UI.
- Live today: theme, launch at login, Dock icon, Show pill, Sounds, shortcut recorder, mic ranking and test meter, models (download, cancel, resume, delete, verify), hardware summary, open data folder, export all, delete all, updates link.
- Stored but with no effect yet, because nothing reads them: shortcuts (dictation isn't wired to them), microphone ranking, languages, pipeline, styles, dictionary, meetings, media, context, retention. App language offers only System and English, since no translations exist.
- `wf-platform`: `autostart` (LaunchAgent, XDG entry, registry) and `desktop` (open with default app, Dock icon, activate).
- mimalloc is the global allocator: with system malloc, closing the window gave back about 2 MB, because freed memory stayed fragmented in the heap.

Measured on macOS (Apple Silicon), all offscreen with Slint's software rasterizer (`crates/app/examples/settings_probe.rs`), on the merged tree:
- **Every setting persists across restarts: verified.** One process set all settings through the window layer and exited, a second process read the file: 43 values compared, 0 differ, none equal to a default. Tampering with one value makes the check fail. Window tests also drive the real .slint controls and restart the controller.
- **Closing releases memory: only partly verified.** 20 open/close cycles with every page drawn: 5.8 MB at start, 18.5 MB open, 10.6 MB after every close, flat across cycles, so no leak. This is what the app itself holds. **The real app draws Settings with femtovg, whose GL memory, plus AppKit and the window surface, are not in these numbers.** See below.
- Every page rendered to PNG in light and dark and checked by eye.
- `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.

The final UI was never shown in a real window. The owner was using this Mac, so after an instruction during this ticket no window, synthetic input or screen capture was used. An early spike with a placeholder UI did open real windows from the shell, and they may have taken focus. It is probably the "Settings test app" mentioned in ticket 05's notes. No screenshots were taken and no input was sent.

Needs human verification (`cargo run --release -p wispr-free -- --settings`, or tray → Settings…):
- **Real window memory, which decides this ticket's done-when.** Note `footprint <pid>` with the app idle, open Settings, visit each page, close, wait 1 s, read it again, and repeat three times. Pass: the closed figure returns close to idle and stops growing. The spike saw about 200 MB for any femtovg window, while ticket 05 measured 33 MB idle on the same renderer, so the result is genuinely unknown. If it fails, the fallback in the notes is Settings in its own process.
- **Shortcut recorder with real keys.** Never run with real capture, since it swallows all typing. Check Change on each action, Fn, Esc alone, a reserved combo, a side mouse button, that the 15 s timeout gives the keyboard back, that the Stop button works with the mouse, and that the keys don't reach the app underneath.
- **Real window behaviour on femtovg:** the look of every page; typing, focus and caret in fields; scrolling; dropdown placement near the window edge; System theme following the OS; opening while already open brings it forward; the pill keeps working and stays non-activating while Settings is open.
- **Tray and pill menu "Settings", Show pill and Sounds taking effect, Show in Dock turning on and off:** wiring in `main.rs` compiled but never run.
- **Mic list and test meter:** real devices, levels moving (needs mic permission).
- **Launch at login:** the LaunchAgent starts the app after a real login. Windows and Linux entries have never run.
- **Model download from the page:** not run through the UI. The store underneath was tested in ticket 11.
- **Windows and Linux:** nothing in `crates/app` has been built there. CI has never run.
