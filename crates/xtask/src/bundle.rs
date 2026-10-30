//! `cargo xtask bundle` builds `target/Wispr Free.app` for this Mac; `--install` also copies it to
//! `~/Applications`. `cargo xtask signing-identity` makes a local certificate, once, so macOS keeps
//! the app's permissions across rebuilds. Releases (signed for other people) are ticket 18's.
//!
//! macOS ties a permission to the code's designated requirement. For an ad-hoc signature that is
//! the hash of the code, so any rebuilt binary is a stranger. For a certificate it is the
//! identifier and the certificate, which a rebuild doesn't change.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, bail, ensure};

pub const APP: &str = "Wispr Free.app";
const BUNDLE_ID: &str = "com.wispr-free.app";
const EXECUTABLE: &str = "wispr-free";
const ICON: &str = "AppIcon";
const ICON_SOURCE: &str = "crates/app/ui/icons/app-icon.svg";
/// The oldest macOS the spec supports. Ticket 15's weak CoreAudio link is what makes it true.
const MINIMUM_MACOS: &str = "13.0";
const IDENTITY: &str = "Wispr Free Local";
const IDENTITY_DAYS: &str = "3650";
/// Outside the login keychain, so signing never brings up a password dialog, and outside `target`,
/// so `cargo clean` doesn't change the app's identity. Its password is empty: it holds this one
/// key, for this user's own builds.
const KEYCHAIN: &str = ".config/wispr-free-dev/signing.keychain-db";

/// The system's own, so nothing here needs Homebrew.
const OPENSSL: &str = "/usr/bin/openssl";
const LLAMA_PACKAGE: &str = "#llama-cpp-sys-2@";

/// How the app gets signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signer {
    AdHoc,
    Local { keychain: PathBuf, hash: String },
}

pub fn codesign_args(signer: &Signer) -> Vec<String> {
    match signer {
        Signer::AdHoc => vec!["--sign".to_owned(), "-".to_owned()],
        Signer::Local { keychain, hash } => vec![
            "--keychain".to_owned(),
            keychain.display().to_string(),
            "--sign".to_owned(),
            hash.clone(),
        ],
    }
}

pub fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>Wispr Free</string>
    <key>CFBundleExecutable</key>
    <string>{EXECUTABLE}</string>
    <key>CFBundleIconFile</key>
    <string>{ICON}</string>
    <key>CFBundleIdentifier</key>
    <string>{BUNDLE_ID}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Wispr Free</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>LSMinimumSystemVersion</key>
    <string>{MINIMUM_MACOS}</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSAppleEventsUsageDescription</key>
    <string>Wispr Free pauses this player while you dictate and resumes it afterwards, if you turned that on in Settings.</string>
    <key>NSAudioCaptureUsageDescription</key>
    <string>Wispr Free records the other side of a meeting, only after you start meeting notes. It stays on this Mac.</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>Wispr Free listens while you dictate and turns your speech into text. Your voice stays on this Mac.</string>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
</dict>
</plist>
"#
    )
}

/// The files of a macOS `.iconset`, with the pixels each one has.
pub fn iconset_files() -> Vec<(String, u32)> {
    [16, 32, 128, 256, 512]
        .into_iter()
        .flat_map(|points: u32| {
            [
                (format!("icon_{points}x{points}.png"), points),
                (format!("icon_{points}x{points}@2x.png"), points * 2),
            ]
        })
        .collect()
}

/// What `otool -L` reports as loaded through the rpath, by file name.
pub fn rpath_libraries(otool: &str) -> Vec<String> {
    linked(otool)
        .filter_map(|path| path.strip_prefix("@rpath/"))
        .map(str::to_owned)
        .collect()
}

/// What `otool -L` reports that neither macOS nor the bundle provides. It would tie the app to
/// this machine's state.
pub fn foreign_libraries(otool: &str) -> Vec<String> {
    linked(otool)
        .filter(|path| {
            !path.starts_with("@rpath/")
                && !path.starts_with("/usr/lib/")
                && !path.starts_with("/System/")
        })
        .map(str::to_owned)
        .collect()
}

