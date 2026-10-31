# 15 Cutover on the branch
Status: ready-for-agent
Blocked by: 12, 13, 14

Spec: Phase 5, Build ("Also"), Testing (CI), Settled mismatches.

`cargo xtask bundle` now makes the Swift app under `com.wispr-free.app`, binary name `wispr-free`, `MINIMUM_MACOS` 26.0, `CFBundleIconName`. Delete `crates/app`, Slint, `wf-platform`'s `overlay/`, `crates/app/build.rs`'s weak link, and the Slint attribution. CI becomes macOS only. `real_dictation`, `real_meeting` and `engine_host` still pass. Flip ADR-0006 to accepted and ADR-0002 to superseded, edit ADR-0001 and ADR-0005, update `GLOSSARY.md`'s engine host entry, `README.md:28,30` and `.scratch/v1/spec.md:5,14,207,245`.

This stays on `apple-ui`. Don't install, and don't touch `v1` or `main`: `v1` keeps the last Slint source and the user's installed app stays the fallback, so the spec's "confirm grants survived" happens in ticket 16 before anything merges.

Done when: `cargo fmt --check`, clippy, `cargo test --workspace`, `swift build` and `swift test` pass, no `slint` remains in `Cargo.lock`, `cargo xtask eval` is unchanged, and the bundle passes `codesign --verify --strict` and reports the expected designated requirement.

## Comments

### 2026-10-31, implementation

Testing was cut to one pass on the user's instruction, and that limit overrides the ticket: no deliberate breaks, no `cargo xtask eval`, no `ax-check`.

**Done.** `cargo xtask bundle` now makes the Swift app as `target/Wispr Free.app`, `com.wispr-free.app`, binary `wispr-free`, `MINIMUM_MACOS` 26.0, `CFBundleIconName`, on the existing certificate. `crates/app`, Slint, `rfd`, `raw-window-handle`, `wf-platform`'s `overlay/`, the weak link and the Slint attribution are gone. CI is macOS only, on `macos-26`. ADR-0006 is accepted, ADR-0002 superseded, ADR-0001 and ADR-0005 edited, and `GLOSSARY.md`, `README.md`, `.scratch/v1/spec.md` and `docs/benchmarks.md` updated. The memory goal after every window is now 65 MB in the spec, ADR-0006 and `.scratch/v1/spec.md`. Idle stays at 25 MB.

**How real or dev is picked.** `crates/shell/src/launch.rs` is the one decision. `decide` is pure, and `this_launch` feeds it `NSBundle.mainBundle`'s id and the process's arguments, so Swift and every caller have no say. A run is real only when the id is exactly `com.wispr-free.app` and the only flag, if any, is `--debug-pill`. Everything else is a dev run, including the real bundle with any tool's flag.

- Real: `Shell::start_real(sink)` checks `this_launch` itself, so a test process gets `NotTheRealApp` before anything is touched. Then `real_paths()`, `check(&[])` (paths must still be plain), no model fence, `SystemDesktop`, plain `SystemMachine`. In Swift `real` sets `byHand`, which is what takes the keys, the mouse, `NSOpenPanel` and the Dock.
- Dev, unchanged: `Shell::start` and `Shell::rehearse` with `check`, the fence, `DevDesktop`, `DevMachine`.
- Tests stay on temp paths by construction. They aren't the real bundle, and `Shell::real_wiring`, the one way to test the real rules, refuses the real folders. `ax-check`, `shots` and `footprint` run the dev bundle under a temp `HOME`.

**Verified, one pass.** Logs are in `wispr-free-notes/apple-ui/15-cutover/`.
- `cargo fmt --check`, clippy `-D warnings --all-targets`: clean.
- `cargo test --workspace --locked --no-fail-fast`: 1297 passed, 0 failed, 89 ignored. `swift test`: 176 of 176, with ticket 14's one `try` warning left.
- `cargo xtask bundle` and `--dev`: both built, both pass `codesign --verify --strict`.
- `codesign -d -r-` on the real bundle: `identifier "com.wispr-free.app" and certificate leaf = H"ee05a347c2b649f0ce0e440144a96ca271d44b18"`. The installed app prints the same line, read only.
- `cargo test -p xtask --test dev_bundle -- --ignored`: 2 of 2. No `slint` in `Cargo.lock`, which lost 327 packages and gained none.
- `real_dictation`, one case, `fillers_come_out_clean_on_balanced`, on the SSD's models: passed, "So I think we should ship it."
- The decision by unit tests, ported from the old `cli.rs` too: only the real id, plainly started, is real; every tool's flag on the real bundle is dev; `--version`, `--engine-host` and `--probe-clip` end the process. A test in `xtask` holds the build's ids to the decision.
- Real rules in a child with a temp `HOME` (`tests/isolation.rs`): a Slint-era `settings.toml` with `theme` and `app_language` keeps everything else and loses those two at the next save, `pill.toml` is read, the login entry is written at start and follows the setting, and a model in a named folder outside the data folder is deleted. `real_paths` gives the folders the old app used, and exports go to Downloads.
- `stat` only, at the end: the real data folder last changed 02:23 and `settings.toml` on 30 Oct, both before this ticket. No `wispr-free-dev` in the real home. The installed app wasn't touched.

**Not verified.**
- The real bundle was never launched, on purpose. So real keys, the microphone, the pill catching the mouse, `NSOpenPanel` and the real data under the app are proven only by the decision, the wiring test and the `byHand` test. That real path has never run end to end, and ticket 16 is its first real run.
- Whether grants survive. The requirement matches, but that's still inferred.
- `cargo xtask eval`, skipped as told, since no pipeline code changed. `real_meeting` and the ignored `engine_host` tests, not run. `ax-check`, not run. Footprint wasn't measured again, so the 65 MB goal rests on ticket 14's numbers, which never included a real key listener.
- The new CI file hasn't run, and I don't know that a `macos-26` runner is available.

**For ticket 16**, on top of its list: first launch with the existing `settings.toml`, `pipeline.models_folder` on the SSD, launch at login on and off, a Dock icon at launch taking no focus, and what tickets 12 to 14 left open (`NSOpenPanel`, both Delete dialogs, key capture, microphone test, meeting start and stop).

**Decisions the spec didn't cover.**
- `--version`, `--engine-host` and `--probe-clip` moved to Swift's `main` through `Launch::Exit`. Without this, an older running app that starts this path as a host would have opened a second real app with real keys.
- The login entry is rewritten at each real start when the setting is on, as the Slint app did, because the app may have moved.
- A Dock icon at launch no longer activates, as with Slint. Changing the setting still does.
- With Slint gone, `SystemDesktop` became the Swift app's: Dock and folder picks go out as events. `DevDesktop` now only guards it.
- Removed as Slint-only: `flow::start`, `flow::listen_to_keys` and the key retry, both `system_parts`, `meetings::system_folders`, `theme` on `MeetingsParts`, `desktop::activate_app`, `desktop::set_dock_icon_visible`. `MakeMachine` lost its `Clock`. `wf-platform` gained `desktop::bundle_id`.
- `--settings` wasn't ported. On the real bundle it now means a dev run, as does `--by-hand` or `--pill-state`.
- `cargo fmt` also reformatted five files from ticket 14, which hadn't run it.
- Going back to the Slint build after a save loses only the two removed values, which it defaults.
