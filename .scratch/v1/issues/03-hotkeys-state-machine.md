# 03 Global key capture + dictation state machine
Status: ready-for-agent
Blocked by: 01

Platform key press/release streams (macOS CGEventTap + Fn via IOHID, Windows LL hook, Linux evdev). A pure, unit-tested state machine implementing every timing rule in spec §2: hold, quick tap discard, double-tap lock, triple-tap cancel, PTT+Space escalation, stray-key discard, paste-last on full release, 20 min cap.

Done when: the state machine has a table-driven test for every rule in §2 and the keys work on macOS.

## Comments

**2026-10-30 · implementer (branch `t03-hotkeys-state-machine`)**

Done:
- `wf-core::shortcut`: layout-independent `Key`, `Combo`, `Bindings` with the §2 defaults per OS, and `validate` for the recorder rules (modifier or mouse button, 3 keys max, no left+right of a modifier, unique across actions, OS-reserved combos).
- `wf-core::dictation`: pure `DictationMachine` with injected time. Covers hold, too-short discard, double-tap lock, triple-tap cancel, press-to-stop while locked, PTT+Space escalation, stray-key discard, Cancel only while dictating, paste-last on full release, copy-last and meeting notes on press, the 20 min cap with a warning at 19, and pill click/Stop/Cancel. Also reports which key events to swallow and recovers from a lost key stream.
- `wf-platform::keys`: `listen()` with backends for macOS (CGEventTap, IOHID for Fn), Windows (low-level keyboard and mouse hooks) and Linux (evdev), plus pure native-code keymaps.
- `crates/platform/examples/keys.rs` prints live key events and the machine's commands.

Verified on macOS (Apple Silicon): `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass. Table-driven tests cover every §2 rule. The macOS keymap matches HIToolbox `Events.h` (92 keycodes). The example started an active event tap (`can swallow keys: true`) and received one real key event (a stray Enter release). `listener.rs` starts and drops real capture cleanly. Windows and Linux pass `cargo clippy --target` for `wf-platform` and `wf-core`: type-checked only, never linked or run.

Needs human verification:
- **The ticket's "keys work on macOS" is not confirmed.** Nobody has pressed the shortcuts against this code. Run `cargo run -p wf-platform --example keys`, then check: Fn hold/tap/double-tap/triple-tap, Fn+Space (no space typed), Esc while dictating (not delivered to the app), Ctrl+Cmd+V, Ctrl+Cmd+C, Opt+M, and a side mouse button.
- Whether physical Fn arrives through the tap, through IOHID, or both. The IOHID usages come from memory because the header isn't in the SDK. Also check the Globe key's system action: it isn't suppressed, so a Globe setting other than "Do Nothing" may fire on every press.
- Windows and Linux backends have never run, including both injected-event and hook paths. Linux also needs the `input` group.

Open:
- `crates/app` doesn't use the machine or the listener yet; that wiring belongs to the dictation flow.
- The recorder's capture loop (commit on release, Esc alone aborts) is left for ticket 12; only its validation is here.
- Known limits are in the notes: Space pressed before Fn types a space, Linux can't swallow keys, Secure Input may leave a key stuck.
