//! The parts of `cargo xtask bundle` that decide things. The real build runs by hand: it signs
//! files and takes minutes.

use std::path::{Path, PathBuf};
use std::process::Command;

use wf_dictation::remote::engine_host_beside;
use xtask::bundle::{
    Signer, built_binary, codesign_args, engines_in, foreign_libraries, helper_path, iconset_files,
    info_plist, llama_libraries, rpath_libraries, signing_order,
};

const OTOOL: &str = "\
/tmp/libllama-common.0.6.0.dylib:
\t@rpath/libllama-common.0.dylib (compatibility version 0.0.0, current version 0.0.0)
\t@rpath/libllama.0.dylib (compatibility version 0.0.0, current version 0.0.0)
\t/opt/homebrew/opt/openssl@4/lib/libssl.4.dylib (compatibility version 4.0.0, current version 4.0.0)
\t@rpath/libggml-metal.0.dylib (compatibility version 0.0.0, current version 0.26.0)
\t/System/Library/Frameworks/Metal.framework/Versions/A/Metal (compatibility version 1.0.0, current version 383.1.0)
\t/usr/lib/libc++.1.dylib (compatibility version 1.0.0, current version 2200.32.0)
";

#[test]
fn the_libraries_to_carry_are_the_ones_found_through_the_rpath() {
    assert_eq!(
        rpath_libraries(OTOOL),
        [
            "libllama-common.0.dylib",
            "libllama.0.dylib",
            "libggml-metal.0.dylib"
        ]
    );
}

#[test]
fn a_library_outside_the_system_and_the_rpath_is_foreign() {
    assert_eq!(
        foreign_libraries(OTOOL),
        ["/opt/homebrew/opt/openssl@4/lib/libssl.4.dylib"]
    );
}

#[test]
fn the_llama_libraries_come_from_the_build_that_made_the_app() {
    let messages = r#"{"reason":"compiler-message"}
{"reason":"build-script-executed","package_id":"registry+https://github.com/rust-lang/crates.io-index#whisper-rs-sys@0.14.0","out_dir":"/t/whisper"}
{"reason":"build-script-executed","package_id":"registry+https://github.com/rust-lang/crates.io-index#llama-cpp-sys-2@0.1.160","out_dir":"/t/release/build/llama-cpp-sys-2-abc/out"}
{"reason":"build-finished","success":true}"#;

    assert_eq!(
        llama_libraries(messages).as_deref(),
        Some(Path::new("/t/release/build/llama-cpp-sys-2-abc/out/lib"))
    );
    assert_eq!(llama_libraries(r#"{"reason":"build-finished"}"#), None);
}

#[test]
fn a_binary_comes_from_the_messages_of_its_own_build() {
    let messages = r#"{"reason":"compiler-artifact","target":{"name":"wf_core"},"executable":null}
{"reason":"compiler-artifact","target":{"name":"wf-engine-host"},"executable":"/t/release/wf-engine-host"}
{"reason":"build-finished","success":true}"#;

    assert_eq!(
        built_binary(messages, "wf-engine-host").as_deref(),
        Some(Path::new("/t/release/wf-engine-host"))
    );
    assert_eq!(built_binary(messages, "wispr-free"), None);
}

#[test]
fn the_helper_goes_where_the_app_looks_for_it() {
    let app = Path::new("/t/Wispr Free.app");

    assert_eq!(
        helper_path(app),
        Path::new("/t/Wispr Free.app/Contents/Helpers/wf-engine-host")
    );
    assert_eq!(
        engine_host_beside(&app.join("Contents/MacOS/wispr-free")),
        helper_path(app)
    );
}

#[test]
fn signing_goes_inside_out_and_the_helper_is_named() {
    let app = Path::new("/t/Wispr Free.app");
    let libraries = [
        app.join("Contents/Frameworks/libllama.0.dylib"),
        app.join("Contents/Frameworks/libggml.0.dylib"),
    ];

    let order = signing_order(app, &libraries);

    assert_eq!(
        order,
        [
            (libraries[0].clone(), None),
            (libraries[1].clone(), None),
            (
                PathBuf::from("/t/Wispr Free.app/Contents/Helpers/wf-engine-host"),
                Some("com.wispr-free.wf-engine-host")
            ),
            (app.to_path_buf(), Some("com.wispr-free.app")),
        ]
    );
}

/// What cargo resolves for `package` on its own, which is how `bundle` builds it.
fn tree_of(package: &str) -> String {
    let output = Command::new(env!("CARGO"))
        .args(["tree", "--locked", "--package", package])
        .args(["--edges", "normal", "--prefix", "none"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo runs");
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// ADR-0005.
#[test]
fn the_app_depends_on_no_engine() {
    for package in ["wispr-free", "wf-shell"] {
        let tree = tree_of(package);

        assert!(tree.contains("wf-dictation"), "{package}");
        assert_eq!(engines_in(&tree), Vec::<&str>::new(), "{package}");
    }
}

#[test]
fn the_shell_depends_on_no_toolkit() {
    assert!(!tree_of("wf-shell").contains("slint"));
}

#[test]
fn a_tree_with_an_engine_in_it_is_caught() {
    assert_eq!(
        engines_in("wf-pipeline v0.1.0\nwhisper-rs v0.16.0\n"),
        ["whisper-rs"]
    );
}

#[test]
fn the_plist_says_what_macos_asks_for() {
    let plist = info_plist("1.2.3");

    for key in [
        "NSMicrophoneUsageDescription",
        "NSAudioCaptureUsageDescription",
        "NSAppleEventsUsageDescription",
        "CFBundleIconFile",
        "LSUIElement",
    ] {
        assert!(plist.contains(&format!("<key>{key}</key>")), "{key}");
    }
    assert!(plist.contains("<string>com.wispr-free.app</string>"));
    assert!(plist.contains("<string>wispr-free</string>"));
    assert!(plist.contains("<string>1.2.3</string>"));
    assert!(!plist.to_lowercase().contains("cloud"));
}

#[test]
fn the_icon_set_has_every_size_macos_looks_for() {
    let files = iconset_files();

    assert_eq!(files.len(), 10);
    assert!(files.contains(&("icon_16x16.png".to_owned(), 16)));
    assert!(files.contains(&("icon_16x16@2x.png".to_owned(), 32)));
    assert!(files.contains(&("icon_512x512@2x.png".to_owned(), 1024)));
}

#[test]
fn the_app_has_no_dock_icon_until_the_setting_asks_for_one() {
    let plist = info_plist("1.0.0");
    let after_key = plist.split("<key>LSUIElement</key>").nth(1).unwrap();
    assert!(after_key.trim_start().starts_with("<true/>"));
}

#[test]
fn ad_hoc_signs_with_a_dash_and_a_local_identity_by_its_hash() {
    assert_eq!(codesign_args(&Signer::AdHoc), ["--sign", "-"]);
    let local = Signer::Local {
        keychain: "/k/signing.keychain-db".into(),
        hash: "ABCD".to_owned(),
    };
    assert_eq!(
        codesign_args(&local),
        ["--keychain", "/k/signing.keychain-db", "--sign", "ABCD"]
    );
}
