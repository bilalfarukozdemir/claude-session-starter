//! "Launch at startup" — register the app to run at user login.
//!
//! Windows uses the per-user `Run` registry key. macOS uses a per-user
//! LaunchAgent plist in `~/Library/LaunchAgents`. No admin rights are needed.

/// Registry value name under the `Run` key. Uniquely identifies our entry.
#[cfg(windows)]
const VALUE_NAME: &str = "ClaudeTimerReset";

#[cfg(windows)]
const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

// ── Windows implementation ───────────────────────────────────────────────────

#[cfg(windows)]
mod imp {
    use super::{RUN_SUBKEY, VALUE_NAME};
    use std::ptr;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
    };

    /// UTF-16, null-terminated — the encoding every `*W` Win32 API expects.
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Open the `Run` key with the given access rights. Caller must
    /// `RegCloseKey` the returned handle.
    fn open_run_key(access: u32) -> Result<HKEY, String> {
        let subkey = wide(RUN_SUBKEY);
        let mut hkey: HKEY = ptr::null_mut();
        // SAFETY: valid null-terminated subkey; hkey is a live out-param.
        // The 3rd arg (ulOptions) is reserved for RegOpenKeyExW and must be 0.
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, access, &mut hkey) };
        if status == ERROR_SUCCESS {
            Ok(hkey)
        } else {
            Err(format!("RegOpenKeyExW failed (code {status})"))
        }
    }

    /// Absolute path to the running executable, wrapped in quotes so a path
    /// containing spaces survives the shell that Windows uses at login.
    fn quoted_exe_path() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
        Ok(format!("\"{}\"", exe.display()))
    }

    pub fn is_enabled() -> bool {
        let Ok(hkey) = open_run_key(KEY_QUERY_VALUE) else {
            return false;
        };
        let name = wide(VALUE_NAME);
        // Query with a null data buffer — we only care whether the value
        // exists, not its contents.
        // SAFETY: hkey is valid; name is null-terminated; all data out-params null.
        let status = unsafe {
            RegQueryValueExW(
                hkey,
                name.as_ptr(),
                ptr::null(), // lpReserved is *const u32
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        // SAFETY: hkey came from a successful open.
        unsafe { RegCloseKey(hkey) };
        status == ERROR_SUCCESS
    }

    pub fn set_enabled(enable: bool) -> Result<(), String> {
        if enable {
            enable_entry()
        } else {
            disable_entry()
        }
    }

    fn enable_entry() -> Result<(), String> {
        let value = quoted_exe_path()?;
        let hkey = open_run_key(KEY_SET_VALUE)?;
        let name = wide(VALUE_NAME);
        let data = wide(&value);
        // REG_SZ byte count includes the trailing null (u16 → 2 bytes each).
        let cb = (data.len() * std::mem::size_of::<u16>()) as u32;
        // SAFETY: hkey valid; name null-terminated; data points to `cb` valid bytes.
        let status = unsafe {
            RegSetValueExW(
                hkey,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                cb,
            )
        };
        // SAFETY: hkey came from a successful open.
        unsafe { RegCloseKey(hkey) };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("RegSetValueExW failed (code {status})"))
        }
    }

    fn disable_entry() -> Result<(), String> {
        let hkey = open_run_key(KEY_SET_VALUE)?;
        let name = wide(VALUE_NAME);
        // SAFETY: hkey valid; name null-terminated.
        let status = unsafe { RegDeleteValueW(hkey, name.as_ptr()) };
        // SAFETY: hkey came from a successful open.
        unsafe { RegCloseKey(hkey) };
        // A missing value means we were already disabled — treat as success.
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("RegDeleteValueW failed (code {status})"))
        }
    }
}

// ── macOS implementation ─────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod imp {
    use std::fs;
    use std::path::PathBuf;

    const LABEL: &str = "com.claude-timer-reset.app";
    const PLIST_NAME: &str = "com.claude-timer-reset.app.plist";

    fn plist_path() -> Result<PathBuf, String> {
        let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
        let dir = PathBuf::from(home).join("Library").join("LaunchAgents");
        fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create LaunchAgents directory: {e}"))?;
        Ok(dir.join(PLIST_NAME))
    }

    fn current_exe() -> Result<PathBuf, String> {
        std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))
    }

    fn plist_xml(exe: &std::path::Path) -> String {
        let working_dir = exe
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".into());
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>WorkingDirectory</key>
    <string>{}</string>
</dict>
</plist>
"#,
            xml_escape(LABEL),
            xml_escape(&exe.to_string_lossy()),
            xml_escape(&working_dir)
        )
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    pub fn is_enabled() -> bool {
        plist_path().is_ok_and(|path| path.exists())
    }

    pub fn set_enabled(enable: bool) -> Result<(), String> {
        let path = plist_path()?;
        if enable {
            let exe = current_exe()?;
            fs::write(&path, plist_xml(&exe))
                .map_err(|e| format!("could not write LaunchAgent: {e}"))
        } else if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("could not remove LaunchAgent: {e}"))
        } else {
            Ok(())
        }
    }
}

// ── Other platforms ──────────────────────────────────────────────────────────
//
// Keep the app compiling on Linux/BSD without promising a login integration.

#[cfg(all(not(windows), not(target_os = "macos")))]
mod imp {
    pub fn is_enabled() -> bool {
        false
    }

    pub fn set_enabled(_enable: bool) -> Result<(), String> {
        Err("Launch at startup is only supported on Windows and macOS".into())
    }
}

/// Whether the app is currently registered to launch at login.
pub fn is_enabled() -> bool {
    imp::is_enabled()
}

/// Register (`true`) or unregister (`false`) the app for launch at login.
pub fn set_enabled(enable: bool) -> Result<(), String> {
    imp::set_enabled(enable)
}

pub fn checkbox_label() -> &'static str {
    if cfg!(windows) {
        "Run when Windows starts"
    } else if cfg!(target_os = "macos") {
        "Run when macOS starts"
    } else {
        "Run at login"
    }
}
