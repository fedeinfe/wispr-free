# 11 Window shell and test rig
Status: ready-for-agent
Blocked by: 10

Spec: Architecture (AppKit-owned windows), Testing, Settings. This is the walking skeleton tickets 12, 13 and 14 build on at once, so it has to leave them apart in separate files.

- One window controller shape: `NSWindow` plus `NSHostingController`, created on open, released on close, `NSApp.activate()` before showing. Open and raise rules between windows as today, with the rule itself in Rust.
- Settings as a skeleton: sidebar with the 8 pages, last page restored, title follows the page, no content. Skeletons for Onboarding and Meetings, reachable from the menus.
- `AccentColor`, `AppIcon.icon` and the asset catalogue.
- `edit { $0.x = y }` over compare and swap, in one place all pages use.
- ViewInspector 0.10.5 with a fake core.
- `cargo xtask ax-check`, a second process using `AXUIElement` against the real dev bundle, with a way to open a window without activating it.
- `cargo xtask shots` extended to windows in light and dark.

Done when: `swift test` and `cargo test` pass, `ax-check` walks all three skeleton windows of the real dev bundle with today's labels as `AXIdentifier`s, no run takes the focus, and closing a window releases its controller.

## Comments

### 2026-10-31, implementation

Done: `crates/shell/src/app_windows.rs` holds the rule (`OpenWindows::ask`), and `Shell` answers a menu with `ShellEvent::Open` or `Raise`, so Swift never decides. `settings/page.rs` has the 8 pages and `settings_pages()`, `Prefs` keeps the page, and `Shell` gained `settings_page`, `set_settings_page`, `window_closed` and the free `error_text`. `MenuAction::OpenOnboarding` is "Finish setup…". In `apple/`: `Windows/` (`WindowController`, `WindowStage`, `Windows`), `Settings/` (`SettingsCore`, `SettingsEditor`, `SettingsNavigation`, `SettingsView`), `Onboarding/OnboardingView`, `Meetings/MeetingsView`, `App/WindowShots`, `Sources/AxWalk`, `Assets.xcassets` and the real `AppIcon.icon`. `cargo xtask ax-check` is in `crates/xtask/src/ax_check.rs`, and `cargo xtask shots` writes `windows/{settings,meetings,onboarding}-{light,dark}.png` beside the pill's folders. The app gained `--behind` and `--window-shots=`, and `--pick=` repeats.

Verified on b63fbd9, with `apple-ui` at e0cc807 already in it (the merge had nothing to bring in). The one commit after it only adds a Swift test. Logs and shots are in `wispr-free-notes/apple-ui/11-window-shell-rig/`:
- `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked --no-fail-fast`: 1322 passed, 0 failed, 81 ignored. The ignored count didn't change.
- `swift test`: 63 of 63 on the final commit, 15 runs in a row with no failure and no warnings.
- **`cargo xtask ax-check` on the real dev bundle**, signed with Wispr Free Local, `codesign --verify --strict` passing: Settings shows its 8 labels among 39 nodes, Meetings 4 among 22, Set up Wispr Free 2 among 10. Each opens through the status menu's own item. For Settings it also seeds `settings_page = "privacy"` and sees the title "Privacy & data", selects General from outside and sees the title follow, and reads the page back from `pill.toml`. Each window is then closed by pressing its real close button, and the app logs `released <window>` from the controller's `deinit`. A last run picks Finish setup…, Settings… and Meetings… in order: onboarding and Meetings open and Settings doesn't.
- **No run took the focus:** every walk fails if the app is in front, `applicationDidBecomeActive` logs a line that fails `ax-check` and `shots`, and neither ever fired.
- **Closing releases the controller:** `swift test` holds weak references to the controller, the `NSWindow` and an object in the view, and all three go. Every real window does the same, and `ax-check` sees it in the release bundle.
- ViewInspector 0.10.5 with `FakeCore`: rows, labels, identifiers, the page on the right, the bottom bar and the tabs. `SettingsEditor`: order, stale reapply, refused write, and against the real `Shell` with Rust's own change racing it.
- `cargo test -p xtask --test dev_bundle -- --ignored`: 2 of 2. `cargo xtask bundle` still makes the Slint app and passes the strict check. Neither was installed, and `~/Applications/Wispr Free.app` wasn't touched.
- Deliberate breaks, one at a time, none touching a path: Rust 10 of 10 failed a test (`rust-breaks.txt`). Swift 24: 22 failed a test and 2 crashed the test process (`swift-breaks.txt`, `swift-breaks-last.txt`). On the real bundle, a controller kept after close and a Meetings tab losing its identifier each failed `ax-check` (`ax-break-*.txt`). That script restored each file from git.
- The real data folder was only read with `stat`: its time and `settings.toml`'s are unchanged, and no `wispr-free-dev` was left behind. No model was used, so the external SSD wasn't needed.
- **Idle `phys_footprint`: 21.5 MB, peak 21.8**, with no window opened. That's under the 25 MB goal and up from 19.8.

