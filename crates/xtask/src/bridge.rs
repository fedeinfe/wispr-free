//! `cargo xtask bridge` makes what the Swift package needs from Rust: `wf-shell` as a static
//! library with no engine in it, the Swift bindings, the modulemap and `WfBridge.xcframework`.
//! `swift build` and `swift test` work from there. Everything it makes is git-ignored.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, ensure};
use uniffi::SwiftBindingsOptions;

use crate::bundle::{DEV, cargo_tree, engines_in, target_dir, tool, workspace_root};

const SHELL: &str = "wf-shell";
/// What rustc and the C compilers aim at unless told, which would be this Mac's own version.
const DEPLOYMENT_TARGET: &str = "MACOSX_DEPLOYMENT_TARGET";
const LIBRARY: &str = "libwf_shell.a";
/// Each UniFFI namespace, with the crate that sets it up.
const NAMESPACES: [&str; 2] = ["wfshell", "wfsettings"];
const LINK_NOTE: &str = "native-static-libs: ";
const ENGINE_MEMBERS: [&str; 3] = ["llama", "whisper", "sherpa"];
/// Linked into every program anyway.
const IMPLICIT_LIBRARIES: [&str; 4] = ["System", "c", "m", "objc"];

/// A module per namespace. The generated modulemap lists a single one, and `canImport` then drops
/// the other's import in silence.
pub fn modulemap() -> String {
    NAMESPACES
        .iter()
        .map(|name| format!("module {name}FFI {{ header \"{name}FFI.h\" export * }}\n"))
        .collect()
}

/// What rustc says a static library needs, as printed by `--print native-static-libs`.
pub fn link_list(messages: &str) -> Option<Vec<String>> {
    let line = messages
        .lines()
        .find_map(|line| Some(line.split_once(LINK_NOTE)?.1))?;
    let mut words = line.split_whitespace();
    let mut needed = Vec::new();
    while let Some(word) = words.next() {
        match word.strip_prefix("-l") {
            Some(library) if !IMPLICIT_LIBRARIES.contains(&library) => {
                needed.push(library.to_owned());
            }
            Some(_) => {}
            None if word == "-framework" => needed.extend(words.next().map(str::to_owned)),
            None => needed.push(word.to_owned()),
        }
    }
    needed.sort();
    needed.dedup();
    Some(needed)
}

/// The members of an archive, as `ar t` lists them, that come from an engine. `cargo tree` can't
/// see this: `cargo build --workspace` unifies features and leaves a library of the same name
/// with every engine in it. `nm` can't either, as it fails on rustc's objects.
pub fn engine_members(members: &str) -> Vec<&str> {
    members
        .lines()
        .filter(|member| ENGINE_MEMBERS.iter().any(|name| member.contains(name)))
        .collect()
}

/// What `Package.swift` doesn't list of `needed`.
pub fn unlisted(needed: &[String], package: &str) -> Vec<String> {
    needed
        .iter()
        .filter(|name| !package.contains(&format!("\"{name}\"")))
        .cloned()
        .collect()
}

pub fn apple_dir() -> PathBuf {
    workspace_root().join("apple")
}

pub fn run() -> anyhow::Result<()> {
    ensure!(cfg!(target_os = "macos"), "the bridge is macOS only");
    build()?;
    println!("made {}", xcframework_path().display());
    Ok(())
}

fn xcframework_path() -> PathBuf {
    apple_dir().join("WfBridge.xcframework")
}

/// Builds the library, then makes everything the Swift package reads from it.
pub fn build() -> anyhow::Result<()> {
    let library = build_library()?;
    let work = target_dir().join("wf-bridge");
    if work.exists() {
        fs::remove_dir_all(&work)?;
    }
    let (generated, headers) = (work.join("generated"), work.join("headers"));
    fs::create_dir_all(&headers)?;
    generate(&library, &generated)?;
    let sources = apple_dir().join("Sources/WfCore");
    if sources.exists() {
        fs::remove_dir_all(&sources)?;
    }
    fs::create_dir_all(&sources)?;
    for entry in fs::read_dir(&generated)? {
        let path = entry?.path();
        let folder = match path.extension().and_then(|extension| extension.to_str()) {
            Some("swift") => &sources,
            Some("h") => &headers,
            _ => continue,
        };
        fs::copy(
            &path,
            folder.join(path.file_name().context("a generated file")?),
        )?;
    }
    fs::write(headers.join("module.modulemap"), modulemap())?;
    make_xcframework(&library, &headers)
}

/// On its own, or cargo would hand it the features of everything else and with them every engine.
fn build_library() -> anyhow::Result<PathBuf> {
    let (_, messages) = tool("cargo")
        .args([
            "rustc",
            "--release",
            "--locked",
            "--package",
            SHELL,
            "--lib",
        ])
        .args(["--", "--print", "native-static-libs"])
        .env(DEPLOYMENT_TARGET, DEV.minimum_macos)
        .current_dir(workspace_root())
        .outputs()?;
    let tree = cargo_tree(SHELL)?;
    let engines = engines_in(&tree);
    ensure!(engines.is_empty(), "{SHELL} links {engines:?}");

    let needed = link_list(&messages).context("rustc didn't print the link list")?;
    let package = fs::read_to_string(apple_dir().join("Package.swift"))?;
    let missing = unlisted(&needed, &package);
    ensure!(missing.is_empty(), "Package.swift doesn't list {missing:?}");
    let library = target_dir().join("release").join(LIBRARY);
    let members = tool("ar").arg("t").arg(&library).text()?;
    let linked = engine_members(&members);
    ensure!(
        linked.is_empty(),
        "{SHELL} holds {} of an engine",
        linked.len()
    );
    Ok(library)
}

/// Works from the library's own metadata. It has to run inside the workspace: it calls `cargo
/// metadata` to find the crates.
fn generate(library: &Path, out: &Path) -> anyhow::Result<()> {
    std::env::set_current_dir(workspace_root())?;
    uniffi::generate_swift_bindings(SwiftBindingsOptions {
        generate_swift_sources: true,
        generate_headers: true,
        generate_modulemap: false,
        source: library.to_path_buf().try_into()?,
        out_dir: out.to_path_buf().try_into()?,
        xcframework: false,
        module_name: None,
        modulemap_filename: None,
        metadata_no_deps: false,
        link_frameworks: Vec::new(),
        config: None,
    })
}

fn make_xcframework(library: &Path, headers: &Path) -> anyhow::Result<()> {
    let output = xcframework_path();
    if output.exists() {
        fs::remove_dir_all(&output)?;
    }
    tool("xcodebuild")
        .arg("-create-xcframework")
        .arg("-library")
        .arg(library)
        .arg("-headers")
        .arg(headers)
        .arg("-output")
        .arg(&output)
        .succeed()
}
