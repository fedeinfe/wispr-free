# 10 Pill, card, toast, menus and sounds
Status: ready-for-agent
Blocked by: 09

Spec: Pill and card, Menu bar icon, Architecture (panel recipe, Swift timer calling `pill_tick`), Settled mismatches (the Language menu), Phase 1, Memory. Look reference: `prototype/Proto.swift`, `prototype/pills-*.png`.

Every pill state with its sizes and timings from `wf_core::pill`, card, toast, drag, snap, `pill.toml`, Esc, `ignoresMouseEvents` from `Frame.hit_test`, real `NSMenu`, sounds behind `prefs.sounds`. `NSStatusItem` with a template image and the same items. The hidden main menu supplies ⌘, ⌘W and the Edit commands. `PillButtonStyle` on the pill and card. Reduce Motion crossfades.

Add `cargo xtask shots` for every pill state over its own backdrop, and a `cargo xtask` report of `phys_footprint`. Run the pill and menu bar design reviews.

Done when: `cargo test` and `swift test` pass; shots of every state are read by you and recorded in your comment; the footprint is under the spec's 25 MB idle goal or the gap is stated; the flow reaches Rust end to end through the bridge in a test with fake engines. The real-dictation and pointer hand checks for this phase go to ticket 16.

## Comments

### 2026-10-31, implementation

Done: the pill's logic moved out of the Slint `PillController` into `crates/shell/src/pill.rs` (`PillCore`), and the Slint app is now a client of it. `menus.rs` holds both menus as data. `Shell` gained `pill_tick`, `pill_press`, `pill_release`, `pill_answer`, `pill_menu`, `status_menu`, `menu_action`, `stage`, `listen_to_keys` and the `rehearse` constructor, and `Paths.pill` is in use. `apple/` has the panel, `PillView` with every state, card, tooltip, `PillButtonStyle`, the status item with a template icon, real `NSMenu`s and the hidden main menu. `cargo xtask shots --out dir` and `cargo xtask footprint` are in `crates/xtask/src/dev_app.rs`.

Verified on this tree, which has `apple-ui` at dfd27ee in it (the merge had nothing to bring in). Logs and shots are in `wispr-free-notes/apple-ui/10-pill-menu-bar/`:
- `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked --no-fail-fast`: 1301 passed, 0 failed, 81 ignored. The ignored count rose by 1, the new dev bundle test.
- `swift test`: 35 of 35, no warnings.
- **End to end with fake engines, twice.** `EndToEnd` in `PillTests.swift`: a click sent to the real panel's `sendEvent` hits `PressView`, crosses the generated Swift to `Shell`, runs the real flow on `rehearsal.rs`, and the levels, the states and the words come back. The panel docks to a made-up screen at x -20000, so it shows nowhere. `a_click_on_the_pill_dictates_end_to_end` does the same in Rust.
- `cargo test -p xtask --test dev_bundle -- --ignored`: 2 of 2. New: `--pick="Quit wispr-free"` presses the real status menu item, so Quit now runs through AppKit's menu, `Shell.quit` and a clean exit, which ticket 09 couldn't check.
- Launch check from ticket 09's notes, on a temp `HOME`: front app unchanged, `isActive` false, accessory policy, 1 window (the pill), exit 0 in 0.07 s.
- **Idle `phys_footprint`: 19.8 MB, peak 20.3, on the final build, 30 s after start.** Earlier builds read 20.0 and 22.0. All are under the 25 MB goal. It has the pill, status item and Rust flow, but no key listener, and no menu had been opened.
- `cargo xtask bundle` still makes the Slint app, `com.wispr-free.app`, and passes the strict check. `--dev` passes too. Neither was installed, and `~/Applications/Wispr Free.app` wasn't touched.
- Twelve deliberate breaks in Rust each failed a test (`rust-breaks.txt`). Of six in Swift, five failed a test (`swift-breaks.txt`). The sixth showed my guard against redundant redraws did nothing, since `@Observable` already skips an equal value, so I removed the guard and made the test also prove a real change redraws.

Shots, read by me: `shots/{bottom,left,right}/sheet.png`, 11 scenes over 3 backdrops on each edge. Hidden shows nothing. Resting, ready, listening, locked, processing, error, meeting, both cards and the toast are all right, with the capsule, card and tooltip placed as `wf_core::pill` says on every edge. The text fits at `.body`. `menu-bar-icon.png` is the capsule with three bars.

Bugs this found:
- `isFloatingPanel = true` resets the level, so the panel sat at 3 instead of 1000. The recipe test caught it, and the order is now pinned in `PillPanel`.
- A 0.5 pt stroke loses its straight edges at 1x, read from the pixels, and both of this Mac's displays are 1x. The hairline is now 0.5 pt, or one pixel on a 1x screen. **So a true 0.5 pt hairline has never been seen here.** Check it on a Retina display.
- A tooltip with no edge vanished into a dark window. It now has the skin's hairline, which Slint's didn't.

