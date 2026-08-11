//! Claude CLI subprocess management.
//!
//! Runs `claude` commands via `std::process::Command` with
//! `CREATE_NO_WINDOW` on Windows to prevent console flash.

use std::path::Path;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub struct ClaudeRunner {
    pub claude_path: String,
}

impl ClaudeRunner {
    pub fn new(claude_path: &str) -> Self {
        Self {
            claude_path: claude_path.to_string(),
        }
    }

    /// Run `claude -p "/usage"` and return stdout.
    pub fn check_usage(&self) -> Result<String, String> {
        self.run_command(&["-p", "/usage"])
    }

    /// Run `claude --model <model> -p "<message>"` and return stdout.
    pub fn send_message(&self, model: &str, message: &str) -> Result<String, String> {
        self.run_command(&["--model", model, "-p", message])
    }

    fn run_command(&self, args: &[&str]) -> Result<String, String> {
        let mut cmd = self.command(args);

        // Suppress console window on Windows
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let output = cmd
            .output()
            .map_err(|e| format!("Claude çalıştırılamadı: {}\nPath: {}", e, self.claude_path))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        if output.status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(format!(
                "Claude hata (exit {}): {}{}",
                output.status, stdout, stderr
            ))
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        #[cfg(not(windows))]
        {
            if should_run_via_login_shell(&self.claude_path) {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
                let mut cmd = Command::new(shell);
                cmd.arg("-lc")
                    .arg(format!("exec {} \"$@\"", shell_quote(&self.claude_path)))
                    .arg("claude-timer-reset")
                    .args(args);
                return cmd;
            }
        }

        let mut cmd = Command::new(&self.claude_path);
        cmd.args(args);
        cmd
    }
}

#[cfg(not(windows))]
fn should_run_via_login_shell(program: &str) -> bool {
    !Path::new(program).is_absolute() && !program.contains('/')
}

#[cfg(not(windows))]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
