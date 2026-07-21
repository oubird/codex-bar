# CodexPlus

> 一条永远停在 Codex 桌面应用标题栏右侧、用最克制的方式告诉你
> **5 小时与周限额**还剩多少的极简 Windows 小插件。

| 项目 | 内容 |
|---|---|
| 平台 | Windows 10 / 11（x64） |
| 形态 | 单 EXE，无 WebView2、Node.js、Python 或 PowerShell 依赖 |
| 数据 | 直接读取 Codex CLI 已登录的 `auth.json`，只向 `wham/usage` 端点发 1 个请求 |
| 体积 | ~476 KB（`target\release\codexplus.exe`） |

---

## 为什么做这个

你想知道"刚才用掉多少比例、什么时候重置"，但又懒得切窗口去刷网页。
现有的几种方案要么占空间（系统托盘大窗口），要么协议脆弱（抓页面文本），
要么强制你再登录一次。这个插件选择最克制的形式：

> 在你已经打开 Codex 桌面应用的时候，**左上角右侧空白区**贴一条
> 半透明紧凑条。鼠标完全穿透 —— 你像平时一样工作，它安静地告诉你真相。

最大化、全屏、最小化、被其它窗口挡住时它会主动消失，绝不打扰。

---

## 用它做什么 / 不做什么

| | 行为 |
|---|---|
| ✅ 读你的 Codex CLI OAuth 凭据 | 从 `%USERPROFILE%\.codex\auth.json`（或 `%CODEX_HOME%\auth.json`）只读 access token |
| ✅ 显示 | 普通状态、当前活动 Codex 窗口的标题栏右侧：`5h 42% · 1h18m   week 67% · 3d4h` |
| ✅ 悬停 tooltip | 整段详情 + 最后更新时刻 + stale/error 状态提示 |
| ✅ 缓存 | 上次成功快照保存到 `%LOCALAPPDATA%\CodexPlus\usage-cache.json`，离线时显示 stale 值 |
| ❌ 抓网页 | 不调用浏览器、不读 cookie、没有任何依赖互联网子进程 |
| ❌ 改配置 | 不写 Codex `auth.json`、不读 `OPENAI_API_KEY`、不读 `refresh_token`、不调用 `codex exec` |
| ❌ 抢焦点 | 鼠标操作全部透传给 Codex 窗口；不出现在任务栏 / Alt+Tab |

---

## 快速开始

### 1. 安装 / 登录 Codex

```powershell
npm install -g @openai/codex   # 任何一种安装 Codex CLI 的方式
codex login                    # 浏览器登录后回车
```

确认 `%USERPROFILE%\.codex\auth.json` 已存在：

```powershell
Test-Path $env:USERPROFILE\.codex\auth.json   # True
```

### 2. 启动插件

双击 `target\release\codexplus.exe`。
**它没有窗口、没有控制台、没有图标** —— 它在工作线程里先发一次预热请求，
然后退出当前进程（这是 MVP 形态，详见下面的"当前限制"）。

> 真正长时间常驻的版本会在下一轮构建中追加 —— 你随时可以再
> `cargo build --release` 一次拿到最新版本。

### 3. 卸载

```powershell
Remove-Item "$env:LOCALAPPDATA\CodexPlus" -Recurse -Force
Remove-Item ".\codexplus.exe"
```

（当前 MVP 不会写入注册表。Codex `auth.json` 也保持只读。）

---

## 构建 / 测试

仅依赖稳定版 Rust + Windows。

```powershell
# 一次性的预热
rustup toolchain install stable
rustup target add x86_64-pc-windows-msvc

# 构建 release
cargo build --release
.\target\release\codexplus.exe

# 跑所有单元测试
cargo test --lib

# release 模式再跑一次（验证 LTO、strip 后还正常）
cargo test --release --lib
```

`cargo build --release` 完成后，当前已通过 45 个单元测试：

```
test result: ok. 45 passed; 0 failed; 0 ignored
```

