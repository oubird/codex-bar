# CodexPlus

[简体中文](README.md) | English

> A resident Windows notification-area app that draws a transparent overlay on the right side of the foreground Codex window title bar. The overlay shows the **remaining quota** for the 5-hour and weekly limit windows together with the time until reset. You can switch UI language between Simplified Chinese and English in Settings.

<table>
  <colgroup>
    <col style="width: 18%">
    <col style="width: 82%">
  </colgroup>
  <thead>
    <tr><th align="left">Item</th><th align="left">Detail</th></tr>
  </thead>
  <tbody>
    <tr><td>Platform</td><td>Windows 10 / 11 (x64)</td></tr>
    <tr><td>Runtime</td><td>Single EXE, native Win32; no WebView2, Node.js, Python, or PowerShell runtime required</td></tr>
    <tr><td>Data</td><td>Read-only access to Codex CLI's <code>auth.json</code>; only the fixed <code>wham/usage</code> endpoint is requested</td></tr>
    <tr><td>Privacy</td><td>Tokens, account IDs, proxy URLs, and proxy credentials are never written to cache, diagnostics, or UI</td></tr>
    <tr><td>UI</td><td>User-visible surfaces (title-bar overlay, tray menu/tooltip, Settings window, error status, diagnostics completion dialog) ship in both Simplified Chinese and English; the technical diagnostics log stays in English so log search remains stable</td></tr>
  </tbody>
</table>

## Preview

![CodexPlus overlay on the Codex title bar in English: remaining 67% weekly limit, 4 days 20 hours](assets/screenshot-overlay.png)

When a Codex desktop window is in the foreground an overlay appears just to the left of the minimise button on the title bar, showing the remaining percentage and the reset countdown. The overlay hides itself whenever the Codex window is covered or minimised.

## Features

- **Title-bar overlay** — when the foreground window is recognised as Codex, the overlay renders a compact quota strip on the title bar: `5h 58% 1h18m   Weekly 75% 5d14h` in English or `5小时 58% 1时18分   周限额 75% 5天14时` in Simplified Chinese. The percentage is the remaining quota; when the server only returns a single window (currently only the weekly limit), only that segment renders. When there is no cached data, the overlay shows a localised placeholder (`Codex 用量 · 加载中…` / `Codex usage · Loading…` or `Codex 用量 · 暂不可用` / `Codex usage · Unavailable`) instead of vanishing silently.
- **Anti-aliased rendering** — text is rasterised at 3× the logical resolution into an off-screen DIB and downsampled using per-pixel coverage. This avoids the ClearType colour fringes and stair-step edges that are typical of layered transparent windows.
- **Right-aligned to the title bar** — text is right-aligned to the minimise button. Long lines extend to the left; the overlay never covers interactive controls. It is no longer global TOPMOST, does not steal focus, does not appear in the taskbar or Alt+Tab list, and remains mouse-transparent.
- **Focus-bound rendering** — the overlay only paints while the Codex window is visible and not minimised; when other windows cover the Codex window the overlay stays covered together with it.
- **Resident tray icon** — CodexPlus keeps an icon in the notification area on startup (sourced from `assets/app.ico`). The right-click menu and tooltip both follow the active UI language. Menu items in English are `Refresh now / Show title-bar usage / Settings… / Run diagnostics / Exit`.
- **Background refresh and cache** — polls the endpoint every 300 seconds by default. Network or protocol errors keep the last good snapshot and mark it as stale (grey + `!`); auth problems back off the refresh cadence. The on-disk cache schema is v2, backward compatible with v1 and weekly-only historical formats.
- **Single-window tolerance** — when the server temporarily disables the 5-hour window and only returns the weekly limit, CodexPlus still classifies and renders the response without surfacing a protocol error.
- **Settings window (light modern style)** — native Win32 window with language switcher (简体中文 / English) alongside the existing toggles: enable the overlay, set the refresh cadence between 60 and 3600 seconds, choose whether the overlay shows remaining or used quota, choose countdown (`5d14h` style) vs. absolute (`Jul 30 19:34` style) reset text, and pick environment / direct / custom proxy modes. Inputs, buttons, and backgrounds are visually distinct.

## Quick start

### 1. Log in to Codex CLI

