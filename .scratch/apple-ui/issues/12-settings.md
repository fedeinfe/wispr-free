# 12 Settings
Status: ready-for-agent
Blocked by: 11

Spec: Settings, Settled mismatches, Bridge (long jobs move to `Shell`), Phase 2, Memory. Behaviour: `research/surface.md`.

All 8 pages in `Form(.grouped)`, every change commits at once. Shortcut capture with `record_shortcut` and `stop_recording_shortcut`. Models flows with `catalog`, `hardware`, `download`, `cancel_download`, `move_folder`, `verify`, owned by `Shell` so they outlive the window. `NSOpenPanel` replaces `rfd`, then re-check the framework list. Data: `export_all`, `delete_all`, `forget_history`, with `confirmationDialog`. The notice banner becomes a top `Section`.

Remove App language and Theme, both control and field, and an old `theme` key in `settings.toml` is ignored. Tier as a UniFFI enum. One language list and one tier list from Rust, and the pill's Language menu shows all 15.

Port `settings_probe persist-*`. Run the Settings design review.

Done when: the ported tests and `ax-check` pass on every page, persistence is proven to reach disk, footprint after use is recorded, and shots of every page in both appearances are read by you.

## Comments

### 2026-10-31, implementation

Done: all 8 pages are `Form(.grouped)` in `apple/Sources/WisprFree/Settings/`, and every change commits at once. `Pages` stays the one logic source and now runs on `Shell`'s own `Desk` thread (`crates/shell/src/settings/desk.rs`), so downloads, moves and verifies outlive the window. Swift gets the pages as one `PagesState` record (`state.rs`). `SettingsStore` owns the revision and the compare and swap. Tier is a UniFFI enum, tier changes and the other ruled changes go through `change_setting`, there's one language list and one tier list from Rust, and the pill's Language menu shows all 15. App language and Theme are gone, control and field, and an old `theme` key is ignored and dropped at the next save. `NSOpenPanel` replaces `rfd` in the Swift app. `persist-*` is ported to `crates/shell/tests/persist.rs`, two child processes.

The second risk is closed. `ModelStore::fenced` is an allowlist, set from `Paths::model_fence` to the run's own data folder, because the real models folder is named only in the real settings, which a dev run can't read. `delete`, `download` and `move_to` refuse outside it, links are resolved first, and `..` is refused. `Paths::check` now compares resolved paths too, which closes a link into the real root.

Verified on 08be475, with `apple-ui` at 9a1ff4c already in it (the merge had nothing to bring in):
- `cargo test --workspace --locked --no-fail-fast`: 1370 passed, 0 failed, 86 ignored. `swift test`: 104 of 104. `clippy -D warnings --all-targets` and `fmt --check` clean.
- `cargo xtask ax-check` on a fresh dev bundle: Settings, Meetings, Onboarding, all 8 pages (General 12 labels and 4 presses, Models 5 and 2, Pipeline 11 and 1, Style 10 and 3, Meetings 7 and 1, Sounds 2 and 1, Privacy 5 and 2, About 1 and 1), then the rule. No focus taken.
- On 703eeab, one commit earlier: `dev_bundle` ignored tests 2 of 2, `cargo xtask bundle` for the Slint app (not installed), `codesign --verify --strict` on both bundles. The last commit only removes unused imports in the Slint app and changes test and probe lines.
- Persistence reaches disk: one process changes every setting, a new one reads them all back from `settings.toml`. A break that skipped the write failed this test and 5 in `wf-core`.
- Footprint, also on 703eeab: idle 21.4 MB against 25. After one round of every page, closed for 20 s, 51.4 MB, peak 55.3. After six rounds 52.5 MB, peak 59.3. Both under 55, with 2.5 MB to spare at most.
- Framework list: still 21 plus `iconv`, and the check against `--print native-static-libs` passes on every bridge build. `rfd` is out of `wf-shell`'s tree, and stays in `crates/app` until ticket 15. As ticket 09 found, CoreData and CloudKit don't go with it.
- Shots: I read every page in both appearances, with the sidebar at 210 pt and the pill sheet unchanged. Pictures in the notes folder, `apple-ui/12-settings/shots`.
- 13 deliberate breaks, each restored from git, baseline green first. Rust test binaries ran directly under a temp `HOME` with no `WF_*` variables, so a fallback couldn't reach the real folder: fence off, fence ignoring links, fence allowing `..`, `check` ignoring links, no fence in a dev run, stale save landing, save skipping the disk, closing Settings cancelling a move, ruled change not kept. Swift: ruled change reaching nothing, notice never showing, Delete all skipping its dialog. Bundle: a renamed label, caught by `ax-check`. All caught, but the Delete all break got past all 104 tests the first time. Nothing pressed the real button, so I added that to both delete tests and reran the break, which now fails.
- `stat` only: the real data folder and its `settings.toml` kept their times from before the breaks to the end, and no `wispr-free-dev` was left in the real home.