---

## 网络代理

如果你的网络需要本机或企业 HTTP 代理才能访问 ChatGPT/Codex 后端，请在启动前设置代理环境变量。代理只影响到固定的 `wham/usage` 请求；不会改变 endpoint、不会写入缓存，也不会修改 Codex 登录态。

```powershell
# 本应用专用覆盖，优先级最高（当前机器的可用示例）
$env:CODEXPLUS_HTTPS_PROXY = 'http://127.0.0.1:7890'
.\target\release\codexplus.exe --diagnose

# 也兼容标准 HTTPS 代理变量
$env:HTTPS_PROXY = 'http://127.0.0.1:7890'
.\target\release\codexplus.exe --diagnose
```

代理变量优先级：`CODEXPLUS_HTTPS_PROXY` → `HTTPS_PROXY` → `https_proxy` → `HTTP_PROXY` → `http_proxy` → 直连。空值会被跳过。`--diagnose` 日志只显示代理**来源标签**（如 `CODEXPLUS_HTTPS_PROXY`），不会记录代理地址、用户名、密码、Bearer token 或账号 ID。

## 形态说明

| 状态 | 视觉效果 |
|---|---|
| 普通窗口、前台、最大化/全屏外 | 紧凑条贴右边，颜色随最差指标变化：绿 / 橙 / 红 |
| 标题栏右侧空间不足（< 280 逻辑像素） | 自动隐藏，不缩成碎片 |
| Codex 最大化 / 真正全屏 / 最小化 / 不可见 / 被覆盖 | 250ms 内完全隐藏 |
| API 失败且无 last-good | 不显示任何伪造数字 |
| API 失败但有 last-good | 仍然显示数字 + stale 标记 `!` |

颜色阈值：

```
< 70%  : 绿色
≥ 70%  : 橙色
≥ 90%  : 红色
stale  : 灰色 + 感叹号
```

---

## 安全边界

| 私密数据 | 处理 |
|---|---|
| `auth.json` access token | 仅在请求的 Authorization 头内使用，进程退出即释放；永不入缓存、永不入日志 |
| account ID | 仅在请求头内传 `ChatGPT-Account-Id`，不落本地 |
| API 响应原文 | 不保存 |
| Endpoint | 编译时固定 `https://chatgpt.com/backend-api/wham/usage`，不可配置、不做重定向、读取不到 `config.toml` 里的 `chatgpt_base_url` |
| Token 调试输出 | `SecretString::Debug → "[REDACTED]"`；无 `Display` |
| 错误文案 | 仅枚举状态语；不附带 token、Header 或服务端原文 |

缓存目录 `%LOCALAPPDATA%\CodexPlus\` 只能保存 `usage-cache.json`，其内容只有
`{ schema_version, five_hour:{..}, weekly:{..}, fetched_at_unix }`。

---

## 项目结构

```text
codexplus/
├─ Cargo.toml                # Rust 工程 + Win32 features
├─ Cargo.lock
├─ build.rs                  # winres 嵌入资源
├─ README.md
├─ .gitignore
├─ assets/
│  └─ app.ico                # Windows 资源图标（可选）
└─ src/
   ├─ lib.rs                 # 顶层模块声明
   ├─ main.rs                # `#![windows_subsystem = "windows"]` 入口
   ├─ app.rs                 # run() 调度器
   ├─ model.rs               # WindowKind / RateWindow / UsageSnapshot / ViewState
   ├─ error.rs               # AppError + RetryClass（不泄漏 token）
   ├─ formatting.rs          # 标题栏行 + 托盘 tooltip + 倒计时变更点
   ├─ credentials.rs         # Codex CLI auth.json 只读解析 + SecretString
   ├─ storage.rs             # atomic last-good 缓存
   ├─ provider/
   │  ├─ mod.rs
   │  └─ codex.rs            # /wham/usage 客户端（严格窗口验证 + 禁重定向）
   └─ ui/
      ├─ mod.rs
      ├─ native.rs           # Win32 通用 glue + run_app()
      ├─ target_window.rs    # 识别 Codex 桌面应用、排除终端
      ├─ overlay.rs          # 标题栏 popup（host 抽象 + 坐标算法）
      ├─ render.rs           # 文本与色调映射
      ├─ poll.rs             # 后台刷新 worker + 退避分类
      ├─ tray.rs             # 托盘文本组装
      └─ tests.rs            # 跨模块集成断言

