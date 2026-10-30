# 10 Local passes + history
Status: ready-for-agent
Blocked by: 04, 08

Snippets, dictionary replacements, match surrounding casing/spacing, chat trailing period, press-enter command. SQLite history with retention policy; paste-last and copy-last.

Done when: each pass has unit tests and history retention 'never store' writes nothing to disk.

## Comments

**2026-10-30 · implementer (branch `t10-local-passes-history`)**

Done:
- `wf_pipeline::passes`: one pure function per pass, plus `local_passes` to run them for a dictation. Snippets (a dictation that is only a trigger becomes the expansion verbatim; inline triggers expand on word boundaries, longest first, never rescanned), dictionary replacements, casing and spacing against the text around the cursor with no doubled punctuation, short chat messages dropping the period, and the opt-in "press enter" command ("premi invio" too).
- Styles, which undo what rule cleanup always adds: Casual (no final period, no comma before a conjunction), Very casual (also lowercase), Excited (final `!`), and Email's line break after the greeting. `app_category` picks the category from the app id, and from the tab title in browsers. Each is idempotent, so it is safe after LLM cleanup.
- The passes run in a different order from the spec's list: style and casing go before the dictionary and snippets, otherwise Very casual would lowercase "GitHub" or a stored expansion. `local_passes` documents this.
- New crate `wf-history` (SQLite through `rusqlite`, ADR-0003): raw and final text, app, duration and stage timings. Retention keep / delete after 24 h / never store. `last_text` serves paste last and copy last, `recent` the pill menu, `clear` "Delete all data". The file is `0600` on Unix.
- It is a separate crate, not part of `wf-core` as ticket 01 planned: bundled SQLite in `wf-core` broke the `wf-platform` cross-check for Windows and Linux. That was measured, and both cross-checks pass after the move.

Never store: under this policy there is no connection and no file, so nothing can be written. Tests against a real temp directory show that 200 dictations leave it empty and the data folder is never created, that switching to never store erases an existing store and sidecars while leaving neighbouring files alone, and that later dictations change no file's size or modification time. Deleted text was searched for in the raw file bytes. Three deliberate breakages (`record` ignoring Never, Never leaving the old store, `secure_delete` off) each made tests fail. Paste last still works under never store from RAM, for the session only.

Decided where the spec is silent:
- Choosing never store erases past history immediately, so Settings should warn first.
- Very casual also drops the final period, and Excited changes only the final period.
- "Short" chat means one sentence, one line, at most 12 words.
- The newest text is kept in RAM under every retention, because spec §3's no-text-field fallback needs it.

Since this is a sub-agent run, the seams were chosen without asking: the public pass functions plus `local_passes` after real `rule_cleanup` output, and `History` against a real directory.

Not done:
- **The Enter key is never pressed.** `local_passes` only reports `press_enter: true`; `wf-platform` can send paste chords only. Sending Enter needs a keystroke on all three OS backends.
- Nothing is wired: `crates/app` calls no pass and no history, and no UI exists for paste last, copy last or Recent transcripts. The `record` / `last_text` / `prune` calls are written up in the notes.
- `Style`, `Replacement`, `Snippet` and `HistoryRetention` are defined here with the same names and fields as ticket 12's `wf_core::settings`, which is not on `v1` yet, and ticket 09 has its own `Style`. They need unifying once those land.

Needs manual verification:
- `wf-history` has only compiled and run on macOS: cross-checks can't build SQLite's C code, so Windows and Linux are untested, including erasing the store on Windows.
- Casing and spacing against real `TextAround` from live apps. Ticket 04's text reads are themselves unverified on real text fields.
- `app_category`'s ids and tab titles are from memory, so check them against real apps, especially Windows and Linux ids.
- Mid-sentence casing lowercases proper nouns not in the dictionary ("Marco left" after "and then " → "marco left"). Judge whether that is acceptable with real use.

