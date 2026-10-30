# 13 First-run onboarding + permissions
Status: ready-for-agent
Blocked by: 05, 11, 12, 19

Steps: mic → accessibility/input monitoring (mac) or input group (Linux) → pick a tier and download models → try a dictation. Deep links into System Settings, re-check loops, and clear recovery when a permission is revoked.

Done when: a clean macOS user account reaches a working dictation without reading docs.

## Comments

**2026-10-30 · implementer (branch `t13-onboarding`)**

macOS only, as the user parked Windows and Linux. The Linux input-group step isn't built. `wf-platform` still checks and passes clippy for Linux and Windows. Nothing was pushed, and `main` and `v1` are untouched. Packaging is on ticket 18.

Done:
- Four steps: microphone, keyboard and paste (Input Monitoring plus Accessibility), models, try a dictation. No account or marketing step.
- It shows once. Afterwards a broken step brings back only that step, in a window that doesn't take focus. Closing the window pauses setup, and the tray offers "Finish setup…".
- The window is created on open and dropped on close, like Settings. It reuses the Settings controller through a `Surface` trait, so tiers, downloads, the custom models folder, the missing-drive report and the shortcut recorder aren't copied.
- Permissions go through the `Machine` seam, with a test for not asked, denied, granted, revoked while running and "granted but the keys are stuck". `wf_platform::permissions` gained `ask()` and `settings_link()`.
- "No input device" is its own state with its own message, and the page notices by itself when a microphone appears, which enables Continue. This Mac Studio is that case.
- The trial pastes nothing, shows the text in the window, and names the part that failed: shortcut, microphone or models.

Differences from the brief:
- `start_trial` alone can't prove the keys, because a button starts it. `Dictation::arm_trial` makes the real shortcut's next dictation end as a trial. The mouse fallback still uses `start_trial`.
- Only `Pill(ListeningPtt)` proves the keys. Permissions reading granted doesn't, which is why "Quit and reopen" exists.
- Finish needs all three parts proven, so the real shortcut has to work. The mouse fallback only narrows down a failure: it marks the shortcut failed and can't finish. Someone stuck there can change the shortcut on that page, or close the window and finish later from the tray.

Decided here:
- An unplugged models drive or a missing microphone doesn't reopen onboarding after it's finished. Both show their own error when a dictation starts, and something that comes and goes shouldn't raise a window.
- Missing models are checked at launch only, and permissions are checked while running.
- The bare `wispr-free` binary skips onboarding and listens to keys as before, and so does `--probe-clip`. Only the bundle runs it.
- While onboarding is open, Settings comes forward as onboarding instead, because they share the models folder and downloads don't lock files. The models step closes an open Settings.

Memory incident on this branch, caused by the tests and not the product: `install_models` wrote fake models out in full with `fs::write(path, vec![0; size])`. That held about 20 GB across 20 call sites on 18 test threads, wrote about 36 GB per run, and my retries repeated it until the Mac was freezing (17.3 GB seen by the user). Fake models now use `File::set_len`, as `crates/dictation/tests/support/mod.rs` already did, and both window test binaries abort over 1 GB (`crates/app/tests/support/memory_cap.rs`). A temporary 20 MB limit did abort a run, so the guard works.

The real window was measured for the same fault: 5 open and close cycles of 3600 looks with rendering stayed flat at 12.7 to 12.8 MB, and closing gave 8.9 MB. No runaway. The same check found three real faults, all fixed: the 1 Hz look rebuilt the sidebar and every model row and wiped a shown model error; mic looks could pile up behind a slow one; and the look timer kept a closed host alive.

Verified on `14a4a6d`:
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean.
- `cargo test --workspace --locked --no-fail-fast`: 687 passed, 0 failed, 66 ignored, 79 test binaries.
- `cargo check` and `cargo clippy -D warnings` for `wf-platform` on `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`: clean. Check only, nothing ran.
- `settings_probe persist-write` then `persist-check`: 45 values, 0 differ.
- Window tests' peak RSS: `onboarding_window` 71 MB on 6 threads and 88 MB on 18, `settings_window` 87 and 96 MB.
- 30 offscreen PNGs, 15 states in light and dark. 12 of the final render were looked at, across every step in both themes. Fixed from those: a tick that squeezed the selected row, and steps that looked clickable.
- Real dictation through `--probe-clip` from the bundle, in a temp HOME: details on ticket 18.

Not verified, needs a human on macOS:
- `ask()` and the deep links were never run, because they would touch the real privacy state. The pane anchors come from macOS's own `TCCServiceList.plist`.
- A real mic, a real shortcut and a real window on screen. Everything here is the offscreen renderer or `--probe-clip`, so femtovg's look and the focus of a repair window are unseen.
- "Done when" (a clean macOS user account reaches a working dictation without docs) is not met until someone does that.
- "Quit and reopen" was not run.