Shots, read by me: Settings has the 210 pt sidebar and every label fits, Meetings has its sidebar and the four segments, and Onboarding has its bottom bar, all in light and dark. The pill's sheets are unchanged. All of these are inactive windows, so selection and `.borderedProminent` show grey, not teal.

Bugs this found:
- **The sidebar came out 144 pt wide, with "Style & dict…" cut off.** `.toolbar(removing:)` after `.navigationSplitViewColumnWidth(210)` loses the width, so the toolbar modifier goes first. Only the shots showed it. **Ticket 12: no test measures this, so read the shots.**
- `RunLoop.main.perform` never runs under `swift test`, so a controller kept for one more turn was never let go. It's `DispatchQueue.main.async` now.
- **My first "green" `swift test` was luck.** A real-`Shell` test failed about 1 run in 3, and it showed up as a failure on breaks that had nothing to do with it. `SettingsChanged` can beat the reply to `save`, so a waiting change was laid over a state that already held it, and an `append` showed twice. Rust's news now shows once the running save lands. Rerunning the breaks on the fix then showed one had survived, so I added the test it lacked.

Couldn't verify:
- **`WindowStage.front` has never run.** `NSApp.activate()` and `makeKeyAndOrderFront` would take the focus, so only the order of the two is tested, on a recorder. That's ticket 16's "activate brings Settings forward".
- "Took the focus" has never seen a real activation, for the same reason. The text check is tested, and the AppKit callback hasn't fired.
- **Where the focus goes when the last window closes.** An accessory app stays active with nothing to show. It wasn't tested. Please add it to ticket 16.
- ⌘, and ⌘W by a real key press, and a window dragged, resized or minimised by hand.
- Teal on screen. A one-off probe in the real bundle read `NSColor.controlAccentColor` as `#2dd4bf`, and `assetutil` shows both values in `Assets.car`. This Mac has no accent chosen. On one that has, the user's choice wins, as on any app.
- The icon's dark, clear and tinted looks, which the system derives. I read the 256 px picture from `actool`'s `.icns`. `icon.json` is by hand, and Icon Composer never opened it.
- That `window_closed` returns memory. `release_freed` runs on a thread after `WINDOW_TEARDOWN`, and no test can see it. Footprint after using a window wasn't measured: that's tickets 12 and 14.
- Real dictation, keys and microphone. The Slint app wasn't launched, and `cargo xtask eval` wasn't repeated, since no code it runs changed.

Decisions the spec didn't cover:
- **Left for ticket 13:** "the Models step closes Settings", "with nothing to do it doesn't open" and opening at launch all need `steps` in `Shell`. `ask` returns one order, so the first means letting it return a close as well. "Finish setup…" uses `!onboarding.finished` alone, and a broken permission has to join it there. Back and Continue do nothing, and "Step n of 4" is absent because its logic is Rust's.
- The rule exists twice until ticket 15: `OpenWindows` for Swift and `WindowHost.redirect_with` for Slint. `WindowHost` is `Rc`, so it can't sit in `Shell`.
- **The page lives in `pill.toml` as `settings_page`**, read and written through `Prefs`. Not `UserDefaults`, as that would sit outside the data root a run is given, and not `Settings`, as it isn't a setting. A file without it opens on General.
- Today's minimums are kept where the spec gives none: Meetings 760×480, Onboarding 680×460, and Onboarding stays resizable.
- `apple/Sources/WisprFree/Windows/` is a folder the spec's tree doesn't list.
- **For tickets 12 to 14:** each window adds its own protocol, as `SettingsCore` does, and `WindowsCore` inherits it, so no two tickets edit one file. One `SettingsEditor` lives as long as the app and reaches views with `.environment`, so use `@Environment(SettingsEditor.self)`. `notice` is there for ticket 12's top `Section`. `SettingsPageView` is the one place to switch on the page.
- `ax-walk` is an executable target in the package and isn't bundled. It only reads and presses, and prints JSON, and `ax_check.rs` decides what that means, so the decisions are tested in Rust. It needs Accessibility for the terminal and fails with a message rather than prompting.
- `edit` keeps its changes in order, one save at a time, so typing in a field saves once per keystroke. Ticket 12 can batch them if it shows.
- ViewInspector is a test-only dependency, pinned exactly, with `Package.resolved` committed. It came from SwiftPM's cache here, and a machine without it needs the network once to build, even for `cargo xtask bundle --dev`, as SwiftPM resolves test dependencies too. Nothing of it enters the app.
- The sidebar's `Label`s were the only thing a row needed for its identifier. The title moved from "wispr-free settings" to the page's, as the spec says.
- `mark.svg` is gone. `AppIcon.icon` has today's art as two square, unmasked vector layers on a gradient `fill`, with the art's own rounded ground removed.

Design review: only `app-icons` was this ticket's, read live from Apple's JSON. Applied: square unmasked layers the system masks, vector art, a background from the format's own gradient rather than an image, clearly defined edges, centred content. Settings' review is ticket 12's. Ideas, not done: give the light window shots a grey backdrop, as a white window on white shows no edge. Take a shot with the window key, to see teal, which would mean activating.