The shots found four things no test did: the Add rows after a `ForEach` fell out of their cards and pulled the footer in, so they now sit above the list as in Dictionary; a model's fourth line turned tertiary; the Summary template was right-aligned; and a page taller than the screen never showed its end, so `shots` now also takes `-end` pictures, scrolled down.

Couldn't verify:
- `NSOpenPanel` and the `confirmationDialog`s were not shown on the real bundle, because either could take the focus. The dialogs' actions are tested in ViewInspector. The `.destructive` role is only read from the code.
- Real key capture, the microphone test and the Dock icon, because a dev run under test takes no keys or microphone. Launch at login is refused in a dev run. No break was run on these guards: it would have touched the real keys or `~/Library/LaunchAgents`.
- Reset in the context menu isn't reachable from the AX walk.
- Downloads, because there's no network here. A move proves jobs outlive the window.
- Copying the export folder into itself: I didn't break that guard, as it could fill the internal disk.
- One stall I can't explain. A six-round tour under `cargo xtask footprint` didn't finish in 120 s, and its log went with its temp home. The same binary then finished 10 of 10, at 26 to 27 s, so it isn't simple slowness. Closing is free of races (main actor, `closed` is synchronous). `Desk::ask` can block an opening for up to 10 s, which would fit the time, but that's a guess. `footprint` now reports the app's log and kills the app when a tour fails, as it left it running before.
- The shots are of a window that isn't key, so the sidebar's selection in a key window wasn't seen.

Decisions the spec didn't cover:
- `catalog` and `hardware` are `PagesState` fields, not calls.
- Pages take `editor` and `pages` as properties, not `@Environment`, so ViewInspector can reach them.
- A model's Delete also gets a `confirmationDialog`, with new copy "You can download it again.", under the spec's destructive rule.
- The General card is now "Startup", as Appearance is gone.
- `Paths` gained `data` and `exports`, and a dev run's exports stay in its dev root.
- `DevDesktop` takes keys, microphone and `open` only by hand, and refuses launch at login, since `LaunchAtLogin::system()` writes the installed app's entry. **Ticket 15 must hand that to `SystemDesktop` and lift the fence and `check` on purpose.**
- The Add rows have a label ("New replacement", "New snippet") and an "Add" button, matching Dictionary.
- The microphone test level crosses as `ShellEvent::Level`.
- Option labels other than tiers stay in Swift, as plain labels.
- The Slint `Prefs` keeps a `theme` int for the probes only, and `pipeline.slint` keeps its own tier list, until ticket 15.

Design review, using `settings`, `sidebars`, `toolbars`, `materials`, and live `designing-for-macos`, `gestures`, `pointing-devices`:
- Settings guidance wants a toolbar of panes, the spec wants a sidebar. The spec wins. The rest holds: dimmed zoom, title follows the page, last page restored, ⌘, opens it.
- "Style & dictionary" is over the 15 characters advised for a title. The spec's page names win.
- The sidebar has no hide control or customising. Eight fixed pages in one level don't need them.
- No custom material or glass in the content layer, nothing custom for gestures, everything works from keyboard and pointer, except below.
- For ticket 16: Reset lives only in a context menu, which is harder to find than a visible control.

For ticket 16: try `NSOpenPanel`, both Delete dialogs, real key capture, the microphone test, the Dock icon and Reset's context menu by hand, and a real download.

Ideas, not done: the Swift test rig never removes its `wf-bridge-*` folders, about 3,100 in `$TMPDIR` at 50 MB, so a teardown would help. `Desk::ask` blocks each opening of Settings up to 10 s if the desk is busy, so watch that if a window ever feels slow.
