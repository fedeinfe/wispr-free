# 12 Settings
Status: ready-for-agent
Blocked by: 11

Spec: Settings, Settled mismatches, Bridge (long jobs move to `Shell`), Phase 2, Memory. Behaviour: `research/surface.md`.

All 8 pages in `Form(.grouped)`, every change commits at once. Shortcut capture with `record_shortcut` and `stop_recording_shortcut`. Models flows with `catalog`, `hardware`, `download`, `cancel_download`, `move_folder`, `verify`, owned by `Shell` so they outlive the window. `NSOpenPanel` replaces `rfd`, then re-check the framework list. Data: `export_all`, `delete_all`, `forget_history`, with `confirmationDialog`. The notice banner becomes a top `Section`.

Remove App language and Theme, both control and field, and an old `theme` key in `settings.toml` is ignored. Tier as a UniFFI enum. One language list and one tier list from Rust, and the pill's Language menu shows all 15.

Port `settings_probe persist-*`. Run the Settings design review.

Done when: the ported tests and `ax-check` pass on every page, persistence is proven to reach disk, footprint after use is recorded, and shots of every page in both appearances are read by you.

## Comments
