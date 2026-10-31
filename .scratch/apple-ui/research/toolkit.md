# Toolkit and bridge: SwiftUI/AppKit over the Rust core, built from the CLI

Ticket 03. Checked 2026-10-31 on macOS 27.2, Xcode 27.2 (27B52), Swift 6.4.2, rustc 1.99.1.

How to read the marks:
- **[M]** measured here with a throwaway probe in `/tmp/wf-toolkit-probe` (sources and PNGs are still there). No link is possible.
- **[SDK]** read from the installed `MacOSX27.2.sdk`: `SwiftUI.swiftinterface`, `SwiftUICore.swiftinterface`, `AppKit.framework/Headers`.
- **[?]** not verified. Treat it as open.

## Short answer

SwiftUI views hosted in AppKit windows, a Swift executable, one Rust staticlib with no engines in it, UniFFI as the bridge, SwiftPM driven by `cargo xtask bundle`, and a separate Rust helper for engine hosts. Every window is buildable natively. Two problems remain: glass on the pill always looks frosted (§2), and memory after the first window opens is high (§7).

## 1. Liquid Glass and materials

All availability below is macOS, from [SDK].

| API | From | Notes |
|---|---|---|
| [`glassEffect(_:in:)`](<https://developer.apple.com/documentation/swiftui/view/glasseffect(_:in:)>) | 26.0 | Defaults to `.regular` in a capsule. Apply it [after other appearance modifiers](https://developer.apple.com/documentation/swiftui/applying-liquid-glass-to-custom-views) |
| [`Glass`](https://developer.apple.com/documentation/swiftui/glass) | 26.0 | `.regular`, `.clear`, `.identity`, `.tint(_:)`, `.interactive(_:)` |
| [`GlassEffectContainer`](https://developer.apple.com/documentation/swiftui/glasseffectcontainer) | 26.0 | Best performance with several glass views; lets them [blend and morph](https://developer.apple.com/documentation/swiftui/applying-liquid-glass-to-custom-views) |
| [`glassEffectID`](<https://developer.apple.com/documentation/swiftui/view/glasseffectid(_:in:)>), [`glassEffectUnion`](<https://developer.apple.com/documentation/swiftui/view/glasseffectunion(id:namespace:)>), [`GlassEffectTransition`](https://developer.apple.com/documentation/swiftui/glasseffecttransition) | 26.0 | Morphing between pill states |
| [`.buttonStyle(.glass)`](https://developer.apple.com/documentation/swiftui/primitivebuttonstyle/glass), [`.glassProminent`](https://developer.apple.com/documentation/swiftui/primitivebuttonstyle/glassprominent) | 26.0 | Apple prefers these to [custom glass on buttons](https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass) |
| [`backgroundExtensionEffect()`](<https://developer.apple.com/documentation/swiftui/view/backgroundextensioneffect()>) | 26.0 | Mirrors and blurs content under a sidebar or inspector; "apply with discretion" |
| [`scrollEdgeEffectStyle(_:for:)`](<https://developer.apple.com/documentation/swiftui/view/scrolledgeeffectstyle(_:for:)>), `scrollEdgeEffectHidden` | 26.0 | System bars [get it by default](https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass) |
| [`ToolbarSpacer`](https://developer.apple.com/documentation/swiftui/toolbarspacer), `sharedBackgroundVisibility(_:)` | 26.0 | Toolbar grouping |
| [`ConcentricRectangle`](https://developer.apple.com/documentation/swiftui/concentricrectangle), `containerCornerOffset`, `buttonSizing` | 26.0 | Concentric shapes and sizing |
| [`NSGlassEffectView`](https://developer.apple.com/documentation/appkit/nsglasseffectview) | 26.0 | `contentView`, `cornerRadius`, `tintColor`, `style` (`.regular`, `.clear`). Only `contentView` is guaranteed to sit inside the glass |
| `NSGlassEffectView.effectIsInteractive` | **27.0** | Needs `if #available` on a 26 floor [SDK] |
| [`NSGlassEffectContainerView`](https://developer.apple.com/documentation/appkit/nsglasseffectcontainerview) | 26.0 | `contentView`, `spacing` |
| [`NSBackgroundExtensionView`](https://developer.apple.com/documentation/appkit/nsbackgroundextensionview) | 26.0 | AppKit twin of the extension effect |
| [`NSHostingSceneRepresentation`](https://developer.apple.com/documentation/swiftui/nshostingscenerepresentation), [`NSApplication.addSceneRepresentation`](<https://developer.apple.com/documentation/appkit/nsapplication/addscenerepresentation(_:)>) | 26.0 | SwiftUI scenes in an AppKit-lifecycle app |
| [`NSVisualEffectView`](https://developer.apple.com/documentation/appkit/nsvisualeffectview), [`Material`](https://developer.apple.com/documentation/swiftui/material) | old | Still there; [`state`](https://developer.apple.com/documentation/appkit/nsvisualeffectview/state-swift.property) can be forced active, which glass can't (§2) |

`glassBackgroundEffect` isn't in the macOS interface [SDK]. The SwiftUI additions marked macOS 27 are document APIs, not glass [SDK].

What comes free, per [Adopting Liquid Glass](https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass):
- Standard bars, sheets, popovers, menus and controls adopt glass when built with the latest SDK.
- Sidebars and toolbars float in the glass layer, and windows get rounder corners.
- Custom backgrounds in split views and toolbars can interfere, so Apple says to remove them.
- Settings: [the scene](https://developer.apple.com/documentation/swiftui/settings) gives you the app-menu item, its shortcut, and tabs through `TabView`. That page says nothing about glass, so how Settings looks is **[?]**.
- [M] A `NavigationSplitView` with `.toolbar` inside a plain `NSHostingView` in an `NSWindow`, with no SwiftUI `App` scene, exposes a real `AXToolbar` holding "Hide Sidebar" and the custom button. Toolbars bridge through [`sceneBridgingOptions`](https://developer.apple.com/documentation/swiftui/nshostingview/scenebridgingoptions), which defaults to `.all` for a window's content view.

## 2. The pill

The window recipe ports almost unchanged from [`overlay/macos.rs`](../../../crates/platform/src/overlay/macos.rs): level 1000, `.canJoinAllSpaces`, `.stationary`, `.fullScreenAuxiliary`, `.ignoresCycle`, `hidesOnDeactivate = false`, `canBecomeKey` and `canBecomeMain` false. Apple documents [`.fullScreenAuxiliary`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/fullscreenauxiliary) and [`.canJoinAllSpaces`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/canjoinallspaces).

Native code gets simpler:
- Pass [`.nonactivatingPanel`](https://developer.apple.com/documentation/appkit/nswindow/stylemask-swift.struct/nonactivatingpanel) to `NSPanel.init`. The private `_setPreventsActivation:` and the `set_class` swap exist only because winit made an `NSWindow` first ([macos.rs:71](../../../crates/platform/src/overlay/macos.rs)). A [Wine patch](https://list.winehq.org/archives/list/wine-devel@list.winehq.org/message/AOXXVIZRS4Q23UEH64MCK5W7NDTIDVBS) needs the same call to change the mask later, and a [JUCE report](https://forum.juce.com/t/making-a-floating-window-that-doesnt-bring-the-application-to-the-front/36963) saw a late change do nothing. Apple doesn't document the timing.
- [M] A panel made that way with `NSHostingView`, ordered in with `orderFrontRegardless`, stayed non-key. The app stayed inactive and the frontmost app didn't change.
- **[?]** A click on such a panel wasn't tested, because that needs a synthetic click. Run it by hand in the prototype.

Glass on a transparent, never-key panel [M] (`/tmp/wf-toolkit-probe/glass/a.png`, `b.png`; Mac in Dark mode):
- It renders with `isOpaque = false` and `.clear` background, for both `glassEffect` and `NSGlassEffectView`.
- It tracks what's behind it: repainting the window underneath changed the glass in the next capture. A [report of stale glass on 26.2](https://developer.apple.com/forums/thread/810314), still unanswered, didn't reproduce on 27.2. Moving windows underneath wasn't tested **[?]**.
- **It always looks inactive and frosted, `.clear` included.** That matches a [forum report](https://developer.apple.com/forums/thread/818901) (DTS asked for a sample; no answer) and [a third-party issue](https://github.com/egoist/mygo/issues/598).
- The private `_setForceMainAppearance:`, used by [that project's fix](https://github.com/egoist/mygo/pull/599), gives the crisp look with a bright rim. The panel responded to it on 27.2 and the capture shows the difference.
- I found no public API for this. `NSGlassEffectView` has no equivalent of `NSVisualEffectView.state`. Making the panel key would take the keyboard from the app being typed into.
- System controls in a never-key window also draw inactive: a `Toggle` lost its accent colour [M].

Other pitfalls:
- [`ignoresMouseEvents`](https://developer.apple.com/documentation/appkit/nswindow/ignoresmouseevents) is one flag for the whole window, so click-through when idle means toggling it.
- As a window's content view, `NSHostingView` [drives `contentMinSize` and `contentMaxSize`](https://developer.apple.com/documentation/swiftui/nshostingview/sizingoptions) and manages title and toolbar. Set `sizingOptions = []` and `sceneBridgingOptions = []` for a frame the Rust dock logic owns.
- SwiftUI buttons in inactive windows [have needed two clicks](https://christiantietze.de/posts/2024/04/enable-swiftui-button-click-through-inactive-windows), with [`acceptsFirstMouse`](<https://developer.apple.com/documentation/appkit/nsview/acceptsfirstmouse(for:)>) on a subclass as the fix. That source says macOS 15 changed this, so it's **[?]** on 27.
- Apple notes [Reduce Transparency can modify glass](https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass), so custom glass needs testing with [that setting](https://developer.apple.com/documentation/swiftui/environmentvalues/accessibilityreducetransparency) **[?]**.

## 3. Menu bar icon

[`LSUIElement`](https://developer.apple.com/documentation/bundleresources/information-property-list/lsuielement) is already set ([bundle.rs:80](../../../crates/xtask/src/bundle.rs)), so neither option needs Dock work.

| | [`MenuBarExtra`](https://developer.apple.com/documentation/swiftui/menubarextra) (13.0) | [`NSStatusItem`](https://developer.apple.com/documentation/appkit/nsstatusitem) |
|---|---|---|
| Effort | Scene in a SwiftUI `App`; `.menu` or [`.window` style](<https://developer.apple.com/documentation/swiftui/scene/menubarextrastyle(_:)>) | `NSMenu` built by hand |
| Control | [No first-party way](https://github.com/orchetect/MenuBarExtraAccess) to read or set presentation or reach the `NSStatusItem`, "still as of Xcode 26" | Full: `button`, `menu`, `behavior`, template images |
| Removal | App [terminates](https://developer.apple.com/documentation/swiftui/menubarextra) if a menu-bar-only extra is removed | App's choice through `behavior` [SDK] |
| New in 27 | Nothing found | `expandedInterfaceDelegate`, so popover-style items join keyboard navigation [SDK]; menus are handled automatically |
| Idle cost | 18.9 MB with a Settings scene [M] | 13.8 MB alone [M], but the pill loads SwiftUI anyway |

Opening windows is the hard part, and it doesn't favour SwiftUI's scene model:
- [`openWindow`](https://developer.apple.com/documentation/swiftui/openwindowaction) and [`openSettings`](https://developer.apple.com/documentation/swiftui/environmentvalues/opensettings) come from a view's environment. Here the triggers also come from Rust and from the status menu.
- [steipete](https://steipete.me/posts/2025/showing-settings-from-macos-menu-bar-items/) reports that `SettingsLink` in `MenuBarExtra` is unreliable and that `openSettings` needs a live SwiftUI tree. His recipe is a hidden window, a switch to `.regular`, and timed delays. That's one developer's report from 2025.
- `NSHostingSceneRepresentation.environment.openSettings()` is [Apple's documented route](https://developer.apple.com/documentation/swiftui/nshostingscenerepresentation) from AppKit. Two unanswered forum threads report that [`Window` and `UtilityWindow` don't open](https://developer.apple.com/forums/thread/802435) that way, while [`Settings` does](https://developer.apple.com/forums/thread/793180) **[?]**.

**Recommendation:** `NSStatusItem` with `NSMenu`, and AppKit-owned `NSWindow`s with `NSHostingController`, activating the app explicitly before showing one. It needs no scene workarounds, and a template image finally gives the monochrome glyph [`tray.slint`](../../../crates/app/ui/tray.slint) says Slint can't. I didn't run `NSApp.activate()` from an accessory app, because it would have taken focus **[?]**.

## 4. Bridge

| | UniFFI | swift-bridge | C ABI + cbindgen |
|---|---|---|---|
| Latest | [0.32.2](https://crates.io/crates/uniffi), 2026-09-23 | [0.1.59](https://crates.io/crates/swift-bridge), 2026-01-06 | [0.29.4](https://crates.io/crates/cbindgen), 2026-06-10 |
| Activity | [Pushed 2026-10-28](https://github.com/mozilla/uniffi-rs); [Firefox for iOS uses it](https://github.com/mozilla/uniffi-rs/issues/2917) | Last commit 2026-09-05, [one maintainer's repo](https://github.com/chinedufn/swift-bridge) | Mozilla, [headers only](https://github.com/mozilla/cbindgen) |
| Records, enums | [Structs and Swift enums](https://mozilla.github.io/uniffi-rs/latest/swift/overview.html); `Vec<Record>` worked [M] | [Transparent types](https://github.com/chinedufn/swift-bridge), but [`Vec<TransparentStruct>` is open](https://github.com/chinedufn/swift-bridge/issues/305); strings are `RustString` | By hand on both sides |
| Rust thread to Swift | [Foreign traits](https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/foreign_traits.md) with `Send + Sync`; worked [M] | `extern "Swift"`, [boxed `Fn`](https://github.com/chinedufn/swift-bridge/blob/master/book/src/built-in/boxed-functions/README.md) | `@convention(c)` plus [`Unmanaged`](https://developer.apple.com/documentation/swift/unmanaged) context |
| Swift `async` to Rust | [Native; the foreign side is the executor](https://mozilla.github.io/uniffi-rs/latest/futures.html); no Rust runtime needed [M] | [`async` feature pulls in tokio](https://github.com/chinedufn/swift-bridge/blob/master/Cargo.toml) | By hand |
| Swift 6 | No warnings in strict mode [M]; [manual says "partial"](https://mozilla.github.io/uniffi-rs/latest/swift/overview.html), [#2458](https://github.com/mozilla/uniffi-rs/issues/2458) open | [Async fixed in 0.1.59](https://github.com/chinedufn/swift-bridge/commits/master); [#350 `deinit` isolation error](https://github.com/chinedufn/swift-bridge/issues/350) open | Yours to get right; opaque pointers mean `@unchecked Sendable` |
| Library kind | Static or dynamic; [bindgen reads a `.a`](https://mozilla.github.io/uniffi-rs/latest/swift/uniffi-bindgen-swift.html) | [Static via `swiftc`](https://github.com/chinedufn/swift-bridge/blob/master/book/src/building/swiftc-and-cargo/README.md) | Either |
| Built here | Yes | No, docs and issues only | No |

**Recommendation: UniFFI, pinned exactly.** It's the only one here that met the whole brief in a real build [M]. The probe had a Swift 6 target and a default-MainActor app target. A `std::thread` called a Swift `final class` that yields into `AsyncStream`, and Swift made sync, throwing and `async` calls. It ran clean with no warnings on a clean build.

Rules that came out of it:
- Keep the generated Swift in its own target **without** `defaultIsolation(MainActor)`. With it, the build fails [M], which is [#2818](https://github.com/mozilla/uniffi-rs/issues/2818).
- Name the C module `<namespace>FFI`, or `canImport` silently drops the import and everything goes "not in scope" [M]. `uniffi-bindgen-swift` must run inside the Cargo workspace, because it calls `cargo metadata` [M].
- [#2917](https://github.com/mozilla/uniffi-rs/issues/2917) (`use "_Builtin_stdbool"` breaking on Xcode 27) didn't reproduce here, with a C target or an xcframework [M].
- A foreign trait call blocks the Rust thread. The sink should only yield into the stream, and must never wait on the main actor. The sink class compiled and ran off the main thread under default-MainActor isolation with no annotation [M].
- [Panics in non-throwing functions are fatal in Swift](https://mozilla.github.io/uniffi-rs/latest/swift/overview.html), so export `Result` wherever Rust can panic.
- Avoid async foreign traits, which [hit a strict-concurrency error in 0.31.2](https://github.com/mozilla/uniffi-rs/issues/2929). They aren't needed for an event stream.
- Pre-1.0 means [breaking releases](https://github.com/mozilla/uniffi-rs/blob/main/CHANGELOG.md) ([1.0 FFI still open](https://github.com/mozilla/uniffi-rs/issues/2155)). RustBuffer is [reported slow on large reads](https://github.com/mozilla/uniffi-rs/issues/3013), so keep audio samples out of events.

## 5. Build

Three ways of linking a Rust staticlib all worked [M]:
1. **`binaryTarget` with an xcframework** from `xcodebuild -create-xcframework -library lib.a -headers dir`. No absolute paths, no `unsafeFlags`. Binary targets are [SE-0272](https://github.com/swiftlang/swift-evolution/blob/main/proposals/0272-swiftpm-binary-dependencies.md). Best fit.
2. SwiftPM C target holding the header and modulemap, with `unsafeFlags(["-L…"])` and `linkedLibrary`. Simple, but the path has to be absolute.
3. Plain `swiftc … -Xcc -fmodule-map-file=… -L … -lname`, as in [UniFFI's manual](https://mozilla.github.io/uniffi-rs/latest/swift/module.html). No test runner or asset catalogs.

[SE-0482](https://github.com/swiftlang/swift-evolution/blob/main/proposals/0482-swiftpm-static-library-binary-target-non-apple-platforms.md) is for non-Apple platforms, so it isn't relevant.

SwiftPM facts on this toolchain [M]:
- `swiftbuild` is the default backend, `native` is deprecated, and products land in `.build/out/Products/Release`.
- `.process("…xcassets")` **does** produce `Assets.car`. [Older reports](https://forums.swift.org/t/does-spm-support-colors-in-asset-catalogs/54941) saying otherwise are out of date.
- `Bundle.module` now looks in `Bundle.main.resourceURL` first, so a resource bundle belongs in `Contents/Resources`. At the app root, `codesign` fails with "unsealed contents".
- Framework dependencies of the staticlib must be listed for the Swift link; [`--print native-static-libs`](https://doc.rust-lang.org/rustc/command-line-arguments/print-options.html) prints them. The probe needed none, so the real list is unknown **[?]**.

How `cargo xtask bundle` changes (facts only; [bundle.rs](../../../crates/xtask/src/bundle.rs) is the file):
- Add before assembly: build the staticlib and helper, generate bindings, `swift build -c release`, `actool`.
- The smoke test `binary --version` ([:344](../../../crates/xtask/src/bundle.rs)) must point at the helper, or the Swift entry point must handle the flag before starting AppKit.
- `MINIMUM_MACOS` goes to 26.0 and `Info.plist` gains `CFBundleIconName`.

Engine hosts ship as a separate Rust helper:
- The Swift binary shouldn't take `--engine-host`. That would link every engine into the UI process and load AppKit in each host, against ADR-0005's "never touches the window system".
- The change is small on the Rust side: [`ChildEngines::new`](../../../crates/dictation/src/remote.rs) already takes a path, and only [`app/src/dictation.rs:51`](../../../crates/app/src/dictation.rs) passes `current_exe()`.
- **Blocker:** [`wf-dictation`](../../../crates/dictation/Cargo.toml) turns on all engine features unconditionally, so a staticlib over it pulls in whisper, sherpa and the `libllama` dylib dependency. The flow and the engines have to be separated before linking.
- Location: Apple lists [`Contents/MacOS/` and `Contents/Helpers/`](https://developer.apple.com/documentation/bundleresources/placing-content-in-a-bundle) for helper tools. Use `Contents/Helpers/`.
- Dylibs belong in `Contents/Frameworks/` on that same page. Today they sit in `Contents/MacOS` [M], and the rpath in [`.cargo/config.toml`](../../../.cargo/config.toml) follows that. Apple warns misplaced code can fail at notarization.
- Signing: the helper is [its own code item](https://developer.apple.com/forums/thread/701514). Sign inside out with `-i com.wispr-free.<tool>`, never `--deep`. ADR-0005's "nothing extra to sign" stops being true.
- TCC: Quinn says the system [follows the chain from child to parent](https://developer.apple.com/forums/thread/805245), and his test held for Input Monitoring in a child. Elsewhere he calls it [heuristics that break when a child daemonises](https://developer.apple.com/forums/thread/678819). Hosts get samples over stdin, so they need no microphone or keyboard grant.
- **[?]** A helper reading a protected "chosen folder" attributed to the app wasn't tested.
- The app's designated requirement today is `identifier "com.wispr-free.app" and certificate leaf = H"ee05…"` [M]. A Swift executable with the same identifier and certificate has the same requirement, so grants should survive the swap, by the logic in [bundle.rs:5](../../../crates/xtask/src/bundle.rs). That's inferred, not run **[?]**.

Icons without an Xcode project [M]:
- `actool AppIcon.icon --compile out --platform macosx --minimum-deployment-target 26.0 --app-icon AppIcon --target-device mac --output-partial-info-plist p.plist` produced `Assets.car` with light and dark variants, plus `AppIcon.icns`. The plist gained [`CFBundleIconName`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleiconname) and `CFBundleIconFile`.
- The `.icon` was a hand-written `icon.json` around the existing SVG. Apple documents [only the Icon Composer app](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer), so that format is undocumented and may change. [mpv does the same](https://git.quad4.io/Mirrors/mpv/commit/f93df58a8eebf23b7f841bbf34bb2bda9f9cc16b).

xcodegen ([2.46.0](https://github.com/yonaskolb/XcodeGen), installed here) and Tuist ([canaries daily](https://github.com/tuist/tuist), not installed) aren't worth adding. SwiftPM, `actool` and `codesign` already cover the app, tests, assets, icon and signing [M]. They'd only earn their place for XCUITest, which §6 rules out anyway.

## 6. Testing from the CLI, without taking focus

| Tool | Result here [M] | Use it for |
|---|---|---|
| `swift test` (Swift Testing, `@MainActor`) | Ran AppKit and SwiftUI; frontmost app unchanged | The harness |
| [`ImageRenderer`](https://developer.apple.com/documentation/swiftui/imagerenderer) | `Text` fine; `Form` **blank**, glass pill **empty**, `NavigationSplitView` a **no-entry placeholder**. Matches Apple's warning | Nothing here |
| `NSHostingView` + [`cacheDisplay`](<https://developer.apple.com/documentation/appkit/nsview/cachedisplay(in:to:)>), never ordered in | Real layout and controls. **No glass capsule.** Some text drew black on dark; pinning the appearance should fix it **[?]** | Layout and text regressions |
| [swift-snapshot-testing](https://github.com/pointfreeco/swift-snapshot-testing) 1.19.7 | [SwiftUI strategy is iOS/tvOS only](https://github.com/pointfreeco/swift-snapshot-testing/blob/main/Sources/SnapshotTesting/Snapshotting/SwiftUIView.swift); [`NSView` strategy](https://github.com/pointfreeco/swift-snapshot-testing/blob/main/Sources/SnapshotTesting/Snapshotting/NSView.swift) on an `NSHostingView` matched pixel-exact on 2 reruns | Same limits as the row above, plus diffing. References are per-OS by its own note |
| [ViewInspector](https://github.com/nalexn/ViewInspector) 0.10.5 | In Swift 6 on macOS: found text, read a `Toggle`'s label and identifier, tapped a `Button` | Logic in views. Reads structs, not pixels; check [its coverage](https://github.com/nalexn/ViewInspector/blob/master/readiness.md) |
| `AXUIElement` from a second process | Walked 37 nodes of an unfocused window: `AXIdentifier`, values, toolbar, split group. Needs `AXManualAccessibility` set and an Accessibility grant, which this shell has | Best functional check of the real app |
| In-process `accessibilityChildren` | Root node only | Nothing |
| `screencapture -x -R` over your own opaque backdrop window | The only thing that showed real glass. Needs Screen Recording (preflight true here, no prompt) | Pill look, by eye |
| [XCUITest](https://developer.apple.com/documentation/xcuiautomation/xcuiapplication) | Not run. Built on [launching and activating the app](<https://developer.apple.com/documentation/xcuiautomation/xcuiapplication/activate()>), and SwiftPM has no UI-test target kind | Ruled out |

Pixels from `cacheDisplay` can't verify glass, so glass stays a capture-and-look check.

## 7. Memory

Physical footprint from `vmmap -summary` [M]. The probes have **no Rust core**. The panel sat off-screen.

| Process | Footprint |
|---|---|
| Current Slint app, running 1 h | 41.3 MB (peak 222.6) |
| AppKit `NSStatusItem` only | 13.8 MB |
| SwiftUI `MenuBarExtra` + `Settings` scene, no window | 18.9 MB |
| Plus glass pill in an `NSPanel` | 23.2 MB |
| Plus `NavigationSplitView`/`Form` window open | 37.6 MB |
| Same, 17 s after closing | **36.0 MB** |
| Repeat with `malloc_zone_pressure_relief` after close | **37.3 MB** |

- A SwiftUI menu-bar app with the pill idles around 23 MB before Rust is added. That's a good floor.
- The first window costs about 14 MB, and closing it gives almost none back. [`mi_collect`](../../../crates/app/src/memory.rs) only reaches Rust's heap.
- So [ADR-0005](../../../docs/adr/0005-engines-in-child-processes.md)'s "under 40 MB" idle goal is likely met until the first Settings visit and likely missed after it, once Rust's heap is added.
- Rust's own idle share isn't known, because the Slint renderer can't be separated out of the 41 MB **[?]**. A real Settings window will also differ from this probe's.
- I found no published MenuBarExtra footprint figures.

## What this overturns

- [ADR-0002](../../../docs/adr/0002-slint-for-ui.md) goes, along with the need for objc2 window code, the private activation call and the Slint attribution note.
- ADR-0005 needs edits: "the app's own binary", and "nothing extra to sign".
- The Slint surface to replace: 36 `slint::` references in 15 files under `crates/app/src`, 16 of them timers or event-loop hops, and 3,932 lines of `.slint`.
- `MainThreadMarker` survives in [`desktop.rs`](../../../crates/platform/src/desktop.rs). Under a Swift `main`, Rust's calls there must be on AppKit's main thread.

## Not verified

A real click on the Swift-built panel; glass while a window moves underneath it; the Reduce Transparency look; first-click behaviour on 27; `NSApp.activate()` and `openSettings` from an accessory app (both would take focus); the Settings scene's look; grants surviving the swap to a Swift executable; the helper's TCC attribution; the real framework link list; footprint with the Rust core linked in.
