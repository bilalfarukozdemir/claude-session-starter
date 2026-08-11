//! Self-update via GitHub Releases.
//!
//! A background thread checks the latest release on startup and every 24h.
//! When a newer version exists the UI shows a banner; on "Download" the new
//! exe is fetched and swapped in place (rename the running exe to `.old`,
//! move the download to the original path — legal on Windows and Unix).
//! The user then restarts from the banner.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::logger;

const RELEASES_API: &str =
    "https://api.github.com/repos/ozkanerbatuhan/claude-session-starter/releases/latest";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Release binaries stay well under this; guards against a runaway download.
const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

// ── Events (updater → UI) ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum UpdateEvent {
    Available { version: String, url: String },
    Downloading,
    Ready,
    Error(String),
}

// ── Commands (UI → updater) ──────────────────────────────────────────────────

pub enum UpdateCommand {
    Download { url: String },
}

// ── Public handle ────────────────────────────────────────────────────────────

pub struct Updater {
    pub cmd_tx: mpsc::Sender<UpdateCommand>,
    pub event_rx: mpsc::Receiver<UpdateEvent>,
}

impl Updater {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        thread::spawn(move || updater_loop(cmd_rx, event_tx));
        Self { cmd_tx, event_rx }
    }
}

/// Delete a leftover `<exe>.old` from a previous update. Errors are ignored —
/// the old instance may still be exiting; the file gets removed on a later run.
pub fn remove_stale_backup() {
    if let Some(backup) = backup_path() {
        if backup.exists() {
            let _ = fs::remove_file(backup);
        }
    }
}

// ── Background loop ──────────────────────────────────────────────────────────

fn updater_loop(cmd_rx: mpsc::Receiver<UpdateCommand>, tx: mpsc::Sender<UpdateEvent>) {
    loop {
        match check_latest() {
            Ok(Some((version, url))) => {
                logger::log(&format!("update available: v{}", version));
                let _ = tx.send(UpdateEvent::Available {
                    version,
                    url: url.clone(),
                });
            }
            Ok(None) => {
                logger::log(&format!(
                    "update check: v{} is up to date",
                    env!("CARGO_PKG_VERSION")
                ));
            }
            Err(e) => {
                // Network hiccups are routine (offline laptop, rate limit) —
                // log only, don't alarm the UI.
                logger::log(&format!("update check failed: {}", e));
            }
        }

        // Wait for a Download command until the next daily check.
        match cmd_rx.recv_timeout(CHECK_INTERVAL) {
            Ok(UpdateCommand::Download { url }) => {
                let _ = tx.send(UpdateEvent::Downloading);
                logger::log(&format!("downloading update from {}", url));
                match download_and_swap(&url) {
                    Ok(()) => {
                        logger::log("update installed — restart to apply");
                        let _ = tx.send(UpdateEvent::Ready);
                        // Swap done; nothing further to do until restart.
                        return;
                    }
                    Err(e) => {
                        logger::log(&format!("update failed: {}", e));
                        let _ = tx.send(UpdateEvent::Error(e));
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {} // daily re-check
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

// ── GitHub API ───────────────────────────────────────────────────────────────

/// Query the latest release. Returns `Some((version, download_url))` when it
/// is newer than the running build, `None` when already up to date.
fn check_latest() -> Result<Option<(String, String)>, String> {
    let body = ureq::get(RELEASES_API)
        .set("User-Agent", "claude-timer-reset")
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| format!("request failed: {}", e))?
        .into_string()
        .map_err(|e| format!("read failed: {}", e))?;

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("bad JSON: {}", e))?;

    let tag = json["tag_name"].as_str().ok_or("no tag_name in release")?;
    let latest = tag.trim_start_matches('v');

    if !is_newer(latest, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }

    let url = json["assets"]
        .as_array()
        .and_then(|assets| {
            assets.iter().find(|a| {
                a["name"]
                    .as_str()
                    .is_some_and(|n| n.ends_with(update_asset_suffix()))
            })
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or("release has no matching binary asset")?;

    Ok(Some((latest.to_string(), url.to_string())))
}

fn update_asset_suffix() -> &'static str {
    if cfg!(windows) {
        ".exe"
    } else {
        "claude-timer-reset"
    }
}

/// Numeric per-component version compare: is `candidate` newer than `current`?
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parse(candidate), parse(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

// ── Download + swap ──────────────────────────────────────────────────────────

fn backup_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.with_extension("old"))
}

fn download_and_swap(url: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("no exe path: {}", e))?;
    let staging = exe.with_extension("new");
    let backup = exe.with_extension("old");

    // Download to a staging file next to the exe (same volume → rename works)
    let resp = ureq::get(url)
        .set("User-Agent", "claude-timer-reset")
        .timeout(Duration::from_secs(600))
        .call()
        .map_err(|e| format!("download failed: {}", e))?;

    let mut bytes = Vec::new();
    resp.into_reader()
        .take(MAX_DOWNLOAD_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("download read failed: {}", e))?;
    if bytes.is_empty() {
        return Err("downloaded file is empty".into());
    }
    fs::write(&staging, &bytes).map_err(|e| format!("write failed: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&staging, fs::Permissions::from_mode(0o755));
    }

    // Swap: running exe → .old, staged download → exe path
    let _ = fs::remove_file(&backup);
    fs::rename(&exe, &backup).map_err(|e| format!("could not move running exe: {}", e))?;
    if let Err(e) = fs::rename(&staging, &exe) {
        // Roll back so the app still launches next time
        let _ = fs::rename(&backup, &exe);
        return Err(format!("could not install new exe: {}", e));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn test_version_compare() {
        assert!(is_newer("1.2.0", "1.1.4"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("1.1.10", "1.1.9"));
        assert!(!is_newer("1.2.0", "1.2.0"));
        assert!(!is_newer("1.1.4", "1.2.0"));
        assert!(is_newer("v1.3.0", "1.2.0")); // tolerates stray v prefix
        assert!(is_newer("1.2", "1.1.9")); // shorter version strings
    }
}
