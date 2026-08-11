//! Background scheduler — periodically checks /usage, sets timer, auto-starts sessions.
//!
//! Runs in a dedicated thread, communicates with the UI via `mpsc` channels.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

use crate::claude_runner::ClaudeRunner;
use crate::usage_parser::{self, UsageInfo};

// ── Events (scheduler → UI) ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Event {
    /// /usage parsed successfully
    UsageChecked(UsageInfo),
    /// Timer target set for this time
    TimerSet(DateTime<Local>),
    /// No valid timer can be scheduled from the latest usage check
    TimerCleared,
    /// Claude responded to our test message
    MessageSent(String),
    /// Something went wrong
    Error(String),
    /// General log line
    Log(String),
    /// Currently running /usage check
    Checking,
}

// ── Commands (UI → scheduler) ────────────────────────────────────────────────

pub enum Command {
    Start,
    Stop,
    CheckNow,
    UpdateConfig {
        claude_path: String,
        model: String,
        message: String,
        check_interval_minutes: u32,
        cooldown_seconds: u32,
    },
    Quit,
}

// ── Public handle ────────────────────────────────────────────────────────────

pub struct Scheduler {
    pub cmd_tx: mpsc::Sender<Command>,
    pub event_rx: mpsc::Receiver<Event>,
}

impl Scheduler {
    /// Spawn the background scheduler thread.
    pub fn new(
        claude_path: String,
        model: String,
        message: String,
        check_interval_minutes: u32,
        cooldown_seconds: u32,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        thread::spawn(move || {
            scheduler_loop(
                cmd_rx,
                event_tx,
                claude_path,
                model,
                message,
                check_interval_minutes,
                cooldown_seconds,
            );
        });

        Self { cmd_tx, event_rx }
    }
}

// ── Background loop ──────────────────────────────────────────────────────────