Couldn't verify:
- Everything needing a person, which goes to ticket 16: a click through the window server, the right-click menu opening from the never-key panel, the first click, full-screen apps and Spaces. `popUpContextMenu` itself has never run, only the menu it would show.
- The status item on the real menu bar and its open menu. The bundle creates it, but I didn't look at it, and the items ran through `performActionForItem`.
- ⌘, ⌘W and the Edit commands through a real key press. Only their items and selectors are tested.
- Real dictation, keys and microphone. `--by-hand`, `Shell.listen_to_keys` and "Use real mic level" have never run.
- Sounds aloud. The cue choices are tested on a recorder and `sounds::play` isn't called in any test.
- Reduce Motion on screen. The animation choice is tested, not the crossfade.
- Footprint with the key listener, or after a menu has opened.
- The Slint app wasn't launched, and `cargo xtask eval` wasn't repeated, since no code it runs changed.

**A slip of mine:** the deliberate break "prefs from the real root" made the unisolated `pill` tests read the real `pill.toml` from `~/Library/Application Support/wispr-free` for that one run. The rule says never to read it. Writes stayed on temp paths, and `stat` afterwards shows `pill.toml` and `settings.toml` last changed on 30 Oct. A future break like that should run `--test isolation` alone.

Decisions the spec didn't cover:
- **`--by-hand`.** A plain start takes no keys and the pill never catches the mouse, because a test's pill sits exactly on the real one and a click there would open the microphone. For ticket 16: `open "target/Wispr Free Dev.app" --args --by-hand`. The dev root has no models, so `pipeline.models_folder` must be set in its `settings.toml`, and see ticket 09's warning about what a dev run could write there. The dev id needs its own grants.
- **The pill hears events in Rust**, on the flow's thread, in `Board::deliver`, and Swift only polls. They still cross as `ShellEvent`s. Nothing in Swift reads `levels` yet, as the bars come from `PillFrame`.
- **`Shell.rehearse` ships in the library.** It's a flow on fakes, about 250 lines, which is what lets a Swift test dictate without touching the Mac. It's never called in a normal run.
- `pill_release` and `menu_action` are `async` because they can touch the disk. Windows and quitting come back as `ShellEvent::Open` and `QuitAsked`.
- Spec conflicts with today's look, and the spec won: Resting is solid, where Slint's was 50%. Meeting has green bars and no green rim, where Slint's had white bars and a green rim, so "keeps" in the spec isn't quite right.
- "All 15 entries": the Language menu is Auto-detect plus the 15, since without it there's no way back to auto. Italian is now "Italian", from the one list. The Slint menu keeps its 3 until it goes.
- The Debug pill menu and `--pill-state` are ported, in debug builds or with `--debug-pill`. `MicLevels` moved to `wf-shell`.
- `stage` starts the made-up levels too, so a state set by `--pill-state` or the shots has moving bars. Slint's only did that for a menu pick.
- Meetings… and Settings… only log until tickets 11 to 14 bring their windows, and ⌘, the same. "Finish setup…" is absent until ticket 13 brings onboarding to `Shell`.
- `listen_until_allowed` retries like `listen_to_keys` but doesn't restart when permissions change. Ticket 13's watch does that.
- Shots use ScreenCaptureKit with the backdrop and panel behind every other window, not `screencapture`, so the user sees nothing. They need Screen Recording and stop with a message rather than prompting. The panel drops to `.normal` for them, the only way it differs from the real one.
- ViewInspector isn't added: it's a download, and logic tests on the real controller covered this ticket. Ticket 11's rig is the place for it.

Design review, in review mode, on `panels`, `motion`, `color` and `the-menu-bar`. The four live pages returned a JavaScript shell, so I read Apple's JSON data behind them. `app-icons` is ticket 11's and I only fetched it.
- Applied: Close sits in a File menu, not Window. A menu is shown, not a popover. The icon is black and clear. Motion can be interrupted, as a press during a morph still lands. Colour isn't alone: an error also has text, and a meeting a shape of its own.
- Spec wins: panels should hide when the app is inactive and have a title bar, and this one is the signature element. HUDs are for media apps. The spec's reasons stand.
- Ideas, not done: let people remove the menu bar icon, as the HIG asks. Raise the hairline for Increase Contrast, which `.red` and `.green` follow and our 20% white doesn't. "Quit wispr-free" against the name Wispr Free. On a side edge a toast and a card overlap, as they do today. Moving the picker's objc2 features off is still ticket 09's idea.
