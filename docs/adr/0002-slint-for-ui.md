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

## Amendment: software renderer (2026-10-30, ticket 12)

The app selects Slint's software renderer (`wispr_free::select_renderer`), not femtovg or Skia. `SLINT_BACKEND` still overrides it for comparisons.

Measured on Apple Silicon, in `phys_footprint`: with femtovg, a 200×200 empty window takes the process from about 15 MB to about 217 MB, and that memory stays after the window closes. The OpenGL stack costs that much regardless of window size, so spec goal 4 (idle under 40 MB) and "closed window releases its memory" can't both hold with it. With the software renderer the same window costs about 6 MB.

The renderer is chosen per process, so every window shares it. On macOS softbuffer presents opaque images (`CGImageAlphaInfo::NoneSkipFirst`), so a software-rendered window cannot be see-through. The pill (ticket 05) needs transparency and has to solve that without bringing OpenGL into the process. Linux and Windows have not been measured.
