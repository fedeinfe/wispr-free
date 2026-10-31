# Map: Apple-style UI for Wispr Free

Label: wayfinder:map

## Destination

A spec ready to implement (`.scratch/apple-ui/spec.md`) for rewriting Wispr Free's user interface in a native Apple style, with the toolkit chosen, the design rules fixed and the work cut into phases. The rewrite itself happens after this map.

## Notes

- Domain: macOS app, Rust core, UI toolkit choice. Read `GLOSSARY.md` and ADR-0001, 0002 and 0005.
- The user delegated every decision on this map ("risponditi da solo fino ad avere la spec"), so the agent answered its own grilling and approved its own prototype. Each answer below is the agent's call, open to the user's veto.
- Findings live in `.scratch/apple-ui/research/` and the prototype in `.scratch/apple-ui/prototype/`, not on a pushed branch: nothing was asked to be pushed.
- Standing limits: repo clean-room rule (ADR-0001), no network by default (ADR-0004), no synthetic keystrokes into the user's apps while testing, `nice -n 10` and `CARGO_BUILD_JOBS=6`.

## Decisions so far

- [Scope of the rewrite](issues/01-scope.md): spec first, all five parts, native over all-Rust, macOS 26+, system-app look, no new features.
- [Which design skill to use](issues/02-design-skills.md): `dickwu/apple-design-skill`, pinned, reviewer only, never committed.
- [Which toolkit and which bridge to Rust](issues/03-toolkit.md): SwiftUI in AppKit windows, UniFFI, SwiftPM under `xtask`, Rust helper for engines.
- [What the UI needs from the core](issues/04-surface.md): five surfaces, logic nearly toolkit-free, 142 window tests to rewrite, 8 mismatches.
- [Prove the stack with a real pill](issues/05-prototype.md): works end to end, 18 MB idle and 41 MB after a window, pill drops glass for a solid capsule.
- [Write the spec](issues/06-spec.md): `spec.md` and proposed ADR-0006.
- Implementation: the spec's phases became tickets 07 to 16, on branch `apple-ui`. Graph and rules: [implementation.md](implementation.md).

## Not yet specified

Nothing left: the spec answers what was here (tests, pill event loop, bundling). The hand checks in the spec are verification, not open decisions.

## Out of scope

- Windows and Linux UI: parked by the user, and SwiftUI can't reach them.
- New features: the rewrite changes look and interaction, not what the app does.
- Translating the interface: it stays in English.
