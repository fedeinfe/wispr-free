use std::process::{Command, Stdio};

#[test]
fn version_flag_prints_version_and_exits() {
    let output = Command::new(env!("CARGO_BIN_EXE_wf-engine-host"))
        .arg("--version")
        .output()
        .expect("binary runs");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "wf-engine-host 0.1.0"
    );
}

#[test]
fn a_host_ends_when_nobody_is_left_to_call_it() {
    let status = Command::new(env!("CARGO_BIN_EXE_wf-engine-host"))
        .arg("--engine-host")
        .stdin(Stdio::null())
        .status()
        .expect("binary runs");

    assert!(status.success());
}

#[test]
fn without_a_mode_it_does_nothing() {
    let output = Command::new(env!("CARGO_BIN_EXE_wf-engine-host"))
        .stdin(Stdio::null())
        .output()
        .expect("binary runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