fn scheduler_loop(
    cmd_rx: mpsc::Receiver<Command>,
    tx: mpsc::Sender<Event>,
    mut claude_path: String,
    mut model: String,
    mut message: String,
    mut check_interval_minutes: u32,
    mut cooldown_seconds: u32,
) {
    let mut active = false;
    let mut force_check = false;
    let mut next_check = Instant::now();
    let mut timer_target: Option<DateTime<Local>> = None;

    loop {
        // ── Drain pending commands ──
        loop {
            match cmd_rx.try_recv() {
                Ok(Command::Start) => {
                    active = true;
                    next_check = Instant::now(); // check immediately
                    emit(&tx, Event::Log("▶ Autonomous mode started".into()));
                }
                Ok(Command::Stop) => {
                    active = false;
                    timer_target = None;
                    emit(&tx, Event::Log("⏹ Stopped".into()));
                }
                Ok(Command::CheckNow) => {
                    force_check = true;
                }
                Ok(Command::UpdateConfig {
                    claude_path: cp,
                    model: m,
                    message: msg,
                    check_interval_minutes: ci,
                    cooldown_seconds: cs,
                }) => {
                    claude_path = cp;
                    model = m;
                    message = msg;
                    check_interval_minutes = ci;
                    cooldown_seconds = cs;
                    emit(&tx, Event::Log("✓ Configuration updated".into()));
                }
                Ok(Command::Quit) => return,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }

        {
            let now_instant = Instant::now();

            // ── Periodic /usage check (Check Now works even when stopped) ──
            if force_check || (active && now_instant >= next_check) {
                force_check = false;
                emit(&tx, Event::Checking);
                emit(&tx, Event::Log("📊 Checking /usage...".into()));

                let runner = ClaudeRunner::new(&claude_path);
                match runner.check_usage() {
                    Ok(output) => {
                        if let Some(info) = usage_parser::parse_usage(&output) {
                            let now = Local::now();
                            match info.reset_time {
                                // A reset clause on the session line means a session is
                                // active — schedule for it, whatever the window length
                                // (session windows have been observed at 5h and ~8h; don't
                                // assume one). The 24h cap guards against /usage reporting
                                // the *weekly* reset on the session line, seen on older
                                // CLI versions when no session was active.
                                Some(reset) if reset - now <= chrono::Duration::hours(24) => {
                                    let target =
                                        reset + chrono::Duration::seconds(cooldown_seconds as i64);
                                    timer_target = Some(target);

                                    emit(&tx, Event::UsageChecked(info));
                                    emit(&tx, Event::TimerSet(target));
                                    emit(
                                        &tx,
                                        Event::Log(format!(
                                            "⏰ Reset: {} → Timer: {}",
                                            reset.format("%H:%M"),
                                            target.format("%H:%M:%S")
                                        )),
                                    );
                                }
                                // No reset clause at all can mean either a fresh
                                // window (0%) or a CLI output format we do not
                                // understand yet. Only auto-start on 0%; otherwise
                                // wait for a future check instead of spamming prompts.
                                _ => {
                                    let has_usage = info.session_percent > 0;
                                    emit(&tx, Event::UsageChecked(info));
                                    if active && has_usage {
                                        timer_target = None;
                                        emit(&tx, Event::TimerCleared);
                                        emit(
                                            &tx,
                                            Event::Log(
                                                "⚠ Reset time not found — waiting for the next check"
                                                    .into(),
                                            ),
                                        );
                                        crate::logger::log(&format!(
                                            "RAW /usage output without reset time:\n{}",
                                            output
                                        ));
                                    } else if active {
                                        emit(
                                            &tx,
                                            Event::Log(
                                                "💤 No active session — starting one now".into(),
                                            ),
                                        );
                                        // Fires in the timer check below
                                        timer_target = Some(now);
                                        emit(&tx, Event::TimerSet(now));
                                    } else {
                                        emit(
                                            &tx,
                                            Event::Log(
                                                "💤 No active session (press Start to open one)"
                                                    .into(),
                                            ),
                                        );
                                    }
                                }
                            }
                        } else {
                            // The CLI often prints an error (expired login, bad
                            // model, rate limit) instead of usage data. Surface a
                            // clear cause when we recognize one; otherwise stay generic.
                            let msg = usage_parser::classify_cli_error(&output)
                                .unwrap_or_else(|| "Could not parse usage output".into());
                            emit(&tx, Event::Error(msg));
                            // Full raw output goes to app.log; UI gets a preview
                            crate::logger::log(&format!("RAW /usage output:\n{}", output));
                            let preview: String = output.chars().take(200).collect();
                            emit(&tx, Event::Log(format!("Raw: {}", preview)));
                        }
                    }
                    Err(e) => {
                        // A non-zero exit also carries CLI error text — classify it too.
                        let msg = usage_parser::classify_cli_error(&e).unwrap_or(e);
                        emit(&tx, Event::Error(msg));
                    }
                }

                next_check = now_instant + Duration::from_secs(check_interval_minutes as u64 * 60);
            }

            // ── Timer check (only fires in autonomous mode) ──
            if let Some(target) = timer_target.filter(|_| active) {
                if Local::now() >= target {
                    timer_target = None;
                    emit(
                        &tx,
                        Event::Log("🤖 Session reset! Sending message...".into()),
                    );

                    let runner = ClaudeRunner::new(&claude_path);
                    match runner.send_message(&model, &message) {
                        Ok(response) => {
                            let preview: String = response.chars().take(150).collect();
                            emit(&tx, Event::MessageSent(preview));
                        }
                        Err(e) => {
                            emit(&tx, Event::Error(e));
                        }
                    }

                    // Re-check /usage shortly after sending message
                    next_check = Instant::now() + Duration::from_secs(60);
                }
            }
        }

        // Sleep 1 second between iterations to stay light on CPU
        thread::sleep(Duration::from_secs(1));
    }
}

/// Send an event to the UI and mirror it to the persistent file log,
/// so problems are diagnosable even while the window is hidden in the tray.
fn emit(tx: &mpsc::Sender<Event>, ev: Event) {
    let line = match &ev {
        Event::UsageChecked(i) => format!(
            "usage checked: session {}% · reset {:?} · week {:?}",
            i.session_percent, i.reset_time, i.week_percent
        ),
        Event::TimerSet(t) => format!("timer set → {}", t.format("%Y-%m-%d %H:%M:%S")),
        Event::TimerCleared => "timer cleared".into(),
        Event::MessageSent(r) => format!("message sent · response: {}", r),
        Event::Error(e) => format!("ERROR: {}", e),
        Event::Log(m) => m.clone(),
        Event::Checking => "checking /usage...".into(),
    };
    crate::logger::log(&line);
    let _ = tx.send(ev);
}
