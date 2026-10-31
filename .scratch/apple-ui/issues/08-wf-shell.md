# 08 Move toolkit-free logic into wf-shell
Status: ready-for-agent
Blocked by: 07

Spec: Crates, Phase 0, Testing. Inventory: `research/surface.md`, "Where the current app code goes".

New `wf-shell` crate, `staticlib` and `lib`, toolkit-free. It takes `onboarding/{steps,trial}.rs`, `settings_window/data.rs`, `prefs.rs`, `memory.rs`, `meetings_window/disk.rs`, and the logic in `values`, `rows`, `system`, `host` and `dictation.rs`, along with the `Desktop`, `Machine` and `MeetingActions` seams. No UniFFI yet. The Slint app becomes a thin client with no visible change, including the single-source language and tier lists only where that changes nothing a user sees.

Port the logic in the 142 window tests to `cargo test -p wf-shell` on the existing fakes, keeping today's labels as the contract, as far as the logic has moved. Whatever stays Slint-only stays in `crates/app` until ticket 15.

Done when: `cargo test --workspace` and clippy pass, `cargo xtask eval` is unchanged, `wf-shell` has no `slint` in its dependency tree, and `cargo tree -p wf-shell` shows no engine.

Keep `wf-shell` and `wf-engine-host` in separate cargo commands (see ticket 07's comment), and extend `the_app_depends_on_no_engine` and the check in `assemble` to cover `wf-shell`.

## Comments

### 2026-10-31, implementation

Done: new crate `crates/shell` (`wf-shell`, `staticlib` and `lib`), with no toolkit and no engine. It holds `onboarding/{steps,trial}`, `settings/{data,rows,values,system}`, `prefs`, `memory` with the global allocator, `meetings/{disk,rows}`, `flow` (starting the flow, `KeyWatch`, `rank_first`), and the seams `Desktop`, `Machine` and `MeetingActions`. The window controllers' logic moved too: `settings::Pages`, `onboarding::Onboarding` with `OnboardingHost`, `meetings::Meetings`, and `windows::WindowHost` for the open-and-raise rule. The ticket doesn't name those `mod.rs` files, but the seams are only called from there, so the Slint app couldn't be thin without them. The Slint controllers now only bind: 355, 155 and 214 lines against 1312, 639 and 754.

The shape: each controller says what to show through a `View`, a closure taking one `Shown` value, in the same order and at the same moments as the old `set_*` calls. That's why a shown-state snapshot wasn't used: it would have needed diffing and special cases for the two-way `Prefs` values. `lists::Rows` keeps row-level changes row-level. `clock::{Clock, Timer}` replaces `slint::Timer`, and the app supplies `SlintClock`. Replies the old code read back from the window (`recommended_tier`, `models_folder_missing`, the selected meeting, the search text) are now held in Rust.

Tests:
- **All 142 window tests still run in `crates/app`, with not one line of their code changed.** They're the proof of "no visible change", and they cover the binding, which nothing in `wf-shell` can. Ticket 15 deletes them with the app. `tests/settings_values.rs` (11 tests, not among the 142) moved to `wf-shell`, as did the unit tests inside the moved files.
- **135 of the 142 are ported to `cargo test -p wf-shell`** on the existing fakes, with today's labels kept as the vocabulary of each harness's `click`:
  - Settings, 68 of 72: 64 in `tests/settings_pages.rs`, and `the_window_exists_only_while_open`, `opening_twice_brings_the_same_window_forward`, `closing_the_window_drops_it` and `the_window_can_be_opened_again_after_closing_with_what_was_saved` in `tests/window_host.rs`. That last one is now `the_window_can_be_opened_again_after_closing`: the "what was saved" half is covered by the restart tests in `settings_pages.rs`.
  - Onboarding, 38 of 38, in `tests/onboarding.rs`.
  - Meetings, 29 of 32, in `tests/meetings.rs`.
- **7 stay in `crates/app` only**, because what they test lives in the `.slint` files:
  - `a_choice_picked_from_a_dropdown_is_saved`: the popup.
  - `text_typed_into_a_field_is_saved` and `text_typed_into_a_replacement_row_is_saved`: the fields' accessible value.
  - `a_stepper_changes_a_number_within_its_limits`: the limits are in the page.
  - `the_summary_warns_it_can_be_wrong_and_offers_the_transcript`: static text.
  - `the_tabs_switch_panels`: the `tab` property.
  - `the_host_keeps_nothing_while_closed_and_one_window_while_open`: `MeetingsBoard` stays in the app, and its open-and-raise half is covered by `window_host.rs`.
- **Ported in part**, with the rest still only in `crates/app`:
  - Text and enabled states from the `.slint` are out of `no_input_device_is_its_own_state_and_plugging_one_in_clears_it`, `an_unplugged_models_folder_reads_as_in_settings_and_comes_back` and `a_later_launch_with_no_microphone_stays_on_that_step`.
  - `a_re_check_with_nothing_new_rebuilds_nothing` covers the rows. The sidebar guard is in the binding.
  - `delete_asks_first_and_then_it_is_gone` is `delete_removes_the_meeting_and_clears_the_selection`: the question is in the page.
  - `a_summary_time_on_a_real_line_jumps_and_a_wrong_one_does_not`, `a_question_streams_and_ends_with_times_that_jump` and `saved_answers_are_checked_again_when_the_meeting_opens` check which row a time points at. The click that jumps is the page's.
  - `the_pipeline_offers_a_download_for_each_missing_model` and the two tests beside it restate the page's filter in the harness, so they test the rows, not the filter. The harness's `click` also skips the VAD switch while it's forced, as the page does.

Verified, on the tree merged with `apple-ui` at fa5100c:
- `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked --no-fail-fast`: 1234 passed, 0 failed, 76 ignored. Log in `wispr-free-notes/apple-ui/08-wf-shell/`.
- `cargo tree -p wf-shell` has no `slint` and no engine: `the_app_depends_on_no_engine` now covers both packages, and a new `the_shell_depends_on_no_toolkit` covers the toolkit. `assemble` now fails on an engine in either tree, which `otool` couldn't see when statically linked. `cargo build -p wf-shell` makes a `libwf_shell.a` with no llama, whisper or sherpa symbol.
- `cargo xtask eval` with Qwen3-1.7B from the SSD: 83/89 = 93.3%. Failures, per-language scores and fallbacks match ticket 07's report, as expected since nothing the eval runs changed.
- `cargo xtask bundle`, signed with Wispr Free Local, not installed: `codesign --verify --strict` passes, the app binary has no rpath library, 6 dylibs in `Frameworks`, the helper answers `--version`, and mimalloc is still in the app binary after its move.
- The ports came after the moves, so they didn't fail first in the usual way. To check they can fail, I broke six rules one at a time (launch at login rollback, the idle restart of a recording, the pill's silence while the window is open, the flush on closing Meetings, the duplicate live line, the redirect). Each failed exactly the test that covers it.

