# 14 Meetings
Status: ready-for-agent
Blocked by: 11

Spec: Meetings, Bridge, Phase 4, Memory. Behaviour: `research/surface.md`. Look: `prototype/Proto.swift`, `prototype/meetings-dark.png`.

`NavigationSplitView` 1000×680, list and `.searchable(placement: .sidebar)` with the 150 ms debounce, start button in the toolbar. Segmented Summary, Notes, Transcript, Chat in `.principal`, Export and the rest in `.primaryAction` with `ToolbarSpacer`. Title saves after 700 ms. Stage chips become a status row. Chat streaming, `TimeChips`, suggestions, Stop, and `.glassProminent` Send on the floating input. Library calls are `async`, in order, and never on the main actor. Run the Meetings design review.

Done when: the ported tests and `ax-check` pass, a real 30-minute fixture displays with no hitch and you say how you measured that, footprint after using every window is recorded against the 55 MB goal, and shots in both appearances are read by you. If it ends above 55 MB, report it. Don't build the separate-process fallback.

## Comments

### 2026-10-31, implementation

Testing was cut short on the user's instruction: one pass of each check, no deliberate breaks.

**Done.** The Meetings window in SwiftUI as the ticket asks, on a new `meetings-window` Rust thread. Changes reach Swift in batches through `ShellEvent::Meetings`.

**Verified, one pass on the final code.**
- `cargo test --workspace`: 1433 passed, 0 failed, 86 ignored.
- `swift test`: 174 of 174. One compiler warning is left, an unneeded `try` in `MeetingsViewTests`.
- `ax-check`: passed, including list, tabs, chat, saves, export and release on close. The focus failure from ticket 13 did not show.
- Footprint, with the real meeting: idle 20.5 MB, every window once 63.8 MB (peak 74.0), six rounds 62.0 MB (peak 74.8), rehearsal 63.3 MB (peak 74.3). **That is 7 to 9 MB over the 55 MB goal.** It doesn't grow across rounds, so it isn't a leak. Taking each tab away in turn saved at most about 3 MB, so no single part causes it. Live heap was 20.7 MB against 41.4 MB dirty in `Malloc Small`, and pressure relief didn't return it. No separate-process fallback was built.
- 30-minute fixture (195 lines, 63 summary lines, kept on the SSD), `cargo xtask meeting-walk`, 4 ms timer on the main thread: scrolling 10800 points longest stall 5 ms, jumps 7 ms, search 4 ms, streaming 41 ms, tab changes 51 ms at most, opening the meeting 99 ms. Opening the window stalled once for 130 ms, the only stall over 100 ms. The window sat behind others, so this measures main-thread stalls, not drawn frames.
- Shots read in both appearances for the seeded meetings; for the real meeting, the shots needed for layout. A real recording transcribed and summarised passed earlier in the ticket (2 tests, 437 s), not rerun on the final code.

**Not verified.** Not run after the limits changed: deliberate breaks, clippy, `fmt --check`, the ignored `dev_bundle` tests, the Slint bundle, `codesign --verify`. For ticket 16, on a real bundle: the delete dialog, typing title and notes, real recording start, stop and keep recording, real LLM chat in the app, glass and tint on a key window, drawn frames on a visible window.

**Decisions the spec didn't cover.**
- Library calls are ordered, synchronous sends to the disk thread, not UniFFI `async`. That keeps the order and stays off the main actor.
- Close doesn't wait for the desk; reopen and quit do, so saves reach disk.
- Notes use a plain `NSTextView`. `TextEditor` allocated 170 MB of GPU memory for a moment (peak 249 MB against 74 MB now).
- `.toolbar(removing: .title)` keeps the tabs and actions out of overflow at 760 wide.
- New Rust labels and states (`DetailHint`, `StartLabel`, `CanExport`, `CanDelete`, `ChatHint`), so views hold no rule. `ax-walk` gained `value` and `set`. Rehearsal has a scripted LLM.
- Tabs stay disabled with nothing picked. Regenerate and Update stay in the Summary tab.

**Design review (HIG sidebars, toolbars, liquid-glass).** Applied: borderless toolbar symbols, no title, secondary instead of tint for the speaker and gear marks. Spec wins on glass Send in the content layer. "Every toolbar item in the menu bar" can't apply to an accessory app.

**Leftovers.**
- In the light shots, search text reads white on a window that isn't key. Cause not found; check on a key window.
- `meetings-transcript-end` doesn't scroll with the real meeting, though the walk scrolls it.
- An AX set on the title field doesn't reach the binding, so the title is covered by ViewInspector and `meetings_bridge`, not `ax-check`.
