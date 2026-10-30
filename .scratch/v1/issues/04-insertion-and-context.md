# 04 Text insertion + context
Status: ready-for-agent
Blocked by: 01

Clipboard save → transient write → paste keystroke → restore after 500 ms, per OS, with a terminal Shift+Insert fallback. Context: focused app id, window title, ±500 chars around the cursor, selection (AX / UIA / AT-SPI). Fallback toast when nothing is focused.

Done when: text lands in TextEdit/Notes, Chrome, VS Code and Terminal on macOS, and the previous clipboard is restored.

## Comments

**2026-10-30 · implementer (branch `t04-insertion-context`)**

Done:
- `wf-platform::insert`: OS-free `Inserter` over `Clipboard` / `PasteKeys` / `ContextSource` / `Sleep` traits. Save → transient write → settle 20 ms → paste → wait 500 ms → restore. Skips the restore if the user copied something meanwhile. Falls back to leaving the text on the clipboard (`InsertOutcome::Copied`) when no field is focused or the keystroke can't be sent. An unreadable clipboard doesn't block the paste, and a failed restore still reports the paste.
- `paste_chord`: Cmd+V on macOS, Ctrl+V on Windows/Linux, Shift+Insert in Windows/Linux terminals (known-terminal list).
- `wf-platform::context`: `Context` (app identity, window title, `Focus`, `TextAround`), ±500 chars around the cursor, selection capped at 2000 chars, UTF-16 windowing that survives split surrogate pairs. Password fields are never read.
- macOS: NSPasteboard (all types of all items, nspasteboard.org transient/concealed markers), CGEvent Cmd+V with the key looked up for the current layout, AX context.
- Windows: SendInput, arboard + `GetClipboardSequenceNumber`, UI Automation context. Linux: XTest, wtype/ydotool on Wayland, arboard, X11 EWMH app and title.
- `insertion_probe` example for manual checks. `SYNTHETIC_EVENT_MARKER` tags the keystroke so ticket 03's capture can ignore it.
- Tests (test-first): 9 on `Inserter`, 4 on the context text window, 9 in-module on role mapping and Wayland arguments, plus 1 `#[ignore]`d real-pasteboard round trip.

Verified on macOS (Apple Silicon):
- Real clipboard round trip (`system_clipboard`, run with `--ignored`): multi-format restore, transient markers, change detection. The user's clipboard text hash matched before and after.
- Layout key lookup on the Italian layout: V key found on the main thread; off the main thread with no run loop it falls back after the timeout without crashing.
- AX read of the frontmost app runs and returns an app id, without printing its content.

Windows and Linux: only compiled and linted by cross-checking from the Mac (`cargo clippy --all-targets -D warnings` for both targets). Never run.

Not done:
- **Linux AT-SPI context is not implemented** (spec §7 says best effort): app and title come from X11 only, no text around the cursor, and nothing on native Wayland. Needs a follow-up decision.
- uinput paste on Linux (XTest and wtype/ydotool only).
- The toast itself and keeping the "last transcript" are UI/history work (05, 10); this ticket returns the outcome they need. Nothing in `crates/app` calls insertion yet.

Needs manual verification (the done-when is **not** proven yet):
- No synthetic paste was ever sent, to avoid typing into the user's real apps. The host terminal has Accessibility, so `insert` is live. Run `cargo run -p wf-platform --example insertion_probe -- insert "hello" --delay 3`, click into a scratch field, and check text lands and the clipboard comes back in: TextEdit, Notes, Chrome, VS Code, Terminal.
- `context --delay 3` in the same apps: focus kind, ±500 chars, selection. No real text field has been read yet, including `AXStringForRange` and the UTF-16 path against a live app. VS Code and Chrome are the risky ones (accessibility tree built on demand).
- With a non-text control focused (Finder list), the outcome should be `Copied`.
- A clipboard manager, if installed, should not record the dictated text.
- Everything on Windows and Linux, including Shift+Insert in terminals and clipboard managers skipping the text.
