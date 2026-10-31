# Implementation rules for the apple-ui tickets

Read this, `spec.md` and your ticket before you start. Tickets 07 to 16 implement the spec's phases. They're pointers into the spec, which stays the source of truth.

## Task graph

```text
07 engine host → 08 wf-shell → 09 bridge and dev bundle → 10 pill and menu bar → 11 window shell and test rig
11 → 12 Settings → 13 Onboarding ┐
11 → 14 Meetings                 ├→ 15 cutover → 16 hand checks (ready-for-human)
```

## Hard limits

- **Shared Mac.** Run every build and test under `nice -n 10` with `CARGO_BUILD_JOBS=6`, and `swift build -j 6`. Wrap any test you're unsure of in `/Users/fedeinfe/Documents/Repo/prodotti/wispr-free-notes/probes/13-onboarding/capped.py`. Stop a retry loop as soon as memory climbs. Fake models with `File::set_len`, never by writing their bytes.
- **Never take the focus.** No synthetic keystrokes or clicks into the user's apps, no `osascript ... activate`, no `open` on a bundle. Nothing may activate over the app the user is in. A test window must be ordered in without activating.
- **Leave the installed app alone.** `~/Applications/Wispr Free.app` is the user's daily app and it's running. Never quit, replace, re-sign or install over it, and never run `cargo xtask install` or the like. Phase 0's "the installed app dictates" and every other install are ticket 16's, for the user.
- **Leave the user's data alone.** `ProjectDirs::from("", "", "wispr-free")` is hard-coded in `wf-core`, `wf-models` and `wf-history`, so a process with a different bundle id still lands in the real data folder. Tests use temp dirs. Anything you launch uses the isolated data root from ticket 09. Never read or write `~/Library/Application Support/wispr-free` except to read model files.
- **Real models live on the external SSD.** `wispr-free-notes/models` is a symlink to `/Volumes/Extreme SSD/AI-models/wispr-free`, and the app's own folder is `/Volumes/Extreme SSD/test`. If `/Volumes/Extreme SSD` isn't mounted, real-model tests and evals can't run: say so in your comment. Never copy a model back onto the internal disk.
- **Under test, the dev app takes no keys and no microphone.** It starts the listener and capture only when the user runs it by hand, so it can't race the real app.
- **Clean room (ADR-0001).** No Wispr asset or string enters the repo. **No network in the app (ADR-0004).**
- **The design skill is never committed.** It lives at `/Users/fedeinfe/Documents/Repo/prodotti/wispr-free/.claude/skills/apple-design`, outside your worktree on purpose, and has no licence. Use it in review mode only, loading just the references the spec's Design review table names for your surface. Don't paste its text into the repo. Apply what it finds where it agrees with the spec. The spec wins any disagreement, and the finding goes in your ticket comment.

## How to work

- Follow surrounding code: comment density, naming, idiom. Commit subjects are plain imperative sentences, like `git log` shows.
- No business rule in a Swift view. If a label needs logic, it comes from Rust.
- Don't add features. An idea goes in your ticket comment.
- Keep probes and scratch output in `/Users/fedeinfe/Documents/Repo/prodotti/wispr-free-notes/apple-ui/<ticket>/`, not the repo.
- Keep the Slint app building and its tests green until ticket 15 deletes it.
- If the spec is wrong or can't be met, say so in your ticket comment and take the smallest decision that keeps the goals. Don't quietly build something else.

## Finishing a ticket

Append a dated comment under `## Comments` in your ticket: what was done, what you verified and how, what you couldn't verify, any decision the spec didn't cover. Don't change `Status:`. Merge the integration branch `apple-ui` into your branch, rerun the tests, and report your branch name and head.
