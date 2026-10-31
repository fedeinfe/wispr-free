# ADR-0006: SwiftUI and AppKit over the Rust core

Status: accepted · 2026-10-31. Supersedes ADR-0002.

## Context

The user wants the interface to look like an Apple app. Slint draws its own controls, so it can imitate that look but never get Liquid Glass, system menus, sidebars or accessibility from the OS. Windows and Linux are parked. On a Mac-only product, the reason for a shared UI toolkit has gone.

## Decision

The macOS app becomes a Swift executable with SwiftUI views in AppKit-owned windows, on macOS 26 or later. All logic stays in Rust, in a `wf-shell` static library reached through UniFFI, pinned to an exact version. Engines move to a separate Rust helper, `wf-engine-host`, so that no engine links into the UI process. `cargo xtask bundle` drives the build through SwiftPM, with no Xcode project.

The pill is a solid dark capsule, not Liquid Glass. Glass on a never-key panel always draws as inactive, its contrast fails over white pages, and the only fix is a private call.

## Consequences

- The UI no longer runs on Windows or Linux. Bringing them back means writing a new UI over `wf-shell`. Slint stays in git history.
- Two toolchains, and contributors need Xcode.
- The Slint attribution goes, along with the private activation call and most of the `objc2` window code.
- ADR-0001: the "warm settings window" gives way to system colours. Our own palette, icon and sounds stay.
- ADR-0005: an engine host is no longer the app's own binary, and it needs its own signature.
- Memory after the first window goes from a promise of under 40 MB to a goal of under 65 MB after every window has been used, because SwiftUI keeps its state. Measured at the cutover: 62 to 64 MB, flat across rounds, so not a leak. The separate-process fallback isn't being built. Idle before any window stays under 25 MB, at about 20 MB.
- Only the real bundle, `com.wispr-free.app` started plainly, is the real app. Everything else, tests and tools included, is a dev run on its own folder with no keys or microphone unless a person asks. `wf_shell::launch` decides this from what the OS says.
