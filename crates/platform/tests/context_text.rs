//! The text-around-cursor window (spec §3 "Context"): up to 500 characters each side of the
//! cursor plus the selection, read from accessibility APIs that index in UTF-16 units.

use wf_platform::context::{CONTEXT_CHARS, MAX_SELECTION_CHARS, TextAround, Utf16Window};

#[test]
fn keeps_the_500_characters_nearest_the_cursor_on_each_side() {
    let before = format!("{}{}", "x".repeat(100), "b".repeat(CONTEXT_CHARS));
    let after = format!("{}{}", "a".repeat(CONTEXT_CHARS), "y".repeat(100));

    let around = TextAround::new(&before, "selected", &after);

    assert_eq!(around.before, "b".repeat(500));
    assert_eq!(around.selection, "selected");
    assert_eq!(around.after, "a".repeat(500));
}

#[test]
fn a_select_all_in_a_long_document_is_capped() {
    let around = TextAround::new("", &"s".repeat(10_000), "");

    assert_eq!(around.selection, "s".repeat(MAX_SELECTION_CHARS));
}

#[test]
fn fetches_enough_utf16_units_for_500_characters_each_side_clamped_to_the_text() {
    // 1000 units cover 500 characters even if every one is a surrogate pair.
    let middle = Utf16Window::around(5_000..5_010, 20_000);
    assert_eq!(middle.before, 4_000..5_000);
    assert_eq!(middle.selection, 5_000..5_010);
    assert_eq!(middle.after, 5_010..6_010);

    let near_edges = Utf16Window::around(300..300, 700);
    assert_eq!(near_edges.before, 0..300);
    assert_eq!(near_edges.selection, 300..300);
    assert_eq!(near_edges.after, 300..700);

    let huge_selection = Utf16Window::around(0..50_000, 50_000);
    assert_eq!(huge_selection.selection, 0..4_000);
    assert_eq!(huge_selection.after, 50_000..50_000);
}

#[test]
fn utf16_slices_cut_through_a_surrogate_pair_drop_the_broken_half() {
    let units = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let before = units("🎙️ hi 👋");
    let after = units("👋 there 🎉");

    // The window edges fall inside the first emoji of `before` and the last one of `after`.
    let around = TextAround::from_utf16(&before[1..], &units("ok"), &after[..after.len() - 1]);

    assert_eq!(around.before, "\u{fe0f} hi 👋");
    assert_eq!(around.selection, "ok");
    assert_eq!(around.after, "👋 there ");
}
