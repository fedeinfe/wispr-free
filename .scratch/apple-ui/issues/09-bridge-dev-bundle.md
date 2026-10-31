# 09 UniFFI bridge and the dev bundle
Status: ready-for-agent
Blocked by: 08

Spec: Architecture, Bridge, Build. Proof: `prototype/stack/`. Facts and traps: `research/toolkit.md` §4 and §5.

- UniFFI pinned to exactly `0.32.2`, the `uniffi` feature in `wf-core`, the `Shell` object with Life, Dictation and Settings first and the other groups as their tickets need them.
- `EventSink`, two streams, compare and swap on `save`, `Result` wherever Rust can panic.
- `apple/` per the spec: `Package.swift`, `WfCore` without `defaultIsolation`, `WisprFree`, `Tests/WisprFreeTests`.
- `xtask` generates bindings, writes the modulemap, makes `WfBridge.xcframework`, runs `swift build`, runs `actool`, assembles and signs. Check the link list against `--print native-static-libs`.
- A dev mode of `cargo xtask bundle` makes `Wispr Free Dev`, id `com.wispr-free.app.dev`, while the default stays the Slint app.

**Data isolation, not in the spec and required.** The app's data root is hard-coded by name (see `implementation.md`), so a dev id alone would share the user's settings, history and meetings. `Shell.start(paths, sink)` must get every path from its caller, with no fallback to the real root, and the dev bundle uses a `wispr-free-dev` root. Add a test that fails if a dev start resolves any path under the real root. Reading existing model files is the one exception.

Done when: `swift test` passes round trips for settings, events, errors and compare and swap; the dev bundle builds, passes `codesign --verify --strict`, starts without activating, shows no window, and quits cleanly through `Shell.quit`; idle `phys_footprint` is recorded in your comment.

## Comments

### 2026-10-31, implementation

Done: UniFFI pinned to `=0.32.2`, the `uniffi` feature in `wf-core`, and in `wf-shell` the `Shell` object (`bridge.rs`) with `start`, `quit`, `on_pill`, `paste_last`, `paste_recent`, `copy_text`, `settings` and `save`, plus `EventSink`, `Versioned`, `SaveOutcome`, `ShellEvent` and `ShellError`. `paths.rs` holds `Paths` and `dev_paths`. `apple/` has `Package.swift`, `WisprFree` (`main`, `AppDelegate`, `EventStreams` with the two streams), `Tests/WisprFreeTests` and a first `AppIcon.icon`. `WfCore` and `WfBridge.xcframework` are generated and git-ignored. `cargo xtask bridge` makes what `swift test` needs. `cargo xtask bundle --dev` makes `target/Wispr Free Dev.app`, and `--dev --install` is refused. The default bundle stays the Slint app.

Data isolation:
- `Shell.start` gets every path in `Paths` and resolves none. It refuses a relative or `..` path (`UnclearPath`) and any path inside the real folders (`RealData`). `dev_paths` fails rather than guess if there's no home folder.
- `crates/shell/tests/isolation.rs` runs `Shell` in a child process with `HOME` on a temp folder, so a fallback lands in a seeded fake real root and the user's own stays out of reach. It checks the seeded files don't change, that nothing is read from them, and that nothing lands outside `wispr-free-dev`.
- `the_flow_gets_every_file_from_the_given_paths` pins what a run can't show. My first version seeded a loose recording to catch a read-only meetings fallback. That mutation survived, because a broken recording is never moved, so I replaced it with this test.
- Six deliberate breaks each failed a test: dev root set to the real one, no guard, guard ignoring unclear paths, settings, history and meetings on the real root.

Verified, on the tree with `apple-ui` at 8e9f3ec already in it (the merge had nothing to bring in). Logs are in `wispr-free-notes/apple-ui/09-bridge-dev-bundle/`:
- `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked --no-fail-fast`: 1262 passed, 0 failed, 80 ignored. The ignored count rose by 4: the three isolation children, which do nothing unless a parent test starts them, and the dev bundle test below.
- `swift test`: 14 of 14, with no link warnings: settings round trips including a restart, compare and swap, events from Rust's own threads, a flood of levels, errors, a shortcut Rust can't read thrown rather than fatal, `quit`.
- Compare and swap: six deliberate breaks in `Board` each failed a test in `crates/shell/tests/bridge.rs`, and making levels unbounded failed the flood test.
- `cargo xtask bundle --dev`, signed with Wispr Free Local: `codesign --verify --strict` passes, id `com.wispr-free.app.dev`, designated requirement `identifier "com.wispr-free.app.dev" and certificate leaf = H"ee05a347…"`, helper `com.wispr-free.wf-engine-host` answers `--version`, 6 dylibs in `Frameworks`, nothing but macOS linked.
- Started by running the binary directly, with `HOME` on a temp folder and once on the real one (`launch_check.swift` in the notes). Front app unchanged (`com.t3tools.t3code`), `isActive` false, accessory policy, 0 windows, SIGTERM reaches `Shell.quit`, exit 0 in 0.07 s. The real root's folder and `settings.toml` kept their modification times, read with `stat` only. I removed the `wispr-free-dev` that run created.
- **Idle `phys_footprint`: 10.3 MB, peak 10.5, 30 s after start.** This is the Rust flow, AppKit and no pill: no status item, no SwiftUI, no key listener. It's a floor for ticket 10, not a reading against the 25 MB goal.
- `cargo xtask bundle` still makes the Slint app, `com.wispr-free.app` on 13.0, and passes the strict check. Not installed. `~/Applications/Wispr Free.app` wasn't touched.
- `crates/xtask/tests/dev_bundle.rs`, ignored by default, keeps the real-bundle start and quit as a test for later tickets: `cargo test -p xtask --test dev_bundle -- --ignored`.

