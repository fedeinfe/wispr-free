# 12 Settings window (Slint)
Status: ready-for-agent
Blocked by: 03, 11

All pages in spec §5, including the shortcut recorder with the validation rules in §2. Created on open, dropped on close. Light/dark/system theme. Own palette and icon set.

Done when: every setting persists across restarts and closing the window releases its memory (measured).

## Comments

**2026-10-30 · implementer (branch `t12-settings-window`)**

Done:
- `wf-core::settings`: `Settings` for everything in spec §5 and `SettingsStore` (TOML, atomic write, missing fields default, a damaged file is set aside as `settings.toml.bad`).
- `wf-core::shortcut::recorder`: the capture loop as pure logic: commit on release, Esc alone aborts, reset to default. It reuses `validate`. Keys now have stable file names and per-OS labels.
- `crates/app`: all 8 pages, light/dark/system theme, own palette and icons, tray "Settings…" item, created on open and dropped on close (`SettingsHost`). Live today: theme, launch at login, Dock icon, shortcut recorder, mic ranking and test meter, models (download, cancel, resume, delete, verify), hardware summary, open data folder, export all, delete all, updates link. The other settings persist for their features to read later; they do nothing yet. App language offers only System and English, since no translations exist.
- `wf-platform`: `autostart` (LaunchAgent, XDG entry, registry) and `desktop` (open with default app, Dock icon, activate).
- **Renderer change, recorded in ADR-0002:** the app uses Slint's software renderer, because any OpenGL window costs about 200 MB permanently on macOS. mimalloc is the global allocator, because system malloc kept the freed memory.
- **Blocks ticket 05:** the software renderer can't draw a transparent window on macOS, and the renderer is per process. Details and options are in the notes file.

Measured on macOS (Apple Silicon), all offscreen through the production software renderer (`crates/app/examples/settings_probe.rs`):
- **Every setting persists across restarts:** one process set all settings through the window layer and exited, a second process read the file: 43 values compared, 0 differ, none equal to a default. Tampering with one value makes the check fail. 41 window tests also restart the controller and drive the real .slint controls.
- **Closing releases memory:** 20 open/close cycles with every page drawn: 5.9 MB at start, 19.9 MB open, 11.0 MB after each close, flat across cycles. The window's 9 MB comes back; about 5 MB stays once (fonts, audio and hardware libraries).
- Every page rendered to PNG in light and dark and checked by eye.
- `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. `wf-core` and `wf-platform` also pass clippy for Windows and Linux targets (typecheck only).

The final UI was never shown in a real window. The owner was using this Mac, so after an instruction during this ticket no window, synthetic input or screen capture was used. An early spike (a placeholder UI, not the final pages) did open real windows, which is where the OS-window numbers in the notes come from. Those windows may have taken focus; no screenshots were taken and no input was sent.

Needs human verification (`cargo run --release -p wispr-free -- --settings`, or tray → Settings…):
- **Real window memory.** The headless figures leave out the OS window. Expect roughly 6 MB more one-time AppKit cost, plus the window surface while open. Run the app, note `footprint <pid>`, open and close Settings a few times, wait 1 s after each close, compare. Don't loop `footprint` fast.
- **Shortcut recorder with real keys.** Never run with real capture, since it swallows all typing. Check Change on each action, Fn, Esc alone, a reserved combo, a side mouse button, that the 15 s timeout gives the keyboard back, and that the Stop button works with the mouse. Also check that the keys don't reach the app underneath.
- **Real window behaviour:** typing, focus and caret in fields; scrolling; dropdown placement near the window edge; System theme following the OS; tray item and window close button; opening while already open brings it forward; the window comes to the front with the Dock icon hidden, and the Dock setting takes effect.
- **Mic list and test meter:** real devices, levels moving (needs mic permission).
- **Launch at login:** the LaunchAgent starts the app after a real login. Windows and Linux entries have never run.
- **Model download from the page:** not run through the UI. The store underneath was tested in ticket 11.
- **Windows and Linux:** nothing in `crates/app` has been built there, and neither has the software renderer's look or memory. CI has never run.