```powershell
npm install -g @openai/codex
codex login
Test-Path $env:USERPROFILE\.codex\auth.json
```

`auth.json` can also live at `%CODEX_HOME%\auth.json`. CodexPlus only reads its access token, never modifies it, and never reads `OPENAI_API_KEY` or refresh tokens.

### 2. Build and launch

```powershell
cargo build --release
.\target\release\codexplus.exe
```

The process keeps running in the background; find its icon in the Windows notification area. Open or focus the Codex desktop window to make the overlay appear on its title bar. Right-click the tray icon for the localised menu; choose **Exit** to stop the process and remove the icon.

### 3. Uninstall

Quit CodexPlus, delete the EXE, and clear local state:

```powershell
Remove-Item "$env:LOCALAPPDATA\CodexPlus" -Recurse -Force
```

This does not modify Codex's `auth.json`.

## Settings and proxy

Non-sensitive settings live at:

```text
%LOCALAPPDATA%\CodexPlus\settings.json
```

The file persists the overlay toggle, refresh cadence, proxy mode, custom proxy URL, quota display mode, reset-time display mode, and active UI language. The usage snapshot is stored separately as `usage-cache.json`. The custom proxy URL is plain-text desktop configuration; it surfaces in the Settings window whenever you reopen it.

Persisted fields:

| Field | Purpose |
|---|---|
| `overlay_enabled` | Draw the title-bar overlay |
| `refresh_interval_secs` | 60–3600 second refresh cadence |
| `proxy_mode` / `custom_proxy_url` | Environment / Direct / Custom proxy URL |
| `usage_display` | `remaining` or `used` |
| `reset_time_display` | `countdown` (e.g. `5d14h`) or `absolute` (e.g. `Jul 30 19:34`) |
| `language` | `simplified_chinese` or `english`; missing field is treated as Simplified Chinese |

The settings schema remains v2. Older files that omit the newer fields deserialise successfully and adopt the documented defaults without losing existing values.

### Proxy mode

Pick a mode in **Settings**:

1. **Use environment proxy** — reads the process environment with the priority `CODEXPLUS_HTTPS_PROXY` → `HTTPS_PROXY` → `https_proxy` → `HTTP_PROXY` → `http_proxy` → direct. Empty values are skipped.
2. **Connect directly** — ignore every proxy environment variable and reach the fixed endpoint directly.
3. **Use a custom proxy URL** — type an `http://` or `https://` URL in the address box, save it, and CodexPlus uses it verbatim at runtime. The value is stored in `settings.json` and shown again when you reopen Settings.

Environment proxy example:

```powershell
$env:CODEXPLUS_HTTPS_PROXY = 'http://127.0.0.1:7890'
.\target\release\codexplus.exe
```

Proxy settings only affect the fixed `wham/usage` request. The endpoint, Codex's login state, and the proxy URL / credentials are never written to the diagnostics report or error text.

## Quota state

- The overlay defaults to showing the **remaining** percentage (matching what you can still use). Colour thresholds keep tracking the server-reported `used_percent`: default colour when the remaining quota is ≥ 30% (used < 70%); orange when used ≥ 70%; red when used ≥ 90%; stale renders as grey with `!`. Switching to `used` mode just flips the displayed number (e.g. `Weekly 25%`); the colour thresholds do not change.
- When neither cache nor live data is available, the overlay shows the localised `Codex 用量 · 暂不可用` / `Codex usage · Unavailable` message — a real status, never a fabricated `0%` or `100%`.
- Tray tooltip examples:
  - Simplified Chinese: `CodexPlus / 周限额：剩余 75%，5天14时 / 更新于 12:34`
  - English: `CodexPlus / Weekly: 75% remaining, 5d14h / Updated 12:34`

## Switching language

Open **Settings** → **Display & refresh** → **Language**:

1. Pick the `Simplified Chinese` or `English` radio button.
2. Click **Save**. The current window closes immediately and the host process receives `WM_APP_SETTINGS_CHANGED`.
3. The next overlay update, tray tooltip, right-click menu, and the next Settings window all use the new language.
4. The choice is persisted in `settings.json` and survives restart.

A pre-existing `settings.json` that lacks the `language` field is loaded as Simplified Chinese.

## Target window detection

