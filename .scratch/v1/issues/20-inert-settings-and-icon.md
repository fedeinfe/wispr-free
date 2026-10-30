# 20 Make the last inert settings work, and give the app an icon
Status: ready-for-agent
Blocked by: 19

Added during implementation: tickets 12 and 19 left two settings saved and shown in Settings with no effect. Spec §3 and §5 ask for both.

- `media.while_dictating`: mute system output or pause media while dictating, and put it back afterwards, also when a dictation is cancelled or fails and when the app quits mid-dictation.
- `style.auto_add_corrections`: when the user corrects a word just after a dictation, add it to the dictionary.
- The bundle from ticket 13 has no app icon. Make an original one.

macOS only for now.

Done when: both settings change what the app does on macOS, each off by its setting, and `Wispr Free.app` shows an icon.

## Comments

**2026-10-30 · implementer (branch `t20-inert-settings-icon`)**

macOS only, as the user decided. `wf-platform` still passes clippy for Windows and Linux, where `SystemMedia` is a stub. Nothing was pushed, and `main` and `v1` are untouched.

Done:
- `media.while_dictating`. The flow decides when and a `wf-media` thread does the system calls, so a key-down never waits. Media is quiet for as long as the mic is open. The coordinator looks at the recording rather than at what ended it, so a cancel, a failed dictation, a tap, hands-free and a stray key all take the same way back.
- Mute uses the default output's own mute and only undoes its own: an output that was already muted stays muted. The device's UID goes into `media-note.json` before the mute and comes out after the unmute. Quitting restores, and the next launch finishes what a crash left, whatever the setting says by then. A device that's unplugged stays on the list and is retried.
- `style.auto_add_corrections`. For 30 s after a paste a `wf-corrections` thread reads the field once a second. A word is learned only if the inserted text was first seen whole, then one word of it changed by a small edit and held still for 4 looks. A toast says what was added, and the word is in the dictionary list, where Remove works.
- App icon, from `crates/app/ui/icons/app-icon.svg`, rendered by `sips` and `iconutil` at bundle time, wired through `CFBundleIconFile`.

Decided here:
- **Pause reaches only Music and Spotify.** macOS has no public way to learn what's playing. MediaRemote is private and from 15.4 serves only `com.apple.*` processes. A media key is a toggle, so it would start whatever was paused. CoreAudio's process list shows who holds the output open, not who is playing, so a call or an idle engine would count. Music and Spotify answer `player state`, so they are asked first, and only what was paused is resumed. Browsers keep playing, and the Settings text says so. Mute covers everything.
- Pause waits 1 s into a recording, because a tap or a stray key is over by then. Music plays on through that second. Mute is immediate.
- Paused players aren't recorded for the next launch. Only quitting resumes them. After a crash they stay paused, because music coming on by itself at some later launch is worse.
- Choosing Pause in Settings raises the OS's question then, for players that are running. Otherwise the first dictation would raise it and take the focus.
- Nothing is muted or paused while a meeting records: the user would miss the call, and the Others channel might go silent.
- Media comes back when the mic closes, not when the text lands, so a slow cleanup doesn't hold the sound.
- **Learned words go to the dictionary only, never to replacements.** Both ASR engines and LLM cleanup already read the dictionary. A replacement reads "always swap", so one wrong guess such as form → from would rewrite every later "form".
- The rule refuses words under 4 characters, changed numbers, contractions, addresses and paths, words cut short, and case-only changes except an inner capital ("GitHub", not "NASA" or "Hello"). A capitalized word that opens a sentence is refused too, since the capital says nothing there.
- Context awareness off also stops the watch: it means the field isn't read at all.

Verified:
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings` clean.
- `cargo test --workspace --locked --no-fail-fast`: 1053 passed, 0 failed, 75 ignored, 96 suites. New here: `tests/media.rs` 34, `tests/corrections.rs` 16, plus the rule's unit tests.
- Mutations: 9 deliberate breaks to the guards (note before mute, already-muted check, the level check, both settings, watch end, new dictation, secure field, seen whole). Each one failed a test. One survived at first and got its own test.
- `cargo xtask bundle` under a temp HOME, signed ad-hoc: `codesign --verify --deep --strict` exit 0. The `.icns` holds all 10 sizes and the PNG was checked by eye, down to 16 px. Settings text checked in offscreen renders.
- The scripting dictionaries of Music and Spotify on this Mac declare `player state`, `pause` and `play`.

Not verified, on purpose (no real system calls while testing):
- No real mute, unmute, `osascript`, permission question or AX read of a correction has run. `crates/platform/src/media/macos.rs` compiles and its script text is tested, but it has never talked to the OS.
- `--probe-clip` and the real-model tests were not run.
