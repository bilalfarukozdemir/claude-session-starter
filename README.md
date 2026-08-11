# Claude Timer Reset

A tiny native desktop app that tracks your Claude CLI session usage and automatically starts a fresh session the moment your limit resets — so a new 5-hour window is always ticking when you sit down to work.

![Screenshot](screenshot.png)

## How it works

1. On a configurable interval, the app runs `claude -p "/usage"` in the background and parses your session usage percentage and the reset time (e.g. `resets Jul 11, 12:30pm`).
2. It schedules a countdown targeting the reset time plus a small safety cooldown (default +60 s, to let server clocks sync).
3. When the countdown hits zero, it sends one lightweight prompt (default model: `haiku`) to open a fresh session, then re-checks usage to schedule the next cycle.

Close the window and it keeps working from the system tray. Left-click the tray icon to bring the window back; right-click for **Open** / **Quit**.

## Features

- **Autonomous** — check → schedule → trigger → repeat, no interaction needed after pressing Start.
- **Lightweight** — single native binary (~7 MB), ~10–15 MB RAM. No Node, no Python, no Electron.
- **Tray-first** — closing the window hides it to the tray; the scheduler keeps running in the background.
- **Live dashboard** — big countdown to the next session, session/weekly usage bars, event log.
- **No console flashing** — Claude CLI calls run fully hidden (no terminal windows popping up).
- **Auto-detects the Claude CLI** — finds `claude` in the usual npm/Homebrew/native-installer locations, or set the path manually in Settings.

## Getting started

### Windows

1. Download `claude-timer-reset.exe` from the [latest release](https://github.com/ozkanerbatuhan/claude-session-starter/releases/latest) (or build from source) and run it.
2. Open **Settings** if you need to change anything.

   | Setting | Default | Notes |
   |---|---|---|
   | Model | `haiku` | Cheapest way to open a session (`sonnet`, `opus` also available) |
   | Message | test message | The prompt sent to start the fresh session |
   | Claude path | auto-detected | Absolute path to `claude.cmd` / `claude` if detection fails |
   | Check interval | 60 min | How often `/usage` is polled |
   | Wait after reset | 60 s | Cooldown after the reset time before triggering |
   | Launch at startup | off | Run the app automatically at user login (per-user, no admin needed) |

3. Press **Start**. The app checks usage, schedules the countdown, and takes it from there. Settings (including the running state) persist in the per-user app data folder, so it resumes automatically on next launch.

### macOS

Build from source (see below), then run the binary:

```bash
cargo build --release
./target/release/claude-timer-reset
```

The app auto-detects Claude from common native installer, npm, and Homebrew locations. If Claude was installed through a shell-managed path such as `nvm`, the app also asks your login shell where `claude` lives, so it still works when launched from Finder or at login.

The **Launch at startup** toggle writes a per-user LaunchAgent at `~/Library/LaunchAgents/com.claude-timer-reset.app.plist`.

## Building from source

Requires [Rust](https://rustup.rs/) (stable).

```bash
cargo build --release
```

The optimized binary lands in `target/release/`.

## Requirements

- Windows 10/11 or macOS
- Claude CLI installed and authenticated (`npm install -g @anthropic-ai/claude-code`)

## Data files

- Windows: `%LOCALAPPDATA%\claude-timer-reset\config.json` and `app.log`
- macOS: `~/Library/Application Support/Claude Timer Reset/config.json` and `app.log`

## Architecture

```
src/
├── main.rs           # eframe entry point
├── app.rs            # egui UI, tray icon, native window show/hide
├── scheduler.rs      # background thread: usage checks + countdown + trigger
├── claude_runner.rs  # Claude CLI subprocess wrapper (hidden console)
├── usage_parser.rs   # parses `/usage` output (percentages, reset time)
├── startup.rs        # "launch at login" via Windows Run key or macOS LaunchAgent
├── logger.rs         # persistent app.log with auto-trim
├── updater.rs        # in-app self-update from GitHub Releases
└── config.rs         # config.json persistence + CLI auto-detection
```

The UI and the scheduler run on separate threads and talk over `mpsc` channels — the UI never blocks on a CLI call.

## License

MIT
