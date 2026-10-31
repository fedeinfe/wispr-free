# 13 Onboarding
Status: ready-for-agent
Blocked by: 12

Spec: Onboarding, Phase 3. Logic and copy don't change: `research/surface.md`, `crates/app/src/onboarding` as moved to `wf-shell`.

Window 680×560, one step per page, bottom bar with Step n of 4, Back and Continue. `steps`, `start_trial`, `arm_trial`. Grants detected live at the same 1 s and 5 s rates. The first run shows all steps, later runs only broken ones. The Models step keeps the folder choice above the downloads, and closes Settings. Opening Settings during onboarding raises onboarding. Trial messages name the failed part. System Audio stays out. Run the Onboarding design review.

Done when: the ported tests and `ax-check` pass, a run with a reset dev data root shows the first-run path through to the trial step without any permission prompt being triggered by the check itself, and shots in both appearances are read by you. Don't request a grant from an automated run.

## Comments

### 2026-10-31, implementation

Done: the four steps are SwiftUI in `apple/Sources/WisprFree/Onboarding/`, one page each, in the 680×560 window, with Step n of 4, Back and Continue in the bottom bar. `wf-shell` still decides everything: `Onboarding`, `OnboardingHost` and the trial logic only moved onto the `Desk` thread through `onboarding/desk.rs`. The three things ticket 11 left are in: the Models step closes Settings, setup opens at launch, and the status menu shows Finish setup… when a permission is broken. Grants are looked at every 1 s while the window is open and every 5 s while it's closed. Opening Settings during onboarding raises onboarding. The folder choice sits above the downloads, shared with the Settings page as `ModelsFolderSection`.

The Swift rig now removes its own `wf-bridge-*` folders: `BridgeTests.swift` records each folder it makes and removes only those, at exit. A crashed run leaves its folders. The 3,160 from earlier runs are untouched, so the count still reads 3,160. The 116 folders my own break run left to prove the removal are gone.

Verified:

- `cargo test --workspace --locked --no-fail-fast` on `b0ac702`: 1406 passed, 0 failed, 86 ignored, the same ignored set as ticket 12. `swift test`: 131 of 131, plus 10 runs in a row earlier, 0 folders left. `cargo test -p xtask --test dev_bundle -- --ignored`: 2 passed. Clippy with `-D warnings` and `fmt --check` are clean.
- Breaks: Rust 22 of 23 caught. The survivor, "the window opens before the watch", changes nothing because `open` calls `watch` itself. Swift 17 of 17. Bundle 4 of 4, caught by `ax-check`. Break runs had a temp `HOME` and no `WF_*`. The `DevMachine` guard breaks ran `--lib` only, because `tests/setup.rs` wraps the real `SystemMachine`.
- `ax-check` on a fresh data root walks launch to trial, with every step's labels and presses, then the real look at launch, then Settings closing for the models step. It failed once, below, and then passed 10 times in a row.
- No prompt: tccd logged 0 `AUTHREQ_PROMPTING` lines from 13:39 to 14:40, for any process. The dev app, `pill` and the Swift test helper made only `kTCCServiceMicrophone` checks with `preflight=yes`, answered 2. `ax-walk` asks Accessibility under T3 Code, which already has it. Five `preflight=no` microphone requests appear, one each from T3 Code's helper ×3, Discord and Chrome, none of them mine. No `tccutil`.
- Every one of the 22 onboarding shots read, in both appearances. Nothing wrong. A scrolled Form's content meets the sentence with no gap, which is how the window's fixed heading works, so I left it.
- `cargo xtask bundle` for the Slint app, not installed, and the dev bundle: both pass `codesign --verify --strict`.
- `cargo xtask footprint`: idle 20.2 MB, one round of Settings 50.1 MB, six rounds 51.2 MB, peak 57.6 MB. It doesn't walk onboarding, so memory after onboarding isn't measured.
- The real data folder and `settings.toml` were stat'd only: last changed 02:23 and 22:05, both before this ticket began at 13:39. No `wispr-free-dev` in the real home. The installed app's date is 00:01, also earlier.

