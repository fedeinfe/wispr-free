//! Launch at login: a LaunchAgent on macOS, a registry Run value on Windows and an XDG autostart
//! entry on Linux.

use std::io;
#[cfg(not(target_os = "windows"))]
use std::{
    fs,
    path::{Path, PathBuf},
};

/// The OS entry that starts `executable` when the user logs in.
#[cfg(not(target_os = "windows"))]
#[derive(Debug, Clone)]
pub struct LaunchAtLogin {
    entry: PathBuf,
    executable: PathBuf,
}

#[cfg(not(target_os = "windows"))]
impl LaunchAtLogin {
    /// The entry for this user and the running executable.
    pub fn system() -> io::Result<Self> {
        Ok(Self::at(entry_path()?, &std::env::current_exe()?))
    }

    pub fn at(entry: impl Into<PathBuf>, executable: &Path) -> Self {
        Self {
            entry: entry.into(),
            executable: executable.to_path_buf(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.entry.exists()
    }

    pub fn set(&self, enabled: bool) -> io::Result<()> {
        if !enabled {
            return match fs::remove_file(&self.entry) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        if let Some(parent) = self.entry.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.entry, entry_text(&self.executable))
    }
}

#[cfg(target_os = "macos")]
fn entry_path() -> io::Result<PathBuf> {
    let user = directories::UserDirs::new().ok_or(io::ErrorKind::NotFound)?;
    Ok(user
        .home_dir()
        .join("Library/LaunchAgents/com.wispr-free.app.plist"))
}

#[cfg(target_os = "linux")]
fn entry_path() -> io::Result<PathBuf> {
    let base = directories::BaseDirs::new().ok_or(io::ErrorKind::NotFound)?;
    Ok(base.config_dir().join("autostart/wispr-free.desktop"))
}

#[cfg(target_os = "macos")]
fn entry_text(executable: &Path) -> String {
    let path = executable
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.wispr-free.app</string>
    <key>ProgramArguments</key>
    <array>
        <string>{path}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#
    )
}

#[cfg(target_os = "linux")]
fn entry_text(executable: &Path) -> String {
    let path = executable.to_string_lossy().replace('"', "\\\"");
    format!(
        "[Desktop Entry]\nType=Application\nName=wispr-free\nExec=\"{path}\"\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// The Run value that starts the running executable when the user logs in.
#[cfg(target_os = "windows")]
#[derive(Debug, Clone)]
pub struct LaunchAtLogin {
    executable: std::path::PathBuf,
}

#[cfg(target_os = "windows")]
impl LaunchAtLogin {
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "wispr-free";

    pub fn system() -> io::Result<Self> {
        Ok(Self {
            executable: std::env::current_exe()?,
        })
    }

    pub fn is_enabled(&self) -> bool {
        std::process::Command::new("reg")
            .args(["query", Self::KEY, "/v", Self::VALUE])
            .output()
            .is_ok_and(|output| output.status.success())
    }

    pub fn set(&self, enabled: bool) -> io::Result<()> {
        let mut reg = std::process::Command::new("reg");
        if enabled {
            let command = format!("\"{}\"", self.executable.display());
            reg.args(["add", Self::KEY, "/v", Self::VALUE, "/t", "REG_SZ", "/d"])
                .arg(command)
                .arg("/f");
        } else if self.is_enabled() {
            reg.args(["delete", Self::KEY, "/v", Self::VALUE, "/f"]);
        } else {
            return Ok(());
        }
        let status = reg.output()?.status;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("reg exited with {status}")))
        }
    }
}
