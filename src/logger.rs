//! Persistent file log — `app.log` in the per-user app-data directory, so the
//! app's own folder stays clean.
//!
//! Every scheduler event is appended here so problems can be diagnosed
//! even when the UI is hidden in the tray. The file self-trims: once it
//! grows past `MAX_BYTES`, only the newest half of the lines is kept.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 512 * 1024;

static LOCK: Mutex<()> = Mutex::new(());
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn log_path() -> PathBuf {
    LOG_PATH.get_or_init(init_log_path).clone()
}

fn init_log_path() -> PathBuf {
    let path = crate::config::app_data_dir().join("app.log");
    migrate_legacy_log(&path);
    path
}

/// One-time move of the old `app.log` that used to live next to the exe.
/// Copy + delete (rename fails across volumes); all errors ignored.
fn migrate_legacy_log(new_path: &Path) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(old) = exe.parent().map(|d| d.join("app.log")) else {
        return;
    };
    if old == new_path || !old.exists() {
        return;
    }
    if new_path.exists() || fs::copy(&old, new_path).is_ok() {
        let _ = fs::remove_file(&old);
    }
}

/// Append a timestamped line to `app.log`, trimming the file first if it
/// has grown past the size cap. Never panics — logging failures are ignored.
pub fn log(msg: &str) {
    let _guard = LOCK.lock();
    let path = log_path();
    trim_if_needed(&path);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

fn trim_if_needed(path: &Path) {
    let Ok(meta) = fs::metadata(path) else { return };
    if meta.len() <= MAX_BYTES {
        return;
    }
    if let Ok(content) = fs::read_to_string(path) {
        let total = content.lines().count();
        let kept: String = content
            .lines()
            .skip(total / 2)
            .flat_map(|l| [l, "\n"])
            .collect();
        let _ = fs::write(path, kept);
    }
}