fn linked(otool: &str) -> impl Iterator<Item = &str> {
    otool
        .lines()
        .skip(1)
        .filter_map(|line| line.trim().split(" (compatibility").next())
}

/// Where the build that made the app left llama.cpp, from `cargo build --message-format=json`.
///
/// Not `target/release`: the crate's build script keeps a file already there, so what sits there
/// may come from an older build or another feature set.
pub fn llama_libraries(messages: &str) -> Option<PathBuf> {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "build-script-executed")
        .filter(|message| {
            message["package_id"]
                .as_str()
                .is_some_and(|id| id.contains(LLAMA_PACKAGE))
        })
        .find_map(|message| Some(Path::new(message["out_dir"].as_str()?).join("lib")))
}

pub fn run(args: impl Iterator<Item = String>) -> anyhow::Result<()> {
    let mut install = false;
    for arg in args {
        match arg.as_str() {
            "--install" => install = true,
            other => bail!("unknown flag {other:?}"),
        }
    }
    ensure!(cfg!(target_os = "macos"), "app bundles are macOS only");

    let target = target_dir();
    let signer = local_signer()?.unwrap_or(Signer::AdHoc);
    let app = assemble(&target, &signer)?;
    println!("built {}", app.display());
    match signer {
        Signer::AdHoc => println!(
            "signed ad-hoc: macOS forgets its permissions when the code changes. \
             `cargo xtask signing-identity` keeps them across rebuilds."
        ),
        Signer::Local { .. } => println!("signed with {IDENTITY}"),
    }
    if install {
        println!("installed {}", install_app(&app)?.display());
    }
    Ok(())
}

/// Creates the local signing identity, or says it's there already.
pub fn create_identity() -> anyhow::Result<()> {
    ensure!(
        cfg!(target_os = "macos"),
        "signing identities are macOS only"
    );
    let keychain = keychain_path()?;
    if local_signer()?.is_some() {
        println!("{IDENTITY} is already in {}", keychain.display());
        return Ok(());
    }
    ensure!(
        !keychain.exists(),
        "{} exists without {IDENTITY} in it",
        keychain.display()
    );
    let folder = std::env::temp_dir().join(format!("wf-identity-{}", std::process::id()));
    tool("mkdir").args(["-m", "700"]).arg(&folder).succeed()?;
    let made = make_identity(&folder, &keychain);
    let _ = fs::remove_dir_all(&folder);
    made?;
    println!(
        "created {IDENTITY} in {}. Every `cargo xtask bundle` now signs with it.",
        keychain.display()
    );
    Ok(())
}

fn make_identity(folder: &Path, keychain: &Path) -> anyhow::Result<()> {
    const TRANSFER_PASSWORD: &str = "wispr-free";
    let (key, certificate, bundle) = (
        folder.join("key.pem"),
        folder.join("certificate.pem"),
        folder.join("identity.p12"),
    );
    tool(OPENSSL)
        .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout"])
        .arg(&key)
        .arg("-out")
        .arg(&certificate)
        .args(["-days", IDENTITY_DAYS, "-subj"])
        .arg(format!("/CN={IDENTITY}"))
        .args(["-addext", "extendedKeyUsage=critical,codeSigning"])
        .args(["-addext", "keyUsage=critical,digitalSignature"])
        .succeed()?;
    tool(OPENSSL)
        .args(["pkcs12", "-export", "-inkey"])
        .arg(&key)
        .arg("-in")
        .arg(&certificate)
        .arg("-out")
        .arg(&bundle)
        .args(["-name", IDENTITY, "-passout"])
        .arg(format!("pass:{TRANSFER_PASSWORD}"))
        .succeed()?;
    if let Some(parent) = keychain.parent() {
        fs::create_dir_all(parent)?;
    }
    tool("security")
        .args(["create-keychain", "-p", ""])
        .arg(keychain)
        .succeed()?;
    // No lock timeout, and only `codesign` may use the key.
    tool("security")
        .arg("set-keychain-settings")
        .arg(keychain)
        .succeed()?;
    unlock(keychain)?;
    tool("security")
        .arg("import")
        .arg(&bundle)
        .arg("-k")
        .arg(keychain)
        .args(["-P", TRANSFER_PASSWORD, "-T", "/usr/bin/codesign"])
        .succeed()?;
    tool("security")
        .args([
            "set-key-partition-list",
            "-S",
            "apple-tool:,apple:,codesign:",
        ])
        .args(["-s", "-k", ""])
        .arg(keychain)
        .succeed()?;
    Ok(())
}

