# 01 Workspace scaffold
Status: ready-for-agent
Blocked by: none

Create the Cargo workspace: `app` (binary, Slint UI, tray), `core` (state machine, settings, history), `audio`, `pipeline` (vad/asr/cleanup/llm traits + impls), `platform` (per-OS hotkeys, paste, context, overlay), `models` (catalog + downloader + hw detect), `meetings`.
Verify and pin the binding crates named in ADR-0003, then mark it accepted. Add CI that builds on macOS, Windows and Linux. Add a Slint attribution line to the README.

Done when: `cargo build` passes on all 3 OSes in CI and an empty tray app launches.
