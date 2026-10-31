# 08 Move toolkit-free logic into wf-shell
Status: ready-for-agent
Blocked by: 07

Spec: Crates, Phase 0, Testing. Inventory: `research/surface.md`, "Where the current app code goes".

New `wf-shell` crate, `staticlib` and `lib`, toolkit-free. It takes `onboarding/{steps,trial}.rs`, `settings_window/data.rs`, `prefs.rs`, `memory.rs`, `meetings_window/disk.rs`, and the logic in `values`, `rows`, `system`, `host` and `dictation.rs`, along with the `Desktop`, `Machine` and `MeetingActions` seams. No UniFFI yet. The Slint app becomes a thin client with no visible change, including the single-source language and tier lists only where that changes nothing a user sees.

Port the logic in the 142 window tests to `cargo test -p wf-shell` on the existing fakes, keeping today's labels as the contract, as far as the logic has moved. Whatever stays Slint-only stays in `crates/app` until ticket 15.

Done when: `cargo test --workspace` and clippy pass, `cargo xtask eval` is unchanged, `wf-shell` has no `slint` in its dependency tree, and `cargo tree -p wf-shell` shows no engine.

Keep `wf-shell` and `wf-engine-host` in separate cargo commands (see ticket 07's comment), and extend `the_app_depends_on_no_engine` and the check in `assemble` to cover `wf-shell`.

## Comments