fn unlock(keychain: &Path) -> anyhow::Result<()> {
    tool("security")
        .args(["unlock-keychain", "-p", ""])
        .arg(keychain)
        .succeed()
}

fn local_signer() -> anyhow::Result<Option<Signer>> {
    let keychain = keychain_path()?;
    if !keychain.exists() {
        return Ok(None);
    }
    unlock(&keychain)?;
    let found = tool("security")
        .args(["find-certificate", "-c", IDENTITY, "-Z"])
        .arg(&keychain)
        .text()?;
    let hash = found
        .lines()
        .find_map(|line| line.strip_prefix("SHA-1 hash: "))
        .map(str::to_owned);
    Ok(hash.map(|hash| Signer::Local { keychain, hash }))
}

fn keychain_path() -> anyhow::Result<PathBuf> {
    Ok(home()?.join(KEYCHAIN))
}

fn home() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

fn workspace_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::canonicalize(&root).unwrap_or(root)
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| workspace_root().join("target"), Into::into)
}

/// Lays out the bundle and signs it, replacing any earlier one.
fn assemble(target: &Path, signer: &Signer) -> anyhow::Result<PathBuf> {
    let (executable, libraries) = build_with_portable_libraries()?;
    let app = target.join(APP);
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    if app.exists() {
        fs::remove_dir_all(&app)?;
    }
    let resources = contents.join("Resources");
    fs::create_dir_all(&macos)?;
    fs::create_dir_all(&resources)?;
    fs::write(
        contents.join("Info.plist"),
        info_plist(env!("CARGO_PKG_VERSION")),
    )?;
    render_icon(target, &resources.join(format!("{ICON}.icns")))?;

    let binary = macos.join(EXECUTABLE);
    fs::copy(&executable, &binary)?;
    let carried = carry_libraries(&binary, &libraries, &macos)?;
    // Inside out: signing the app seals what it holds.
    for library in &carried {
        codesign(signer, library, None)?;
    }
    codesign(signer, &app, Some(BUNDLE_ID))?;
    tool("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&app)
        .succeed()?;
    // Fails if dyld can't find a library from inside the bundle.
    tool(&binary).arg("--version").current_dir("/").succeed()?;
    Ok(app)
}

/// Draws the icon from its SVG, each size from the source so small ones stay sharp.
fn render_icon(target: &Path, icns: &Path) -> anyhow::Result<()> {
    let source = workspace_root().join(ICON_SOURCE);
    let iconset = target.join(format!("{ICON}.iconset"));
    if iconset.exists() {
        fs::remove_dir_all(&iconset)?;
    }
    fs::create_dir_all(&iconset)?;
    for (name, pixels) in iconset_files() {
        let pixels = pixels.to_string();
        tool("sips")
            .args(["-s", "format", "png", "-z", &pixels, &pixels])
            .arg(&source)
            .arg("--out")
            .arg(iconset.join(name))
            .succeed()?;
    }
    tool("iconutil")
        .args(["-c", "icns", "-o"])
        .arg(icns)
        .arg(&iconset)
        .succeed()?;
    fs::remove_dir_all(&iconset)?;
    Ok(())
}

/// Builds the app. `.cargo/config.toml` keeps OpenSSL out of llama.cpp, but nothing tells cargo
/// that a build from before that line is stale, so such a build is cleaned once.
fn build_with_portable_libraries() -> anyhow::Result<(PathBuf, PathBuf)> {
    let (executable, libraries) = build()?;
    if !foreign_in(&libraries)?.is_empty() {
        println!("rebuilding llama.cpp: this build links libraries outside the app");
        tool("cargo")
            .args(["clean", "--release", "--package", "llama-cpp-sys-2"])
            .current_dir(workspace_root())
            .succeed()?;
        return build();
    }
    Ok((executable, libraries))
}

fn build() -> anyhow::Result<(PathBuf, PathBuf)> {
    let output = tool("cargo")
        .args(["build", "--release", "--locked", "--package", EXECUTABLE])
        .arg("--message-format=json-render-diagnostics")
        .current_dir(workspace_root())
        .stderr(Stdio::inherit())
        .text()?;
    let libraries = llama_libraries(&output).context("cargo didn't report llama.cpp's build")?;
    let executable = output
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["target"]["name"] == EXECUTABLE)
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
        .context("cargo didn't report the app's binary")?;
    Ok((executable, libraries))
}

