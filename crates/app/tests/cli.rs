use std::process::{Command, Stdio};

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

fn refused(arg: &str) -> bool {
    !Command::new(env!("CARGO_BIN_EXE_wispr-free"))
        .arg(arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("binary runs")
        .success()
}

#[test]
fn the_app_refuses_to_be_an_engine_host() {
    assert!(refused("--engine-host"));
}

#[test]
fn the_app_refuses_a_clip_run() {
    assert!(refused("--probe-clip=clip.wav"));
}
