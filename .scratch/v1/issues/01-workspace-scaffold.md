# 01 Workspace scaffold
Status: ready-for-agent
Blocked by: none

Create the Cargo workspace: `app` (binary, Slint UI, tray), `core` (state machine, settings, history), `audio`, `pipeline` (vad/asr/cleanup/llm traits + impls), `platform` (per-OS hotkeys, paste, context, overlay), `models` (catalog + downloader + hw detect), `meetings`.
Verify and pin the binding crates named in ADR-0003, then mark it accepted. Add CI that builds on macOS, Windows and Linux. Add a Slint attribution line to the README.

Done when: `cargo build` passes on all 3 OSes in CI and an empty tray app launches.

## Comments

**2026-10-30 · implementer (branch `t01-workspace-scaffold`)**

Done:
- Cargo workspace (resolver 3, edition 2024) with `crates/{app,core,audio,pipeline,platform,models,meetings}`. Packages are `wispr-free` (bin) and `wf-*`, since a package named `core` would shadow std's `core`. Every external version is pinned in the root `[workspace.dependencies]`. Native engines (whisper-rs, llama-cpp-2, sherpa-onnx) are pinned there but not used by any member yet, so the scaffold builds in seconds.
- `wf-core::audio`: the pipeline format (16 kHz mono f32, 40 ms = 640-sample frames) and `AudioClip`. `wf-pipeline`: object-safe `Send` traits `Vad`, `Asr`, `Llm` (streaming tokens), plus `PipelineError`. The other crates are documented stubs naming the tickets that fill them.
- The app is an empty tray app using Slint 1.18's built-in `SystemTrayIcon` (no `tray-icon` crate; Linux uses ksni/D-Bus, no GTK), with a "Quit wispr-free" menu item. `--version` prints and exits without starting the UI.
- ADR-0003 is accepted with a verified version table. Refinements: Silero VAD runs through sherpa-onnx rather than `ort` (avoids a second onnxruntime), the official `sherpa-onnx` crate is used, and a duplicate-ggml warning is recorded for tickets 07/09.
- README: added the "Made with Slint" badge (Slint Royalty-free License 2.0, attribution option b). The in-app `AboutSlint` widget belongs on the About page in ticket 12.
- CI in `.github/workflows/ci.yml`: fmt check, then clippy `-D warnings`, build and test on macos-latest, windows-latest and ubuntu-latest.
- Tests (test-first) at three seams: the binary's `--version`, the audio format and clip duration, and the engine traits driven as boxed objects on a worker thread.

Verified on macOS (Apple Silicon): `cargo build`, `cargo test`, `cargo clippy -D warnings` and `cargo fmt --check` all pass. The tray icon appears in the menu bar (screenshot-checked) and the app idles without errors. Release binary: 11 MB, idle `phys_footprint` about 14 MB. A throwaway probe linking whisper-rs (metal), llama-cpp-2 (metal) and sherpa-onnx builds and runs.

Needs human verification:
- CI has never run: nothing was pushed. Check that Windows and Linux build green on the first push. Linux apt deps are a best guess (`libfontconfig1-dev libxkbcommon-dev`).
- The tray icon on Windows and on Linux desktops (GNOME needs the AppIndicator extension for SNI icons).
- The "Quit" menu item was not clicked (no GUI automation used); check that it exits.
- On macOS a Dock icon may appear. Hiding it (accessory activation policy) is part of the "show in Dock" setting (ticket 12).
- The tray glyph is neutral gray because Slint 1.18 doesn't expose template images or the menu bar's appearance. Polish later.
