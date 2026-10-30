use std::process::Command;

#[test]
fn version_flag_prints_version_and_exits_without_starting_the_ui() {
    let output = Command::new(env!("CARGO_BIN_EXE_wispr-free"))
        .arg("--version")
        .output()
        .expect("binary runs");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "wispr-free 0.1.0"
    );
}
