# 07 Split the engines into wf-engine-host
Status: ready-for-agent
Blocked by: none

Spec: Crates, Architecture, Build, Phase 0. Proof: `prototype/stack/repo-changes.patch`.

Add the `engines` feature to `wf-dictation`, off by default, gating `load_engine` and `run_engine_host`. Add the `wf-engine-host` crate: `--engine-host`, `--version`, the dev-only `wf-probe` binary (`--probe-clip`), and the real tests (`engine_host`, `real_dictation`, `real_meeting`) moved out of `crates/app`, meaning unchanged.

The Slint app starts hosts from `Contents/Helpers` and stops being a host itself. Dylibs move to `Contents/Frameworks`, and the `.cargo/config.toml` rpath follows. `cargo xtask bundle` signs inside out, dylibs then helper with `-i com.wispr-free.wf-engine-host` then the app, never `--deep`. The `--version` smoke test targets the helper. Dev runs from `target/` need a path to the helper too, so don't break `cargo run`.

Done when: `cargo test --workspace` and clippy pass, `cargo xtask eval` is unchanged against the baseline, `cargo xtask bundle` makes a bundle in `target/` that passes `codesign --verify --strict` and has the layout above, and that bundle's engine host answers the smoke test. Don't install it: "the installed app dictates" is checked in ticket 16.

## Comments
