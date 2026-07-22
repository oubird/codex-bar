# CodexPlus

> 常驻在 Windows 通知区域、在前台 Codex 窗口标题栏显示 **5 小时与周限额** 的轻量本地工具。

| 项目 | 内容 |
|---|---|
| 平台 | Windows 10 / 11（x64） |
| 形态 | 单 EXE、原生 Win32；无 WebView2、Node.js、Python 或 PowerShell 运行时 |
| 数据 | 只读 Codex CLI 的 `auth.json`；仅请求固定 `wham/usage` endpoint |
| 隐私 | Token、账号、代理 URL 与代理凭据不写入缓存、诊断报告或 UI |

## 功能

- **标题栏 overlay**：当前台窗口被识别为 Codex 时，在标题栏右侧显示 `5h 42% · 1h18m   week 67% · 3d4h`。
- **自动隐藏**：窗口不在前台、最小化、最大化、不可见或标题栏空间不足时隐藏；overlay 鼠标穿透，不抢焦点、不出现在任务栏或 Alt+Tab。
- **托盘常驻**：启动后保留通知区域图标。右键菜单提供 Refresh now、Show title-bar overlay、Settings、Run diagnose 和 Exit。
- **后台刷新与缓存**：默认每 300 秒请求一次；网络或协议异常时保留最近成功快照并标记为 stale，认证问题会降低刷新频率。
- **设置窗口**：可开关 overlay、设置 60–3600 秒刷新周期，以及选择环境代理、强制直连或自定义代理。

## 快速开始

### 1. 登录 Codex CLI

```powershell
npm install -g @openai/codex
codex login
Test-Path $env:USERPROFILE\.codex\auth.json
```

`auth.json` 也可位于 `%CODEX_HOME%\auth.json`。CodexPlus 只读取其中的 access token，不会修改它，也不读取 `OPENAI_API_KEY` 或 refresh token。

### 2. 构建并启动

```powershell
cargo build --release
.\target\release\codexplus.exe
```

进程会在后台常驻；请在 Windows 通知区域找到图标。打开或切换到 Codex 桌面窗口时，符合条件的标题栏会显示 overlay。右键托盘图标打开菜单；选择 **Exit** 才会停止进程并移除图标。

### 3. 卸载

退出 CodexPlus 后删除 EXE；如需清除本地设置和快照：

```powershell
Remove-Item "$env:LOCALAPPDATA\CodexPlus" -Recurse -Force
```

这不会修改 Codex 的 `auth.json`。如果使用过自定义代理，其 URL 存在 Windows Credential Manager；可在其中删除 `CodexPlus.ProxyUrl.v1`。

## 设置与网络代理

非敏感设置保存在：

```text
%LOCALAPPDATA%\CodexPlus\settings.json
```

该文件保存 overlay 开关、刷新周期和代理模式。用量快照单独保存至 `usage-cache.json`。自定义代理 URL（可包含用户名和密码）不会写入 JSON，而是使用 Windows Credential Manager 中的 `CodexPlus.ProxyUrl.v1` 凭据。

### 代理模式

在 **Settings** 中选择：

1. **Use environment proxy**：读取启动进程的环境变量，优先级为 `CODEXPLUS_HTTPS_PROXY` → `HTTPS_PROXY` → `https_proxy` → `HTTP_PROXY` → `http_proxy` → 直连。空值会跳过。
2. **Force direct connection**：忽略所有代理环境变量，直接连接固定 endpoint。
3. **Use custom proxy address**：保存 `http://` 或 `https://` 代理地址到 Credential Manager，之后不再回显地址。

环境代理示例：

```powershell
$env:CODEXPLUS_HTTPS_PROXY = 'http://127.0.0.1:7890'
.\target\release\codexplus.exe
```

代理仅影响固定的 `wham/usage` 请求；不会改变 endpoint、不会修改 Codex 登录状态，也不会把地址或凭据写入缓存。

## 用量状态

| 状态 | 行为 |
|---|---|
| 正常 | 根据当前两个窗口中较高的已用比例显示绿 / 橙 / 红色文字 |
| 已用 ≥ 70% | 橙色 |
| 已用 ≥ 90% | 红色 |
| stale | 灰色并带 `!`，继续显示 last-good 数值 |
| 无实时数据且无缓存 | 不显示伪造数值 |

## 目标窗口识别

默认会匹配 Codex 相关的可执行文件名；如果你的桌面应用 host 名称不同，可在启动前设置逗号分隔的 basename 覆盖：

```powershell
$env:CODEXPLUS_TARGET_BIN = 'codex.exe,chatgpt.exe'
.\target\release\codexplus.exe
```

这项覆盖只影响当前启动进程。不同安装包、窗口类、主题或无边框全屏模式可能影响标题栏定位；如未匹配，可先通过该变量确认实际 host 名称。

## 诊断

使用 `--diagnose` 执行一次凭据、缓存和实时请求检查：

```powershell
.\target\release\codexplus.exe --diagnose
```

诊断使用与当前 Settings 相同的代理模式，成功的实时请求会更新 last-good 缓存，并将报告写入 `%TEMP%\codexplus-diagnose-*.log`。结束后会显示报告路径。

报告会记录凭据文件存在性、缓存读取结果、代理**来源标签**、请求结果和 ViewState 摘要；不会记录 token、账号 ID、Authorization header、代理地址、代理用户名/密码、响应 body 或 API 原文。

## 构建与验证

```powershell
cargo fmt --all -- --check
cargo check --all-targets
cargo test --lib
cargo test --release --lib
cargo build --release
```

Windows 手动验收建议：

1. 启动后确认进程持续运行且托盘图标存在。
2. 检查右键菜单的 Refresh、overlay 开关、Settings、Diagnose 和 Exit。
3. 在 Codex 与其它应用之间切换，验证 overlay 只在符合条件的前台窗口出现。
4. 修改刷新周期和代理模式，退出并重新启动，确认设置仍生效。
5. 分别验证环境、直连和自定义代理；检查诊断日志只含安全代理标签。
6. 使用 Exit，确认托盘图标消失且进程结束。

## 安全边界

| 数据 | 处理 |
|---|---|
| `auth.json` access token | 只用于请求 Authorization header；不进缓存、日志或错误文案 |
| account ID | 仅用于请求头；不落本地 |
| 自定义代理 URL | 仅保存在 Windows Credential Manager；不进 settings、缓存或诊断日志 |
| API 响应原文 | 不保存 |
| Endpoint | 固定为 `https://chatgpt.com/backend-api/wham/usage`；不读取自定义 base URL，不跟随重定向 |

## 当前范围

- 不提供自动更新、开机启动或多 Codex 账号切换。
- 不抓浏览器页面、不读 cookie、不调用 `codex exec`。
- `wham/usage` 是内部接口；schema 变化时会显示协议错误，不会回退为伪造的 0% 或 100%。
- Overlay 的位置与显隐会周期性协调；未接入更细粒度的 WinEventHook 监听。

## 许可证

MIT License。本项目独立实现，和 `@openai/codex` 的协议无关。
