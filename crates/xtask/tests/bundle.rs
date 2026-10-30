//! The parts of `cargo xtask bundle` that decide things. The real build runs by hand: it signs
//! files and takes minutes.

use std::path::Path;

use xtask::bundle::{
    Signer, codesign_args, foreign_libraries, info_plist, llama_libraries, rpath_libraries,
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
fn the_plist_says_what_macos_asks_for() {
    let plist = info_plist("1.2.3");

    for key in [
        "NSMicrophoneUsageDescription",
        "NSAudioCaptureUsageDescription",
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
