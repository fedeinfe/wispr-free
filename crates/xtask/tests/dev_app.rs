//! The parts of `cargo xtask shots` and `cargo xtask footprint` that decide things. The bundle
//! itself runs in `dev_bundle.rs`.

use xtask::dev_app::{Footprint, footprint_in, no_focus_taken, pill_file, verdict};

const SUMMARY: &str = "Process:         wispr-free [123]\n\
    Physical footprint:         18.4M\n\
    Physical footprint (peak):  1.2G\n\
    Idle exit:                  untracked\n";

#[test]
fn the_footprint_comes_out_of_what_vmmap_prints() {
    assert_eq!(
        footprint_in(SUMMARY),
        Some(Footprint {
            now_mb: 18.4,
            peak_mb: 1228.8,
        })
    );
    assert_eq!(footprint_in("Physical footprint: 18.4M\n"), None);
    assert_eq!(
        footprint_in("Physical footprint: lots\nPhysical footprint (peak): 1M\n"),
        None
    );
}

#[test]
fn the_report_states_any_gap_to_the_goal() {
    let under = Footprint {
        now_mb: 18.4,
        peak_mb: 19.0,
    };
    assert_eq!(
        verdict(under),
        "idle footprint 18.4 MB, peak 19.0 MB: under the 25 MB goal"
    );
    let over = Footprint {
        now_mb: 26.5,
        peak_mb: 30.0,
    };
    assert_eq!(
        verdict(over),
        "idle footprint 26.5 MB, peak 30.0 MB: 1.5 MB over the 25 MB goal"
    );
}

#[test]
fn the_pill_file_is_one_the_app_reads() {
    assert_eq!(pill_file("left"), "edge = \"left\"\nsounds = false\n");
}

#[test]
fn a_run_that_was_activated_fails() {
    assert!(no_focus_taken("shell started\nshell closed\n").is_ok());
    assert!(no_focus_taken("shell started\ntook the focus\nshell closed\n").is_err());
}