A bug this found: the first bundle hung on quit forever. `NSApp.terminate` from a block on the main queue runs its modal loop inside that block, so a `.terminateLater` that waited on the main actor could never get back. The reply now goes through `RunLoop.main.perform` from a detached task. The dev bundle test failed on the old build and passes on the new one. Ticket 10: this covers any `terminate`, including one called from a `Task`.

Couldn't verify:
- The Quit path through AppKit's own menu, which doesn't exist until ticket 10. Only SIGTERM has run it.
- That `async` calls stay off the main actor is read from the generated Swift (`uniffiRustCallAsync` is nonisolated, and `WfCore` has no default isolation), not measured. A `save`, or a `quit` of up to 8 s, holds one thread of Swift's global pool.
- No automated test covers "doesn't activate" or "no window": that's the notes probe. Ticket 11's rig is the place for it.
- Real models, a real dictation, the microphone and keys: nothing here uses them. `cargo xtask eval` wasn't repeated, since no code it runs changed. The Slint app wasn't launched. Linux and Windows CI weren't run, and the new `wf-shell` test files are macOS only because `isolation.rs` depends on macOS paths and `bridge.rs` makes the real inserter.

Decisions the spec didn't cover:
- **`Pages` stays out of `Shell`.** It's built on `Rc` and shaped like the windows, so it can't be a UniFFI object. `save` takes a whole `Settings` and applies no rule. So ticket 12 must not set `pipeline.tier` from Swift: `choose_tier` and the tier rules in `apply_values` have to reach Swift as Rust calls, and its long jobs need to move into `Shell` as the spec says.
- `pill_tick`, `pill_press` and `pill_release` aren't here. Their logic is in `PillController::poll` in `crates/app`, which ticket 08 left for ticket 10, and a wrapper now would guess its shape. So `Paths.pill` is unused until then.
- `Trial`, `TrialFailed`, `Inserted` and `Meetings` don't cross yet: tickets 13 and 14 bring what they need. `WordLearned` and `Unloaded` are handled in Rust.
- `Saved` carries the new `Versioned`, which the spec leaves out. A failed disk write returns `Disk` and still holds for this run, as `SettingsStore` does today, with the revision moved on and `SettingsChanged` sent.
- `edit { $0.x = y }` is ticket 11's, so the tests call `save` directly.
- The refusal of the real root stays until the cutover. **Ticket 15 must remove it on purpose**, along with adding the real paths. It compares paths as text, so a symlink inside the dev root would get past it.
- The real models exception: `Paths.models` can't be in the real root, but a dev `pipeline.models_folder` may name any folder, including that one. Ticket 12 needs care here: a download or folder move from a dev run would write there.
- The bindings come from UniFFI's generator called as a library in `xtask`, not an `uniffi-bindgen-swift` binary. Same generator, and nothing else to build.
- `MACOSX_DEPLOYMENT_TARGET=26.0` for the library, or every object says 27.2 and `ld` warns. `opusic-sys` and `ring` don't rebuild on that change, so a `target` from before it needs `cargo clean --release -p opusic-sys` and the same for `ring`, once. The Slint bundle then shares those objects. That's harmless on this Mac.
- `AppIcon.icon` is the existing art in the probe's `icon.json`, enough for `actool`. Ticket 11 owns the real one.
- SIGTERM stands in as the quit request, since there's no menu yet.

Spec corrections and findings:
- The link list is 21 frameworks plus `iconv`, checked against `--print native-static-libs` on every bridge build. Moving the picker off `rfd` didn't drop CoreData or CloudKit, because they come from `objc2-app-kit`'s default features in `wf-platform`. Turning those off is the way to lose them. Idea only, not done.
- **Correction to ticket 08:** its "no llama, whisper or sherpa symbol" came from `nm`, but Xcode's `nm` fails on rustc's objects, so it can't show that. The bridge now lists the archive with `ar t`: 694 members, none from an engine. `xtask` fails the build if one appears, which catches the library `cargo build --workspace` leaves.