references/                  # 三个外部参考项目，git 仓库但只读
├─ Win-CodexBar/
├─ Claude-Code-Usage-Monitor/
└─ codex-usage-monitor/
```

`references/` 在 `.gitignore` / 提交上保持只读：实现期发现需要看
任意一个的代码片段，应该拷到主仓库后用自己的方式重写，而不是直接 fork。

---

## 当前限制（MVP 已知项）

- **真实 Win32 顶层窗口与 `SetWinEventHook` 还没写**：
  `src/ui/native.rs::run_app()` 内部触发一次后台 refresh 然后 `Ok(())`
  返回。生产代码需要：
  1. 在 `native.rs` 创建不可见的顶层 host window；
  2. 安装 `SetWinEventHook(EVENT_SYSTEM_FOREGROUND | EVENT_OBJECT_LOCATIONCHANGE | EVENT_SYSTEM_MINIMIZESTART | EVENT_OBJECT_HIDE, ...)`，
     回调只投递消息给 UI 线程；
  3. 写真正的 `Shell_NotifyIconW` 托盘图标。
  抽象已经在 `OverlayHandle` 里准备好；hook 的样式模板可参考
  `references/Claude-Code-Usage-Monitor/src/native_interop.rs::set_tray_event_hook`
  和 `::embed_in_taskbar` 的 `WS_EX_*` 标志。
- **Codex 桌面应用的真实可执行文件名/窗口类还没在你的机器上验证**：
  默认白名单 `["codex", "codex desktop"]` 通过环境变量 `CODEXPLUS_TARGET_BIN`
  覆盖，逗号分隔、可添加 `.exe`。后续需要在你打开 Codex 时
  调用一次 `target_window::diagnostics()` 导出实际身份，再固化到默认值。
- **没有自动更新 / 开机启动 / 多 Codex 账号切换**。
- **没有离线编辑器集成**：仅支持 Codex CLI 已经写的 OAuth token。

---

## FAQ

**为什么不用现成的 Win-CodexBar / Claude-Code-Usage-Monitor？**
它们已经做得很好。此项目是给"想要最小化、不抢屏、纯本地"的用户的
另一种形态选择。`references/` 里下载了它们的源码用来读设计，不会原样
拼装。

**会不会写坏我的 Codex 登录态？**
不会。`credentials.rs` 只读取 `auth.json`，并且没有 `OPENAI_API_KEY`
的回退分支；不调用任何会改动登录态的 Codex 命令。

**断网后会怎样？**
- 启动：有 last-good 时显示 cached 数据 + `!`；无 last-good 时
  工具栏不显示伪造的数字。
- 运行中：刷新请求失败按 30s → 2min → 5min 退避，恢复后立即回到 5min 节拍。

**接口是公开的吗？**
不。`/backend-api/wham/usage` 是 ChatGPT 内部端点。
provider 严格校验响应，schema 变化时直接报 `ProtocolChange` 并在界面
显示"用量接口格式已变化，等待更新"，**绝不**回退到默认 `0%` 或
`100%`。

---

## 验证命令速查

```powershell
cargo fmt --all -- --check
cargo check --all-targets
cargo test --lib                                # 45 个单元测试
cargo test --release --lib
cargo build --release                           # 产出 ~476 KB EXE
```

---

## 许可证

MIT License。和 Codex 项目使用的 `@openai/codex` 协议无关 —— 本项目
独立实现并运行。

