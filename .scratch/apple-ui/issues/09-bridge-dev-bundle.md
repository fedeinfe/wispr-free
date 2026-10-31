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
