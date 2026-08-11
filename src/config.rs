//! Configuration management — JSON persistence + Claude CLI auto-detect.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_claude_path")]
    pub claude_path: String,

    #[serde(default = "default_model")]
    pub default_model: String,

    #[serde(default = "default_check_interval")]
    pub check_interval_minutes: u32,

    #[serde(default = "default_cooldown")]
    pub cooldown_seconds: u32,

    #[serde(default = "default_message")]
    pub test_message: String,

    #[serde(default)]
    pub auto_start: bool,
}

fn default_claude_path() -> String {
    detect_claude_path()
}
fn default_model() -> String {
    "haiku".into()
}
fn default_check_interval() -> u32 {
    60
}
fn default_cooldown() -> u32 {
    60
}
fn default_message() -> String {
    "bu bir test mesajıdır".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            claude_path: detect_claude_path(),
            default_model: default_model(),
            check_interval_minutes: default_check_interval(),
            cooldown_seconds: default_cooldown(),
            test_message: default_message(),
            auto_start: true,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}

/// Per-user data directory, created on first use. Falls back to the exe's
/// directory when the platform env var is missing.
pub fn app_data_dir() -> PathBuf {
    #[cfg(windows)]
    if let Ok(base) = std::env::var("LOCALAPPDATA") {
        let dir = PathBuf::from(base).join("claude-timer-reset");
        if fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }

    #[cfg(target_os = "macos")]
    if let Ok(home) = std::env::var("HOME") {
        let dir = PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Claude Timer Reset");
        if fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    if let Ok(home) = std::env::var("HOME") {
        let dir = PathBuf::from(home).join(".claude-timer-reset");
        if fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }

    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_config_path() -> PathBuf {
    app_data_dir().join("config.json")
}

/// Auto-detect the Claude CLI binary for the current platform.
pub fn detect_claude_path() -> String {
    // 1. Common install locations
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let npm_path = PathBuf::from(&appdata).join("npm").join("claude.cmd");
            if npm_path.exists() {
                return npm_path.to_string_lossy().into_owned();
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            for rel in [
                ".claude/local/claude", // native installer
                ".local/bin/claude",
                ".npm-global/bin/claude",
            ] {
                let candidate = PathBuf::from(&home).join(rel);
                if candidate.exists() {
                    return candidate.to_string_lossy().into_owned();
                }
            }
        }
        for abs in ["/opt/homebrew/bin/claude", "/usr/local/bin/claude"] {
            if Path::new(abs).exists() {
                return abs.to_string();
            }
        }

        if let Some(path) = find_in_login_shell("claude") {
            return path;
        }
    }

    // 2. Search PATH
    let sep = if cfg!(windows) { ';' } else { ':' };
    let names: &[&str] = if cfg!(windows) {
        &["claude.cmd", "claude.exe", "claude"]
    } else {
        &["claude"]
    };
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(sep) {
            for name in names {
                let candidate = PathBuf::from(dir).join(name);
                if candidate.exists() {
                    return candidate.to_string_lossy().into_owned();
                }
            }
        }
    }

    // 3. Fallback
    "claude".into()
}

#[cfg(not(windows))]
fn find_in_login_shell(binary: &str) -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let output = Command::new(shell)
        .arg("-lc")
        .arg(format!("command -v {}", shell_quote(binary)))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

#[cfg(not(windows))]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
