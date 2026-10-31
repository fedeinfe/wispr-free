//! What `cargo xtask ax-check` decides from a walk. The walk itself runs on the real dev bundle.

use xtask::ax_check::{CONTRACTS, Contract, Node, Walk, Window, broken};

const APP: i32 = 77;
const OTHER_APP: i32 = 12;

const CONTRACT: Contract = Contract {
    item: "Meetings…",
    window: "Meetings",
    title: "Meetings",
    labels: &["Summary", "Notes"],
};

fn node(identifier: &str) -> Node {
    Node {
        role: "AXButton".to_owned(),
        identifier: identifier.to_owned(),
        title: String::new(),
        enabled: true,
    }
}

fn window(title: &str, labels: &[&str]) -> Window {
    Window {
        identifier: "Meetings".to_owned(),
        title: title.to_owned(),
        nodes: labels.iter().map(|label| node(label)).collect(),
    }
}

fn walk(windows: Vec<Window>) -> Walk {
    Walk {
        trusted: true,
        front_pid: OTHER_APP,
        windows,
    }
}

#[test]
fn a_window_with_its_title_and_labels_holds() {
    let walked = walk(vec![window("Meetings", &["Notes", "Extra", "Summary"])]);

    assert_eq!(broken(&CONTRACT, &walked, APP), Vec::<String>::new());
}

#[test]
fn a_missing_window_is_reported() {
    assert_eq!(
        broken(&CONTRACT, &walk(Vec::new()), APP),
        ["no window Meetings"]
    );
}

#[test]
fn a_missing_label_and_a_wrong_title_are_each_reported() {
    let walked = walk(vec![window("wispr-free meetings", &["Summary"])]);

    assert_eq!(
        broken(&CONTRACT, &walked, APP),
        [
            "Meetings is titled \"wispr-free meetings\", not \"Meetings\"",
            "Meetings has no Notes",
        ]
    );
}

#[test]
fn the_app_in_front_is_a_failure_whatever_else_holds() {
    let mut walked = walk(vec![window("Meetings", &["Notes", "Summary"])]);
    walked.front_pid = APP;

    assert_eq!(broken(&CONTRACT, &walked, APP), ["the app is in front"]);
}

#[test]
fn the_pills_panel_is_no_window_of_the_menus() {
    let mut pill = window("", &[]);
    pill.identifier = String::new();
    let walked = walk(vec![pill, window("Meetings", &[])]);

    let opened: Vec<_> = walked.opened().map(|window| &window.title).collect();

    assert_eq!(opened, ["Meetings"]);
}

#[test]
fn a_walk_comes_out_of_what_ax_walk_prints() {
    let printed = r#"{"trusted":true,"front_pid":12,"windows":[{"identifier":"Settings",
        "title":"General","nodes":[{"role":"AXRow","identifier":"General","title":""}]}]}"#;

    let walked: Walk = serde_json::from_str(printed).unwrap();

    assert_eq!(walked.windows[0].nodes[0].role, "AXRow");
    assert_eq!(walked.front_pid, 12);
}

#[test]
fn the_contract_keeps_the_eight_pages_and_every_window() {
    let windows: Vec<_> = CONTRACTS.iter().map(|contract| contract.window).collect();

    assert_eq!(windows, ["Settings", "Meetings", "Set up Wispr Free"]);
    assert_eq!(CONTRACTS[0].labels.len(), 8);
}