Couldn't verify:
- I never launched the Slint app, so "no visible change" rests on the 142 tests and the bundle building. The installed app dictating stays ticket 16's.
- `real_dictation`, `real_meeting` and `engine_host`'s real runs weren't repeated. The ignored tests stayed ignored, and `wf-dictation` and `wf-engine-host` didn't change.
- The real `SystemDesktop`, `SystemMachine` and `KeyWatch` listeners and the `rfd` picker moved or were rewired with compile coverage only, since a test mustn't take keys or show a dialog. `settings_probe persist-*` compiles but wasn't run.
- Linux and Windows CI.

Decisions the spec didn't cover:
- `PillActions`, `deliver_to` and the app's `MeetingsBoard` stay in `crates/app`, as they're tied to `PillController` and the window types. `pill.rs` isn't in this ticket, so its logic is ticket 10's.
- Single source: `settings::values::TIERS` now serves both Settings and the pill's menu. The language lists stay separate, because merging them would change what the user sees (3 entries against 15, "Italiano" against "Italian"). Ticket 12 does that, as the spec says. The pill's four tier labels stay in `pill.rs` for the same ticket.
- The picker stays with the toolkit: `SystemDesktop` takes it as `PickFolder`, and `crates/app/src/folder_picker.rs` keeps `rfd` until `NSOpenPanel`. `SystemMachine` takes the tray, closing Settings and quitting as closures in the same way.
- Hosts return `Result<(), String>`, not `PlatformError`. Every caller only prints it.
- `WindowHost` reaches itself through a weak reference where Settings and Meetings held a strong one. Nothing visible changes, and a dropped host no longer leaks an open window.
- `tests/support/memory_cap.rs` is copied, not shared, so it survives ticket 15.

For ticket 09: `cargo build --workspace` still unifies features, so it makes a `libwf_shell.a` with every engine in it. Only `cargo build -p wf-shell` gives the real one, as ticket 07 said. The debug archive is 326 MB. `Pages` is shaped like the windows, so `Shell` needs to decide how much of it to expose: the spec's Swift pages save `Settings` directly and never send `SettingsValues`, and its long jobs still die with `Pages`. `apply_values` is where the tier rules live today.
