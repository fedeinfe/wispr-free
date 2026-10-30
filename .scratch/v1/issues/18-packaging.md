# 18 Packaging + releases
Status: ready-for-human
Blocked by: 13

macOS signed/notarized .dmg (Metal), Windows .msi/.exe (CPU+Vulkan; CUDA variant), Linux AppImage + .deb. GitHub Actions release workflow. Human: Apple Developer ID and Windows signing certificate decisions.

Done when: tagged releases publish installers for all three OSes.

## Comments

**2026-10-30 · implementer (branch `t13-onboarding`)**

A local slice for this Mac only, done with ticket 13. No `.dmg`, Developer ID, notarization, Windows, Linux or release workflow, all parked by the user, so this ticket stays open.

Built: `cargo xtask bundle` makes `target/Wispr Free.app`. `--install` copies it to `~/Applications` and refuses to replace anything that isn't ours. `cargo xtask signing-identity` makes a stable local signing identity, once.
- `Info.plist` has the microphone and system audio usage descriptions and `LSUIElement`, which fits the Dock setting's default. Minimum macOS 13.0.
- llama.cpp's six dylibs sit beside the binary in `Contents/MacOS`, the option in ADR-0003 that the `@executable_path` rpath already fits, so there is no rpath to patch and engine hosts need nothing extra.
- `crates/app/build.rs` weak-links CoreAudio (ticket 15), and `nm` shows `_AudioHardwareCreateProcessTap` as a weak external.
- `CMAKE_DISABLE_FIND_PACKAGE_OpenSSL` in `.cargo/config.toml` stops `libllama-common` linking Homebrew's OpenSSL by absolute path. That HTTPS client is never called (ADR-0004).
- The bundler takes the dylibs from the build script's `OUT_DIR`, because `llama-cpp-sys-2` keeps whatever is already in `target/release`. If a foreign library still turns up it cleans that crate once and retries.

**Does a rebuilt app keep its permissions?** Ad-hoc, no. Self-signed with a stable identity, the signature check that macOS uses says yes. TCC stores the designated requirement of the app it granted.
- Ad-hoc requirement: `cdhash H"..."`, one exact build. Stable identity: `identifier "com.wispr-free.app" and certificate leaf = H"..."`, any build signed by that key.
- Tested with `codesign --verify -R`, the check tccd applies, on real rebuilds where `main.rs` changed and `--version` output differed:

| Build tested | Against the requirement of | Result |
|---|---|---|
| Stable identity, changed code, cdhash `47f20593…` | Stable build `a58e0c4d…` | accepted, exit 0 |
| Ad-hoc, changed code, cdhash `74cc7511…` | Ad-hoc build `165bf3ec…` | refused, exit 3 |
| Ad-hoc, same source rebuilt | Ad-hoc build `165bf3ec…` | same cdhash, accepted, exit 0 |
| Same app, control with a wrong identifier | n/a | refused, exit 3 |

- TCC itself was not exercised, since the brief forbids changing the real privacy state. A person still has to confirm it: grant, rebuild, see whether it holds.
- The default stays ad-hoc so a plain `cargo xtask bundle` touches no keychain. Run `cargo xtask signing-identity` once **before the first grant**, because moving from ad-hoc to the stable identity is itself a change of requirement.
- The identity is a self-signed certificate in a dedicated keychain, `~/.config/wispr-free-dev/signing.keychain-db`, with an empty password so nothing prompts. The login keychain and search list aren't touched. That key can only sign as a certificate no other Mac trusts, but anyone who can read the file can sign as it, and it stays good for this Mac's grants.

Verified on `14a4a6d`, final ad-hoc build:
- 55 MB. `codesign --verify --deep --strict` passes. `codesign -dv`: Identifier `com.wispr-free.app`, Signature `adhoc`.
- `otool -L`: only `/System`, `/usr/lib` and `@rpath` in the binary and all six dylibs, and CoreAudio is weak.
- `--probe-clip` from the bundle in a temp HOME: Balanced "Selected and translated by C. Fillingham, Coxwell, M. D." and Lite "Selected and translated by C. Fillingham, Cockswell, MD.", twice each. The frontmost app didn't change.
- All engine hosts ran from the bundle and mapped only its libraries, with `default.metallib` loaded. Metal doing the work was shown earlier on this branch with `sample` (`ggml_metal`, `waitUntilCompleted`, AGX), before the last commits, which touched no engine code.
- `--install` and its refusal of a stranger were tested earlier with a fake HOME. Not repeated on the final build, and the real `~/Applications` wasn't touched.

Not done: no app icon. `target/release` can hold stale llama.cpp dylibs for the bare binary until they're removed, though the bundle isn't affected.
