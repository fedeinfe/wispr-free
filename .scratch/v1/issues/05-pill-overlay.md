# 05 Pill overlay (Slint)
Status: ready-for-agent
Blocked by: 01, 02

Transparent, non-activating, always-on-top, all-Spaces window with click-through outside the pill. Every state in spec §1, 10-bar waveform driven by the level meter, edge docking with drag-snap, follows the cursor's display, avoids the Dock/taskbar. Pill menu. Original sounds.

Done when: the pill never steals focus from the app being typed into, and all states can be triggered from a debug menu.

## Comments

**2026-10-30 · implementer (branch `t05-pill-overlay`)**

Done:
- `wf_core::pill`: pure model of every §1 state, hover, press/drag/Esc, drop snapping, display following, Dock and taskbar avoidance, click-through decision, Hide for 1 hour, sound cues. `pill::dock` for geometry, `pill::waveform` for the ten bars (bell, travelling wobble, frame-rate-independent smoothing).
- `wf_platform::overlay`: macOS (`NSPanel` swap, screen-saver level, all Spaces, full-screen auxiliary), Windows (no-activate, topmost tool window), X11 (override-redirect, periodic raise). Screens, global cursor and Esc state on all three.
- App: `PillWindow` in Slint, `PillController` as the API for later tickets, tray "Debug pill" menu with all 8 states, the 4 sounds and a real-mic switch. Native pill menu with every §1 entry. Edge and sounds saved to `pill.toml`.
- `wf_audio::sounds`: four cues synthesized at runtime from decaying sine notes at the device's rate. No sample files.
- Hide for 1 hour and Quit work. The other pill menu entries emit `PillEvent`s that nothing consumes yet (tickets 10, 12, pipeline wiring). Microphone lists the real devices; Language and Pipeline quality use placeholder lists.

Tests at five seams, chosen by the implementer since no human was available to confirm them: `pill::dock`, `Waveform`, `Pill`, `sounds::synthesize`, and the `PillWindow` component through Slint's test backend. Prefs and debug-name parsing have unit tests as well.

Decisions worth a look:
- **Private API on macOS.** `_setPreventsActivation:` is needed because the window begins as winit's `NSWindow`, and the public style mask has no effect after init. Without it, and without re-ordering the panel, a click steals focus. Details in the shared notes. `apply` reports an error if a macOS update removes it.
- **Click-through deviates from the spec's Windows row.** It uses cursor polling plus winit's `set_cursor_hittest` on every OS, not a window region, because a region would also clip the tooltip and it keeps one code path.
- **Hidden** paints nothing and ignores the mouse, but the window stays mapped. Re-showing it is the risky path for focus, particularly on Windows.
- **Linux** prefers X11 (XWayland) whenever `DISPLAY` is set. winit has no layer-shell, and Wayland has no global cursor.
- **Side docks** keep the capsule horizontal except at Resting, which lies along the edge. The pill is always centred along its edge, since only the edge is saved.

Verified on macOS (Apple Silicon, two displays):
- `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` on the tree merged with `v1`.
- Cross-checks: `wf-platform` passes `cargo check` on `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` on the merged tree. The whole app passed on Windows before the merge; it can no longer be cross-checked from macOS, because `ring` and `opusic-sys` (from `v1`) need a Windows C toolchain. The whole app has never been cross-checked for Linux (`fontique` needs fontconfig), so the Linux-only winit calls in `pill.rs` were type-checked in a scratch crate.
- **Focus, with a real focused text field in a separate app**, before the merge: launching, hovering, clicking Ready, clicking Cancel and Stop, and clicking through outside the capsule all left that app active and key with the field as first responder, in 19 of 19 samples. A key typed while listening landed in the field, and the click-through reached the app below. While the native menu is open that app stays frontmost and key status moves to the menu, returning when it closes, as with any menu.
- All 8 states, triggered from the real tray debug menu via Accessibility and screenshot-checked over live apps, on the pre-merge and merged binaries. The frontmost app did not change. No macOS window code changed after the focus run, but the click scenario itself was not repeated on the merged binary, to avoid taking the foreground from someone using the machine.
- Window layer 1000; position flush with the Dock's top; follows the cursor across both displays. Real drag from the bottom to the left edge snaps, saves `edge = "left"`, and survives a restart. Hide for 1 hour leaves nothing visible. Menu Quit exits.
- The sounds example ran on the real output device with no error. Release build: 33 MB `phys_footprint`, ~0.1 % CPU at rest.

Needs human verification:
- **Listen to the sounds** (`cargo run -p wf-audio --example sounds`). Only their shape has been checked: duration, peak, fades, pitch direction.
- **Esc during a drag.** Tested only in the pure model; synthetic Esc was not sent because a person was using the machine. `CGEventSourceKeyState` may also need Input Monitoring. Check it cancels the drag from a normal launch.
- **Dropping on the other display**, and on the right edge. Tested in the pure model; the live attempts missed the capsule.
- **Waveform on real speech.** Debug menu → "Use real mic level". Agent shells only get silence (see ticket 02), and the switch was not exercised, to avoid a mic prompt on a machine in use.
- **All Spaces and full-screen apps.** The collection behaviour and level are set, but no Space was switched and no app was full-screened.
- **Windows and Linux: nothing has run there.** Check focus by clicking the pill while typing elsewhere. Windows: `WS_EX_NOACTIVATE` surviving hit-test changes relies on `overlay::raise`, found by reading winit's source. X11: override-redirect window, `_NET_WORKAREA` is per desktop rather than per monitor, and Esc assumes keycode 9. **Linux context menus are drawn by Slint inside the 320×120 window and will be clipped, and the window is click-through outside the capsule while they are open.** Wayland without XWayland has no always-on-top, no cursor following and no real click-through.
- Debug menu's sound items and the tray "Quit" were not pressed.

