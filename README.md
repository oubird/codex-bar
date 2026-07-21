# CodexPlus

A Windows title-bar overlay that surfaces your Codex 5-hour and weekly
usage limits on the Codex desktop app. Built as a single Rust binary with
no WebView2, Node.js, Python, or PowerShell runtime dependencies.

## Status

This is an **MVP scaffold**. The non-Win32 surface is fully tested
(45 unit tests covering credentials, provider parsing, formatting, and
last-good caching). The actual HWND/mouse-passthrough plumbing for the
title-bar overlay and a system tray icon are stubbed in
`src/ui/native.rs` and `src/ui/overlay.rs`. Run
`cargo run --release` to start the polling worker; the Win32 host window
and `SetWinEventHook` integration will be added in the next pass.

## Layout

```text
src/
├─ lib.rs           # crate root
├─ main.rs          # `#![windows_subsystem = "windows"]` entry point
├─ model.rs         # WindowKind / RateWindow / UsageSnapshot / ViewState
├─ error.rs         # AppError taxonomy (no token/header leakage)
├─ formatting.rs    # overlay row + tray tooltip rendering
├─ credentials.rs   # read-only Codex CLI auth.json loader
├─ provider/        # Codex /wham/usage client with strict validation
├─ storage.rs       # atomic last-good cache in %LOCALAPPDATA%
└─ ui/
   ├─ native.rs     # Win32 glue + run_app()
   ├─ target_window.rs  # identifies the Codex desktop window
   ├─ overlay.rs    # click-through popup (host logic)
   ├─ render.rs     # pure rendering helpers (text/tone)
   ├─ poll.rs       # background refresh thread
   ├─ tray.rs       # tray-icon request formatter
   └─ tests.rs      # cross-module sanity tests

references/
├─ Win-CodexBar/         # read-only — not modified by CodexPlus
├─ Claude-Code-Usage-Monitor/  # read-only
└─ codex-usage-monitor/        # read-only
```

## Building

```powershell
cargo build --release
.\target\release\codexplus.exe
```

## Testing

```powershell
cargo test --lib
cargo test --release --lib
```

## Security posture

- Reads `%USERPROFILE%\.codex\auth.json` (or `%CODEX_HOME%\auth.json`).
- Does *not* read `OPENAI_API_KEY` as a ChatGPT OAuth token.
- Does *not* read `refresh_token`, does *not* start `codex exec`.
- Token is wrapped in `SecretString`: `Debug` → `[REDACTED]`, no
  `Display`, never serialized to the cache or logs.
- Endpoint is compiled in: `https://chatgpt.com/backend-api/wham/usage`.
- No redirect-following; cache only ever holds a numeric snapshot.

## Known limitations of this scaffold

The MVP does not yet:
- Create the actual top-level host window / `SetWinEventHook`.
- Wire a real `Shell_NotifyIconW` tray icon.
- Hook foreground changes / max/min events on the Codex window.

These land in a follow-up pass that will exercise `references/`-style
patterns for foreground tracking and tray icons.