By default CodexPlus matches Codex-style executable basenames. Override the comma-separated allowlist before launch if your host binary is named differently:

```powershell
$env:CODEXPLUS_TARGET_BIN = 'codex.exe,chatgpt.exe'
.\target\release\codexplus.exe
```

The override affects only the current process. The status strip attaches to the Codex window only when it is visible and not minimised; it hides when the window is covered, minimised, off-screen, or when the available title-bar width is too small. Maximised windows work. Different installers, window classes, themes, or borderless full-screen modes may still affect title-bar placement; if the overlay does not appear, run diagnostics first to confirm the actual foreground exe basename.

## Diagnostics

Run a one-shot probe with:

```powershell
.\target\release\codexplus.exe --diagnose
```

Diagnostics use the same proxy mode as the active Settings. A successful live request refreshes the last-good cache. The report is written to `%TEMP%\codexplus-diagnose-*.log`. After completion a localised MessageBox summarises the result (Simplified Chinese: `状态：… 日志：…`; English: `Status: … Log: …`).

The report records credential file presence, cache load result, **proxy source label**, request outcome, ViewState summary (kept in English with `used` semantics so log search stays stable), and overlay target detection (allowed basenames, foreground exe / class, whether a target matches and is displayable). On a successful request the report lists both windows' actual `limit_window_seconds`; protocol errors such as `WindowDurationMismatch` include the role, actual/expected duration, and the role's allowed bounds, which makes it easier to decide whether the current CodexPlus build should keep accepting that response. When investigating "the status bar never appears", bring the Codex window to the foreground before running diagnostics. The report never contains tokens, account IDs, `Authorization` headers, proxy addresses, proxy usernames / passwords, response bodies, raw API strings, window titles, or full process paths.

## Build and verification

```powershell
cargo fmt
cargo fmt --check
cargo check --all-targets
cargo test --lib
cargo test --release --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

Windows manual acceptance checklist:

1. After launch confirm the process keeps running and the tray icon is present.
2. Open the right-click menu and confirm it matches the active language (Simplified Chinese or English).
3. Open a Codex window and confirm the title-bar overlay shows the current language; in English it should look like `5h / Weekly xx% xxdxxh`, in Simplified Chinese like `5小时 / 周限额 xx% xx天xx时`. Clicks on the title bar must not activate the overlay.
4. Cover the Codex window with another window and confirm the overlay is covered together with it; when Codex returns to the foreground the overlay returns automatically.
5. Change refresh cadence, proxy mode, quota display, reset-time display, and language; exit and restart to confirm every choice persists. Validate environment, direct, and custom proxy modes. Inspect the diagnostics log to confirm it only carries safe proxy labels.
6. In Settings flip the language to English and click Save; reopen Settings to confirm the title, controls, subtitle, Save, and Cancel are all in English and the overlay / tray tooltip / menu adopt English immediately.
7. Choose **Exit** and confirm the tray icon disappears and the process ends.

## Safety boundary

| Data | Handling |
|---|---|
| `auth.json` access token | Used solely for the `Authorization` header; never cached, logged, or shown in errors |
| Account ID | Used only as a request header; never persisted |
| Custom proxy URL | Stored as desktop configuration in `settings.json`, editable at any time; never logged, cached, or included in the diagnostics report |
| API response body | Never stored |
| Endpoint | Fixed at `https://chatgpt.com/backend-api/wham/usage`; no custom base URL, no redirect following |
| Cache schema | v2 with v1 compatibility; unrecognised snapshots are discarded and never written as wrong data |
| Settings schema | v2; older v1 files still load but values are folded back to safe defaults according to `schema_version`; new fields such as `language`, `usage_display`, and `reset_time_display` fall back to defaults instead of resetting other entries |

## Scope

- No auto-update, login-item, or multi-account switching.
- No browser scraping, cookie reading, or `codex exec` invocation.
- `wham/usage` is an internal endpoint; on schema drift the overlay reports a protocol error rather than falling back to fabricated percentages.
- Overlay placement and visibility are coordinated on a short timer rather than through a finer-grained WinEventHook.
- User-visible UI is restricted to Simplified Chinese and English; no runtime i18n framework is introduced.

## License

MIT License. Independent reimplementation; not affiliated with or endorsed by `@openai/codex`.