fn foreign_in(libraries: &Path) -> anyhow::Result<Vec<String>> {
    let mut foreign = Vec::new();
    for entry in fs::read_dir(libraries)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "dylib")
        {
            foreign.extend(foreign_libraries(&otool(&path)?));
        }
    }
    Ok(foreign)
}

/// Copies every library `binary` reaches through the rpath next to it, where ADR-0003's
/// `@executable_path` rpath looks, under the name it asks for. Returns the copies.
fn carry_libraries(binary: &Path, from: &Path, to: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut carried = Vec::new();
    let mut waiting = vec![binary.to_path_buf()];
    while let Some(file) = waiting.pop() {
        let linked = otool(&file)?;
        let foreign = foreign_libraries(&linked);
        ensure!(
            foreign.is_empty(),
            "{} needs {foreign:?}, which won't be on another Mac",
            file.display()
        );
        for name in rpath_libraries(&linked) {
            let copy = to.join(&name);
            if !copy.exists() {
                fs::copy(from.join(&name), &copy).with_context(|| format!("copying {name}"))?;
                carried.push(copy.clone());
                waiting.push(copy);
            }
        }
    }
    Ok(carried)
}

fn otool(file: &Path) -> anyhow::Result<String> {
    tool("otool").arg("-L").arg(file).text()
}

fn codesign(signer: &Signer, path: &Path, identifier: Option<&str>) -> anyhow::Result<()> {
    let mut command = tool("codesign");
    command.args(["--force", "--timestamp=none"]);
    command.args(codesign_args(signer));
    if let Some(identifier) = identifier {
        command.args(["--identifier", identifier]);
    }
    command.arg(path).succeed()
}

/// Copies the app to `~/Applications`, replacing an earlier install of this app and nothing else.
fn install_app(app: &Path) -> anyhow::Result<PathBuf> {
    let folder = home()?.join("Applications");
    fs::create_dir_all(&folder)?;
    let installed = folder.join(APP);
    if installed.exists() {
        let plist = fs::read_to_string(installed.join("Contents/Info.plist")).unwrap_or_default();
        ensure!(
            plist.contains(BUNDLE_ID),
            "{} isn't this app, so it stays",
            installed.display()
        );
        fs::remove_dir_all(&installed)?;
    }
    tool("ditto").arg(app).arg(&installed).succeed()?;
    Ok(installed)
}

fn tool(program: impl AsRef<std::ffi::OsStr>) -> Tool {
    Tool(Command::new(program))
}

/// A command that fails loudly.
struct Tool(Command);

impl Tool {
    fn arg(&mut self, arg: impl AsRef<std::ffi::OsStr>) -> &mut Self {
        self.0.arg(arg);
        self
    }

    fn args(&mut self, args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> &mut Self {
        self.0.args(args);
        self
    }

    fn current_dir(&mut self, folder: impl AsRef<Path>) -> &mut Self {
        self.0.current_dir(folder);
        self
    }

    fn stderr(&mut self, stderr: Stdio) -> &mut Self {
        self.0.stderr(stderr);
        self
    }

    fn succeed(&mut self) -> anyhow::Result<()> {
        self.text().map(drop)
    }

    fn text(&mut self) -> anyhow::Result<String> {
        let output = self
            .0
            .output()
            .with_context(|| format!("running {:?}", self.0.get_program()))?;
        ensure!(
            output.status.success(),
            "{:?} failed: {}",
            self.0,
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}
