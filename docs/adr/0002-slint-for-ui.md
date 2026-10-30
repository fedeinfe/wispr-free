# ADR-0002: Slint for all UI

Status: accepted · 2026-10-30

## Context

The app needs a tiny always-on-top transparent pill plus settings and meeting windows on macOS, Windows and Linux, at a minimal memory footprint. Options considered: Tauri 2 (system webview), Slint (native), a hybrid of the two.

## Decision

Use **Slint** (winit backend, femtovg/skia renderer) for every window. Platform-specific window behaviour (non-activating panel, click-through, always-on-top level, all Spaces) is applied through the winit window handle in a small `platform` module per OS.

## Consequences

- Lowest RAM and binary size, and one UI codepath.
- Matching a polished look takes more hand work than CSS would.
- The pill's click-through and non-activating behaviour need per-OS native code (objc2 / windows-rs / X11 & layer-shell).
- **Licensing:** Slint is not MIT. We ship under the Slint Royalty-Free License, which allows open-source desktop apps but requires visible attribution (an "About Slint" note on the About page and in the README). Our own code stays MIT. Re-check the Slint license terms when upgrading major versions.