Couldn't verify, please add these to ticket 16's hand checks:

- The whole `--by-hand` path: real keys, `permissions::ask`, reopening the app, a real trial with the microphone, and `WindowStage.front` with a real grant. A tool run may not do any of them.
- Finish on the real bundle. A Rust bridge test covers it.
- Downloads with a network, and `NSOpenPanel` over onboarding.
- The teal accent on a key window, since tool runs are never key.

One `ax-check` failed, and I didn't find why. After `select Models` the title stayed General for 15 s and the walk said the app was in front. launchservicesd logged `SETFRONT` for the dev app at 14:22:15.750, `foreground=0`, within about half a second of the select returning, and T3 Code came back at 14:22:30.7 when the run killed the app. It's the only `SETFRONT` to the dev app in 116 launches from 14:13 to 14:40. No code path under `--behind` activates the app. Two candidates fit: a person's click on the test window, since T3 Code had come forward over WhatsApp 4 s earlier and a click would also explain the page not following; or macOS reacting to the accessibility selection. I can't tell them apart from the log, and the app's own log went with its temp home. `ax-check` now prints what the app said when it gives up, proved by a forced failure, but "took the focus" would show either candidate the same way. Ticket 12's unexplained stall may be the same thing.

My own slip: the break "a behind run shows quietly in front" added `NSApp.activate()` to the `.behind` stage for one test run. That break should not have called it. `!NSApp.isActive` still held, and launchservicesd's 25 front changes from 13:39 to 14:13 name only T3 Code, Chrome, Discord, Mail and Finder, so the focus never moved.

`log` in zsh is a builtin, not Apple's tool, and it fails silently once its error goes to `/dev/null`. My first TCC and front-app searches came back empty for that reason. Use `/usr/bin/log`.

Design review, HIG `onboarding`, review only:

- Asking for permissions in context, one step at a time, agrees with it.
- "Don't let large downloads hinder onboarding" conflicts with the Models step holding Continue until the models in use are there. The spec says the logic doesn't change, so the spec wins and nothing changed.
- No sidebar, as the spec says.

Decisions the spec didn't cover:

- `steps` returns `SetupPage?`, the page while the window's open. `start_trial` is the mouse trial. `arm_trial` isn't exported, because Rust arms the trial on arrival.
- "One sentence" against "copy doesn't change": four steps had none, so four are new. The keys one comes from the prototype.
- Onboarding lives on the `Desk` thread and shares its `Pages` with Settings, because `Rc` things live there. Settings' ask and a Close can race, harmlessly. `ask_for` syncs with `desk.ask()` if Onboarding is open.
- `--setup` is opt-in and only works with `--behind`, so tool runs don't open at launch by accident.
- New events `OpenQuietly`, `Close` and `Setup`. Open comes before Raise, so a window orders in before the app activates, the reverse of the spec's order, to keep a quiet open quiet.
- `SwiftWindow.unreported` counts closes Rust started, so Swift's report of them isn't taken for the user closing the window.
- `DevMachine`: a tool run reads the Mac and does nothing to it, a rehearsal has fake permissions and answers its asks at once, a by-hand run reaches the Mac. `set_blocked` is silent on tool runs too, or the pill said the shortcut wasn't reaching.
- `KeySlot` and `listen_until_allowed` went: a by-hand run uses `KeyWatch` on the `Desk`.
- A real bug, also in the Slint app: `every(LOOK_IDLE)` after `open` lost the 1 s rate at launch. `OnboardingHost::watch` fixes it for both.
- `PagesState` gained `offered_tiers`, `models_in_use` and `models_to_download`. `View` became `TrialView`, as `View` collides with SwiftUI. `ax-walk` prints `enabled`. `orders()` in the tests filters `Setup` and `Recent`. The `Desk`'s timers gained repeating alarms that can reset or stop themselves.
- Hardware on the Models step comes from `Pages`, so it matches Settings.
