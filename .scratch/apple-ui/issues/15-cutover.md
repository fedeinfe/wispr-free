# 15 Cutover on the branch
Status: ready-for-agent
Blocked by: 12, 13, 14

Spec: Phase 5, Build ("Also"), Testing (CI), Settled mismatches.

`cargo xtask bundle` now makes the Swift app under `com.wispr-free.app`, binary name `wispr-free`, `MINIMUM_MACOS` 26.0, `CFBundleIconName`. Delete `crates/app`, Slint, `wf-platform`'s `overlay/`, `crates/app/build.rs`'s weak link, and the Slint attribution. CI becomes macOS only. `real_dictation`, `real_meeting` and `engine_host` still pass. Flip ADR-0006 to accepted and ADR-0002 to superseded, edit ADR-0001 and ADR-0005, update `GLOSSARY.md`'s engine host entry, `README.md:28,30` and `.scratch/v1/spec.md:5,14,207,245`.

This stays on `apple-ui`. Don't install, and don't touch `v1` or `main`: `v1` keeps the last Slint source and the user's installed app stays the fallback, so the spec's "confirm grants survived" happens in ticket 16 before anything merges.

Done when: `cargo fmt --check`, clippy, `cargo test --workspace`, `swift build` and `swift test` pass, no `slint` remains in `Cargo.lock`, `cargo xtask eval` is unchanged, and the bundle passes `codesign --verify --strict` and reports the expected designated requirement.

## Comments
