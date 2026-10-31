# 07 Split the engines into wf-engine-host
Status: ready-for-agent
Blocked by: none

Spec: Crates, Architecture, Build, Phase 0. Proof: `prototype/stack/repo-changes.patch`.

Add the `engines` feature to `wf-dictation`, off by default, gating `load_engine` and `run_engine_host`. Add the `wf-engine-host` crate: `--engine-host`, `--version`, the dev-only `wf-probe` binary (`--probe-clip`), and the real tests (`engine_host`, `real_dictation`, `real_meeting`) moved out of `crates/app`, meaning unchanged.

The Slint app starts hosts from `Contents/Helpers` and stops being a host itself. Dylibs move to `Contents/Frameworks`, and the `.cargo/config.toml` rpath follows. `cargo xtask bundle` signs inside out, dylibs then helper with `-i com.wispr-free.wf-engine-host` then the app, never `--deep`. The `--version` smoke test targets the helper. Dev runs from `target/` need a path to the helper too, so don't break `cargo run`.

Done when: `cargo test --workspace` and clippy pass, `cargo xtask eval` is unchanged against the baseline, `cargo xtask bundle` makes a bundle in `target/` that passes `codesign --verify --strict` and has the layout above, and that bundle's engine host answers the smoke test. Don't install it: "the installed app dictates" is checked in ticket 16.

## Comments

### 2026-10-31, implementation

Done: `engines` feature on `wf-dictation`, off by default, gating `load_engine` and `run_engine_host`. New crate `crates/engine-host` (`wf-engine-host`, with `--engine-host` and `--version`, and `wf-probe`). `engine_host`, `real_dictation` and `real_meeting` moved there with only the binary name and the commands in their headers changed. The Slint app starts hosts through `remote::engine_host_beside`: `Contents/Helpers` in a bundle, the binary beside it in `target`. `cargo xtask bundle` puts dylibs in `Contents/Frameworks`, signs dylibs, helper with `com.wispr-free.wf-engine-host`, then the app, and runs the `--version` smoke test on the helper. It now fails if the app binary links anything but macOS.

Verified, on the merged tree unless noted:
- `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked --no-fail-fast`: 1093 passed, 0 failed, 76 ignored.
- `cargo tree -p wispr-free` holds no `llama-cpp-2`, `whisper-rs` or `sherpa-onnx` (now a test), and the bundled app's `otool -L` has no rpath library.
- `cargo xtask bundle`, signed with Wispr Free Local: layout as the ticket says, 6 dylibs in `Frameworks`, `codesign --verify --strict` passes, helper identifier and designated requirement correct, helper `--version` from `/` prints `wf-engine-host 0.1.0`.
- `cargo xtask eval` with Qwen3-1.7B: 83/89 = 93.3% on this branch and on a clean export of dd1c516, and the reports differ only in timings. The 73/80 in older notes predates cases added since.
- Before the last app-only change (the `--probe-clip` refusal), with real models: `real_dictation` 13 of 13, `real_meeting` 2 of 2 including the 30-minute fixture, and `wf-probe`'s real dictation. Logs and scripts are in `wispr-free-notes/apple-ui/07-engine-host/`.
- That bundle's signed helper and `Frameworks`, copied to a temp `.app` beside `wf-probe`, ran a real Balanced dictation. `lsof` showed each host's `libllama` coming from `Frameworks`.

Couldn't verify:
- "The installed app dictates" is ticket 16's.
- I never launched the Slint app, in a bundle or by `cargo run`, because it would take keys. Its path to the helper is covered by tests only.
- Linux and Windows CI weren't run. `metal` stayed behind a macOS target, and the new tests are gated, but nothing proves that here.
- The installed app (pid 80103) exited at 02:12:27, during the final `cargo test`, with a voluntary exit and no quit event in the log. Its last engine hosts had started 37 s earlier, which looks like the user dictating. No test touches it, and the same suite ran earlier without stopping it, but I can't prove the cause. Not relaunched: `open` on a bundle is off limits.

Decisions the spec didn't cover:
- **Spec correction, Build step 1.** `cargo build -p wf-shell -p wf-engine-host` as one command unifies features, so the shell would get every engine. `xtask` builds the binaries one at a time, and tickets 08 and 09 must keep doing that. `the_app_depends_on_no_engine` and the check in `assemble` catch it for the app, and need extending to `wf-shell`.
- `engines` leaves out `wf-pipeline/metal`, unlike the proof's patch, which would turn Metal on for Linux and Windows while CI still builds them. `wf-engine-host` asks for it under a macOS target, as `wf-dictation` did.
- `cargo run -p wispr-free` can't build the helper: stable cargo has no way to build another package's binary for it. So it's `cargo build -p wf-engine-host` first, which the README now says, and the app prints that line if the helper is missing. A helper left over from an older build isn't detected.
- `--probe-clip` moved out of the app, not copied. `wf-probe` needs `--data=<folder>` and reads and writes nothing else, so it can't reach the real data root. It runs the flow without the UI, so `probes/perf/tiers.py` and the `probes/19-dictation` and `probes/13-onboarding` scripts need pointing at it, and their numbers will no longer include Slint.
- The app now refuses `--engine-host` and `--probe-clip`. Without that, an older app still running after an install would start a second full app as a "host", and an old script would get a real app with real keys. Ticket 15 deletes this with the app.
- The host keeps mimalloc, since that's what hosts ran on as the app's binary.
- `xtask` still verifies with `--verify --deep --strict`. `--deep` there only checks more. Signing never uses it.
- The helper still links AppKit through `wf-platform`, and never calls it. That's no worse than the old host, but it falls short of "no toolkit" until `wf-dictation` stops depending on `wf-platform`'s UI side. It has no process tap import, so it needs no weak CoreAudio link.
- `GLOSSARY.md` and ADR-0005 still say a host is the app's own binary, and ADR-0005 says there's nothing extra to sign. The spec leaves both to ticket 15. The README lines ticket 15 cites as 28 and 30 have moved down, see `grep -n Slint README.md`.

### 2026-10-31, correction from the integrator

The installed app's exit at 02:12:27 wasn't caused by the suite. The system log shows an AppKit `sendAction:` on its main thread 25 ms before a voluntary exit, so it was quit from its menu. `pkill` in `real_dictation.rs` is scoped with `-P` to the test's own children, and pid 40460 was the app's own engine host going down with it.
