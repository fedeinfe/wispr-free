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
