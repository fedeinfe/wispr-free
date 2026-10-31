# Spec: Apple-style UI for Wispr Free

Status: ready-for-agent

Replace the Slint interface with SwiftUI and AppKit over the existing Rust core, for macOS 26 and later. The app does what it does today and looks like an app Apple ships.

Every decision here was made by the agent on the user's behalf. The reasons are in `.scratch/apple-ui/issues/` and `research/`, and the code proof is in `prototype/`. ADR-0006 records the choice that is hard to reverse.

## Goals

1. **Looks native.** Settings, Onboarding and Meetings can pass for a system app, and they get Liquid Glass from the system.
2. **Same product.** Every feature, shortcut, default and file on disk keeps working. Grants survive, so no new permission prompts.
3. **Light.** See [Memory](#memory).
4. **Built from the command line.** `cargo xtask bundle` makes the app, with no Xcode project.

Not goals: Windows or Linux, new features, a translated interface.

## Design language

The model is **Notes and System Settings**, with one signature element: the pill behaves like the Dynamic Island, a solid black capsule that changes shape.

Rules, in order:

1. Use the system control before drawing one. Nothing gets a custom background in a sidebar or toolbar.
2. Glass comes only from the system's own bars, sidebars, menus, popovers and sheets. No `glassEffect` in content.
3. Boldness goes in one place, the pill. Everything around it stays quiet.

| Topic | Rule |
| --- | --- |
| Colour | Semantic system colours only (`.primary`, `.secondary`, `.separator`, `.quaternary`, `.red`, `.green`), which cover light, dark and Increase Contrast. One brand colour: `AccentColor` in the asset catalogue, `#0d9488` light and `#2dd4bf` dark, the same teal as today. It marks selection and primary actions only |
| Type | SF through text styles (`.body` 13 pt, `.callout`, `.caption`, `.title3`, `.largeTitle`), never a fixed size outside the pill. SF Mono only for key combos and `[mm:ss]` |
| Icons | SF Symbols, weights matching adjacent text. The menu bar icon is a template image, so it finally turns monochrome |
| Spacing | `Form(.grouped)` and system defaults. Hand layouts use 8, 12, 16, 20 and 28 |
| Motion | System animations (`.snappy`, `.smooth`). The pill morphs with `.snappy`. With Reduce Motion it crossfades. No hand-tuned spring values |
| Corners | System defaults, continuous. `ConcentricRectangle` for anything nested |
| Appearance | Follows the system. The Theme control goes (see [Settled mismatches](#settled-mismatches)) |

ADR-0001's "warm settings window" gives way to system colours. Its "small dark capsule pill" and "our own palette, iconography and sounds" stay: the teal, the app icon and the sounds are ours, and no Wispr asset or string enters.

### Pill and card

Sizes, states, precedence, drag, snap, edge margins, bar scale and timings stay exactly as in `wf_core::pill`.

- **Skin:** solid `#17161c` capsule with a 0.5 pt white hairline at 20%, always dark, in both appearances. White bars.
- **Why not glass:** on a never-key panel the system always draws glass as inactive and ignores most of the tint. White on it measured 2.3:1 to 2.7:1 over a white page, and the 82% black tint came out no darker than the 45% one. The only way to the crisp look is a private call. Solid measured 16.7:1 on every backdrop and needs nothing private, nothing sampled from behind it and no Reduce Transparency fallback. Captures: `prototype/pills-*.png`.
- **Error** is a red fill. **Meeting** keeps green bars at 0.85.
- **Card:** same skin, 14 pt continuous radius, 236 wide, `.body` text.
- **Buttons on the pill and card** use our own `PillButtonStyle`, primary white on dark and secondary white at 16%. System buttons draw inactive in a never-key panel, so `.borderedProminent` loses its colour there, as the captures show.
- **Shadow** is drawn in SwiftUI inside the panel, with `hasShadow = false`.
- **Menu:** a real `NSMenu`, same items and order.

### Menu bar icon

`NSStatusItem` with a template image and an `NSMenu`: same items as today. A hidden main menu supplies ⌘, ⌘W and the Edit commands, because an accessory app shows no menu bar but text fields still need ⌘C and ⌘V.

### Settings

`NavigationSplitView` with a 210 pt sidebar and `Form(.grouped)` on the right, 860×640, minimum 760×480. Same 8 pages, every change commits at once.

- The sidebar is a deliberate step away from the HIG's toolbar of panes: 8 panes don't fit a stable toolbar, and System Settings itself uses a sidebar.
- From the HIG: the title follows the page, the last page is restored, minimise and zoom are off, ⌘, opens it.
- Shortcuts are `LabeledContent` rows: combo in SF Mono, a small Change… button, Reset in a context menu.
- Destructive actions use `confirmationDialog` with the `.destructive` role.
- The notice banner becomes a top `Section` in the page that raised it.

### Onboarding

Window 680×560 with one step per page: symbol, `.largeTitle`, one sentence, a grouped `Form`, and a bottom bar with Step n of 4, Back and Continue. Logic and copy don't change. The Models step keeps the folder choice above the download buttons.

### Meetings

`NavigationSplitView` 1000×680.

- Sidebar: list and `.searchable(placement: .sidebar)`. The start button moves to the toolbar.
- Toolbar: segmented Summary, Notes, Transcript, Chat in `.principal`, Export and the rest in `.primaryAction`, grouped with `ToolbarSpacer`.
- Content: a 720 pt reading column.
- Stage chips become a small status row under the title.
- Chat uses a bottom input with `.buttonStyle(.glassProminent)` for Send. It's the one floating control over content, so it's the one place glass is allowed outside system bars.

### Design review

The reviewer is the `apple-design` skill, installed from `dickwu/apple-design-skill` at `ff4ebddf`, in `.claude/skills/apple-design` and excluded from git. Its text is Apple's and it has no licence, so it must never be committed. Reinstall steps: `research/design-skills.md`.

Run it once per phase, review mode only, never its "Design improvement mode". Load these, not the whole set:

| Surface | `references/hig/` |
| --- | --- |
| Pill | `panels`, `motion`, `color` |
| Menu bar | `the-menu-bar` |
| Settings | `settings`, `sidebars`, `toolbars`, `materials` |
| Onboarding | `onboarding` |
| Meetings | `sidebars`, `toolbars`, `liquid-glass` |

Read these four live, since Apple changed them after the skill's pull: `designing-for-macos`, `gestures`, `pointing-devices`, `app-icons`.

## Architecture

```text
Wispr Free.app/Contents
├── MacOS/wispr-free            Swift main: AppKit, SwiftUI views, libwf_shell.a linked in
├── Helpers/wf-engine-host      Rust: one engine per process (ADR-0005)
├── Frameworks/libllama*.dylib  used by the helper only
└── Resources/                  Assets.car, AppIcon.icns, SwiftPM resource bundles
```

- **AppKit owns the main thread and the run loop.** Rust keeps its own threads as today (`wf-keys`, flow, workers).
- **Swift owns windows and drawing. Rust owns every decision.** No business rule lives in a view: labels with logic, model states, onboarding steps and pill frames all come from Rust.
- **Windows are AppKit-owned.** `NSWindow` plus `NSHostingController`, one controller per window, created on open and released on close. No SwiftUI `App`, no scenes. `NSApp.activate()` before showing one, as an accessory app must.
- **The pill stays a pure Rust model.** A Swift timer runs at the tier Rust asks for (16, 40 or 250 ms) and calls `pill_tick(...)`, and the `Frame` it gets back drives the panel. So `ignoresMouseEvents` toggles from `Frame.hit_test`, exactly as now.
- **Panel recipe:** `NSPanel` made with `.nonactivatingPanel`, level 1000, `.canJoinAllSpaces`, `.stationary`, `.fullScreenAuxiliary`, `.ignoresCycle`, `hidesOnDeactivate = false`, `sizingOptions = []`, `sceneBridgingOptions = []`. No private API.

### Crates

| Crate | Today | After |
| --- | --- | --- |
| `wf-dictation` | Turns on all engines | New `engines` feature, off by default, gates `load_engine` and `run_engine_host` (two `cfg` lines, proven) |
| `wf-shell` | New | Toolkit-free app logic moved out of `crates/app`, plus the UniFFI exports. `staticlib` and `lib` |
| `wf-engine-host` | New | Binary: `--engine-host`, `--version`. Depends on `wf-dictation/engines`. Also holds the dev-only `wf-probe` binary (`--probe-clip`), which isn't bundled, and the real tests |
| `wf-core` | Pure logic | New `uniffi` feature: derives on `Settings`, `custom_type` for `PathBuf` and `Combo` (proven) |
| `wf-platform` | Slint overlay code | `overlay/` deleted. Rust calls that need AppKit's main thread keep hopping to the main queue |
| `crates/app` | Slint UI | Deleted in the last phase |

Swift lives in `apple/`:

```text
apple/
├── Package.swift
├── Sources/WfCore/        generated, git-ignored, no defaultIsolation
├── Sources/WisprFree/     App, Pill/, MenuBar/, Settings/, Onboarding/, Meetings/
└── Tests/WisprFreeTests/
```

## Bridge

UniFFI pinned to exactly `0.32.2`, with `uniffi-bindgen-swift` run by `xtask`.

The one object is `Shell`, with these groups of methods:

- **Life:** `start(paths, sink)`, `quit()`, which runs `close_media` and `close_meetings`.
- **Dictation:** `on_pill`, `paste_last`, `paste_recent`, `copy_text`, `pill_tick`, `pill_press`, `pill_release`.
- **Settings:** `settings() -> Versioned`, `save(settings, revision) -> Saved | Stale(Versioned)`.
- **Shortcuts:** `record_shortcut`, `stop_recording_shortcut`.
- **Models:** `catalog`, `hardware`, `download`, `cancel_download`, `move_folder`, `verify`.
- **Onboarding:** `steps`, `start_trial`, `arm_trial`.
- **Meetings:** `meetings_status`, `toggle_meeting`, `resume_meeting`, `finish_meeting`, `summarize_meeting`, `cancel_meeting_work`, `ask_meeting`, plus library calls.
- **Data:** `export_all`, `delete_all`, `forget_history`.

Rules:

- `Event`s cross through one foreign trait, `EventSink`, which only yields into an `AsyncStream` and never waits on the main actor.
- Two streams: state events unbounded, level values `.bufferingNewest(1)`, so a flood of levels can't push out a state change.
- Settings use compare and swap. Swift's `edit { $0.x = y }` applies to its latest copy and calls `save`. On `Stale` it reapplies to the fresh value. `learn_word` in Rust therefore can't lose a change or be lost by one. Every save also sends `SettingsChanged`.
- Export `Result` wherever Rust can panic, because a panic in a non-throwing call is fatal in Swift.
- Calls that touch disk or models are `async`. Nothing blocks the main actor.
- No audio samples in events. No async foreign traits.
- Generated Swift sits in a target without `defaultIsolation(MainActor)`.
- Each UniFFI namespace needs its own module in the modulemap (`wfshellFFI`, `wfsettingsFFI`). The generated one lists a single module, and its `canImport` then drops the import in silence. `xtask` writes the modulemap.

Long jobs (downloads, folder moves) move from the windows to `Shell`, so they outlive their window.

## Build

`cargo xtask bundle` gains these steps:

1. `cargo build --release -p wf-shell -p wf-engine-host`.
2. Generate bindings, write the modulemap, make `WfBridge.xcframework`.
3. `swift build -c release --package-path apple`.
4. `actool` on `AppIcon.icon` for `Assets.car` and `AppIcon.icns`.
5. Assemble: binary keeps the name `wispr-free`, dylibs go to `Contents/Frameworks`, resource bundles to `Contents/Resources`.
6. Sign inside out with the existing certificate: dylibs, helper with `-i com.wispr-free.wf-engine-host`, then the app. Never `--deep`.

Also:

- `MINIMUM_MACOS` becomes 26.0, `Info.plist` gains `CFBundleIconName`.
- The rpath in `.cargo/config.toml` points the helper at `../Frameworks`.
- The `--version` smoke test targets the helper.
- The framework list comes from `--print native-static-libs`. The proof needed 20, including CoreData and CloudKit that nothing here uses. Moving the folder picker from `rfd` to `NSOpenPanel` should drop some, so re-check.
- `crates/app/build.rs`'s weak CoreAudio link goes. It only protected macOS 13.

## Testing

Nothing here takes the focus or types into the user's apps.

| Layer | Tool | Covers |
| --- | --- | --- |
| Rust logic | `cargo test -p wf-shell` | Ports of the logic in the 142 window tests, on the existing `Desktop`, `Machine` and `MeetingActions` fakes. Most of the value lives here |
| Bridge | `swift test` with Swift Testing | Round trips for settings, events, errors and compare and swap |
| Views | ViewInspector 0.10.5 with a fake core | Which control shows when, labels, actions |
| Real app | `cargo xtask ax-check`, a second process using `AXUIElement` | Windows, `AXIdentifier`s and values in the real bundle, using today's labels as the contract |
| Persistence | Port of `settings_probe persist-*` | Writes reach disk |
| Real flow | `real_dictation`, `real_meeting`, `engine_host`, moved to `wf-engine-host` | Unchanged meaning |
| Look | `cargo xtask shots`: every pill state and window, in light and dark, over its own backdrop | PNGs for the agent to read, and the design review's input. Not compared by pixel |

`ImageRenderer` and XCUITest are ruled out (`research/toolkit.md` §6).

CI becomes macOS only: fmt, clippy, `cargo test`, `swift build`, `swift test`. It needs `macos-26` or later runners.

## Phases

Work on one branch off `v1`. Until phase 5 the Swift app is a dev bundle with its own id, `com.wispr-free.app.dev`, named Wispr Free Dev. The real app stays Slint and keeps working.

**Phase 0: split, no visible change.**
- Add the `engines` feature and `wf-engine-host`. The Slint app starts hosts from `Contents/Helpers`, dylibs move to `Contents/Frameworks`.
- Move toolkit-free logic to `wf-shell`, and the Slint app becomes its client.
- Done when: all tests pass, eval is unchanged, the installed app dictates.

**Phase 1: stack, pill, menu bar.**
- `apple/`, bridge, `xtask` steps. Pill in every state, card, toast, menu, drag, sounds, status menu.
- Done when: the dev app dictates end to end and the hand checks below for this phase pass.

**Phase 2: Settings.** All 8 pages, shortcut capture, model flows, `NSOpenPanel`. Done when: the ported tests and `ax-check` pass.

**Phase 3: Onboarding.** Done when: the first run works in a clean user account or a reset dev id, and grants are detected live.

**Phase 4: Meetings.** List, detail, four tabs, chat streaming, export. Done when: a real 30-minute fixture displays with no hitch.

**Phase 5: cutover.**
- Bundle under `com.wispr-free.app` and confirm grants survived. Delete `crates/app`, Slint, `overlay/`.
- Flip ADR-0006 to accepted and ADR-0002 to superseded, edit ADR-0001 and ADR-0005, update `GLOSSARY.md`'s engine host entry, `README.md:28,30`, `.scratch/v1/spec.md:5,14,207,245`, CI.
- Keep the last Slint build until the first full day of use.

## Settled mismatches

| Today | Decision |
| --- | --- |
| App language has no reader | Remove the control and the field |
| Theme picker | Remove. The HIG says no app-specific appearance, and the pill is always dark. An old `theme` key in `settings.toml` is ignored |
| System Audio not in onboarding | Stays out, asked at the first meeting, as now |
| Pill's Language menu: 3 entries, overwrites My languages | One list from Rust, all 15 entries. Overwriting stays, since no behaviour changes in this effort |
| Tier as a Debug string | A UniFFI enum |
| Duplicated language and tier lists | One source in Rust |
| Downloads tied to the window | Owned by `Shell`, so closing the window no longer stops them |
| Windows and Linux CI, `overlay/{windows,x11}.rs` | Deleted. The Rust crates stay portable, and Slint remains in git history |

## Memory

Measured on this Mac with the real `Dictation` flow behind UniFFI, a glass pill, a status item and a prototype Settings window, in a release build:

| Moment | Footprint |
| --- | --- |
| Rust flow and pill, idle | 18.1 MB |
| Settings open | 43.9 MB |
| Settings closed, 20 s later | 41.0 MB |
| After 5 more open and close cycles | 42.1 MB |
| Slint app today, after 1 h | 41.3 MB (peak 222.6) |

So idle gets much better until the first window, then matches today, with no leak across reopens. SwiftUI keeps about 23 MB of framework state after a window closes, and nothing in the process can return it.

The old goal "under 40 MB, and memory released on close" can't be promised. New goals, replacing `.scratch/v1/spec.md:14,245`:

- Under 25 MB idle before any window opens.
- Under 55 MB idle after every window has been used and closed.

Measure at the end of phases 1, 2 and 4, with `cargo xtask` reporting `phys_footprint`. The prototype had a single-page Settings, no key listener and the system allocator rather than mimalloc, so real numbers will differ.

If phase 4 ends above 55 MB, the fallback is to run windows in a separate process that exits on close, which is ADR-0005's idea applied to the UI. Don't build that first: it costs a second main, a second bridge client and focus handoff.

## Hand checks

These need a person at the Mac, because an agent can't do them without taking focus. The user runs them once on the dev app, at the end of the phase shown.

| Phase | Check |
| --- | --- |
| 1 | A real mouse click on the pill fires and doesn't steal focus from the app being typed in. In the proof, a click sent through the panel worked and the front app didn't change, but it didn't go through the window server |
| 1 | Right-click menu opens from the never-key panel, and first click isn't swallowed |
| 1 | Pill shows above full-screen apps and on all Spaces |
| 1 | Clicks pass through when `hit_test` is false |
| 2 | `NSApp.activate()` brings Settings forward from the menu bar and pill |
| 2 | ⌘, ⌘W, ⌘C and ⌘V work in Settings' text fields |
| 5 | Grants survive the swap to the Swift binary. The designated requirement should match, but this is inferred. Quit, reinstall, dictate, no prompt |
| 5 | A model in a chosen protected folder loads from the helper. TCC attribution from a helper is untested |

## Risks

| Risk | Plan |
| --- | --- |
| UniFFI is pre-1.0 | Pin exactly. It stays behind `Shell`, so a change touches one crate |
| The `.icon` format is undocumented | Hand-written JSON worked. If `actool` breaks it, fall back to `.icns` |
| A macOS update changes glass or panel behaviour | The pill uses no glass and no private API. System chrome follows the OS by design |
| One toolchain becomes two | `xtask` hides it. Agents need Xcode's `swift` and `actool`, and CI needs a macOS 26 runner |
