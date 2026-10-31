# 10 Pill, card, toast, menus and sounds
Status: ready-for-agent
Blocked by: 09

Spec: Pill and card, Menu bar icon, Architecture (panel recipe, Swift timer calling `pill_tick`), Settled mismatches (the Language menu), Phase 1, Memory. Look reference: `prototype/Proto.swift`, `prototype/pills-*.png`.

Every pill state with its sizes and timings from `wf_core::pill`, card, toast, drag, snap, `pill.toml`, Esc, `ignoresMouseEvents` from `Frame.hit_test`, real `NSMenu`, sounds behind `prefs.sounds`. `NSStatusItem` with a template image and the same items. The hidden main menu supplies ⌘, ⌘W and the Edit commands. `PillButtonStyle` on the pill and card. Reduce Motion crossfades.

Add `cargo xtask shots` for every pill state over its own backdrop, and a `cargo xtask` report of `phys_footprint`. Run the pill and menu bar design reviews.

Done when: `cargo test` and `swift test` pass; shots of every state are read by you and recorded in your comment; the footprint is under the spec's 25 MB idle goal or the gap is stated; the flow reaches Rust end to end through the bridge in a test with fake engines. The real-dictation and pointer hand checks for this phase go to ticket 16.

## Comments
