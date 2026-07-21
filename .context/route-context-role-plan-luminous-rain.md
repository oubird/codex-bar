# 为 CodexPlus 增加 HTTP/HTTPS 代理支持

## Context

当前 CodexPlus 从本地 `%USERPROFILE%\.codex\auth.json` 读取 Codex OAuth 凭据，但 `src/provider/codex.rs::UreqTransport::new()` 创建的 ureq 客户端直连 `https://chatgpt.com/backend-api/wham/usage`。本机直连该地址超时，而经 `http://127.0.0.1:7890` 可达，因此 `--diagnose` 已确认凭据解析成功、实时限额请求却报 `Network is unavailable`。本次只补代理接入，不改变固定 endpoint、认证头、窗口/UI 规划或本地凭据策略。

## 目标与完成定义

- 常规后台轮询与 `--diagnose` 使用一致的代理解析和 HTTP transport。
- 通过 `CODEXPLUS_HTTPS_PROXY` 支持应用显式代理覆盖，并兼容标准环境变量。
- 环境变量优先级固定为：
  1. `CODEXPLUS_HTTPS_PROXY`
  2. `HTTPS_PROXY`
  3. `https_proxy`
  4. `HTTP_PROXY`
  5. `http_proxy`
  6. 无变量时直连。
- 允许目标机器已验证可用的 `http://127.0.0.1:7890` 作为 HTTPS 请求代理。
- 代理 URL、用户名、密码、Bearer token、account ID 不得进入诊断日志、UI 文案、`AppError::Display` 或缓存。
- 保留 `.redirects(0)`、固定 `wham/usage` endpoint、现有超时/响应大小限制和 3xx → `RedirectRejected` 行为。
- 构建、测试通过；使用本机代理运行 `--diagnose` 能获得 API 成功或服务端的正常认证结果，而不是网络超时。

## 改动边界

### 修改

- `src/provider/codex.rs`
- `src/provider/mod.rs`
- `src/error.rs`
- `src/ui/poll.rs`
- `src/diagnose.rs`
- `README.md`
- 必要时更新内联单元测试（位于 `src/provider/codex.rs`）。

### 禁止修改

- `references/Win-CodexBar/**`
- `references/Claude-Code-Usage-Monitor/**`
- `references/codex-usage-monitor/**`
- `%USERPROFILE%\.codex\auth.json` / `%CODEX_HOME%\auth.json`
- 固定请求 endpoint `https://chatgpt.com/backend-api/wham/usage`

不增加代理 URL 的持久化配置文件；只读取进程环境变量。不得把代理值写入 `%LOCALAPPDATA%\CodexPlus\usage-cache.json` 或诊断日志。

## 现有实现与复用点

- `src/provider/codex.rs::UreqTransport::new()`：现有 ureq + native-tls、10 秒连接超时、20 秒读取超时和 `.redirects(0)` 的唯一生产 transport 构造点。
- `src/provider/codex.rs::ProviderTransport`：已有 fake transport 测试抽象；代理解析与 transport 构建的测试应复用这一模块的内联 `#[cfg(test)]` 模式，不依赖真实网络。
- `src/ui/poll.rs::run_poll()`：常规轮询构造 `UreqTransport` 的调用点。
- `src/diagnose.rs::report_live_request()`：诊断 live probe 构造 `UreqTransport` 的调用点；需要与常规轮询共享同一 factory。
- `src/error.rs::AppError::public_message()`：诊断和 UI 已通过它输出错误；代理配置错误应新增无敏感负载变体。
- `src/diagnose.rs`：日志当前已安全输出认证文件元数据和请求结果；只新增代理**来源标签**，不输出变量值。

## 推荐设计

### 1. provider 内集中解析代理配置

在 `src/provider/codex.rs` 定义：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxySource {
    Direct,
    CodexplusHttpsProxy,
    HttpsProxy,
    HttpProxy,
}

pub struct ProxyConfiguration {
    source: ProxySource,
    proxy: Option<ureq::Proxy>,
}
```

`ProxyConfiguration` 不派生 `Debug`，不暴露原始 URL 或 `ureq::Proxy`。仅公开：

```rust
pub fn from_environment() -> Result<Self, AppError>;
pub fn source(&self) -> ProxySource;
```

实现一个私有的依赖注入解析函数，例如：

```rust
fn from_env_lookup(
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<ProxyConfiguration, AppError>;
```

生产实现传入 `std::env::var_os`；测试用 map/闭包注入变量，避免并行测试改变进程环境。

各变量去首尾空白后为空时继续尝试下一优先级；代理 URL 由 `ureq::Proxy::new` 解析。解析失败或非 Unicode 值返回新的 `AppError::ProxyConfigurationInvalid`，该变体不携带原始值。

### 2. 将 UreqTransport 变成可失败构造

将：

```rust
UreqTransport::new() -> Self
```

改为：

```rust
UreqTransport::new() -> Result<Self, AppError>;
UreqTransport::with_proxy(configuration: ProxyConfiguration) -> Result<Self, AppError>;
pub fn proxy_source(&self) -> ProxySource;
```

构造时保留：

```rust
.timeout_connect(CONNECT_TIMEOUT)
.timeout_read(READ_TIMEOUT)
.redirects(0)
.tls_connector(...)
```

仅当 `configuration.proxy` 为 `Some` 时使用 `ureq::AgentBuilder::proxy(proxy)`。代理只改变 TCP/TLS 连接路径，不允许变更请求 endpoint、认证 header 或重定向策略。

### 3. 收紧网络错误负载

把：

```rust
AppError::Network(String)
```

改为无负载：

```rust
AppError::Network
AppError::ProxyConfigurationInvalid
```

当前 transport 会把 `ureq::Error::Transport(t).to_string()` 传入 `Network(String)`。未来该字符串可能包含代理主机或认证信息，故不能保留。更新 `retry_class()` 和 `public_message()`：

- `Network` → 原有 `Network` retry class、`Network is unavailable.`
- `ProxyConfigurationInvalid` → `Network` retry class、`Configured HTTP proxy is invalid.`

### 4. 统一常规轮询与诊断

- `src/ui/poll.rs::run_poll()`：处理 `UreqTransport::new()` 的 `Result`；构造失败时返回 `PollOutcome::Failure(error)`，不 panic。
- `src/diagnose.rs::report_live_request()`：先解析 `ProxyConfiguration`，报告仅输出：
  - `Proxy mode : direct`
  - `Proxy mode : CODEXPLUS_HTTPS_PROXY`
  - `Proxy mode : HTTPS_PROXY`
  - `Proxy mode : HTTP_PROXY`
  - 或 `Proxy mode : invalid configuration`
- 再用同一个 `UreqTransport::with_proxy(configuration)` 发请求，确保诊断显示的来源与实际连接方式一致。
- 不显示代理 URL、host、port、用户名、密码、环境变量值、Header、token 或响应 body。

### 5. README 文档

在 README 的“安全边界”或“快速开始”中增加“网络代理”小节：

```powershell
# 本应用显式覆盖（优先级最高）
$env:CODEXPLUS_HTTPS_PROXY = 'http://127.0.0.1:7890'

# 或使用标准变量
$env:HTTPS_PROXY = 'http://127.0.0.1:7890'
```

注明变量仅在启动该进程的环境中生效；`--diagnose` 会显示代理**来源**，但不会记录代理地址。不得写入含代理地址的示例到程序日志。

## 实施步骤

1. **实现代理来源模型与环境变量解析**
   - 输入：标准环境变量和应用显式覆盖变量。
   - 动作：在 `provider/codex.rs` 添加 `ProxySource`、私有 `ProxyConfiguration` 和按优先级解析函数。
   - 输出：能区分 direct / 专用变量 / HTTPS / HTTP fallback，且不保存或公开原始 URL。

2. **接入 ureq AgentBuilder**
   - 输入：已校验 `ProxyConfiguration`。
   - 动作：将 `UreqTransport::new` 改为 `Result`，在 agent builder 上条件应用 `.proxy(...)`，保留 `redirects(0)` 与现有超时/TLS。
   - 输出：代理构建失败变成安全 `AppError`，成功 transport 可返回 `ProxySource`。

3. **更新错误类型与调用点**
   - 输入：新 transport constructor。
   - 动作：修改 `AppError`，删除 `Network(String)` 负载；更新 `poll.rs::run_poll()` 与 `diagnose.rs::report_live_request()` 的构造和错误处理。
   - 输出：常规 worker 和 diagnose 均使用相同代理策略，配置错误不会 panic。

4. **补诊断代理来源输出**
   - 输入：`ProxySource`。
   - 动作：抽出纯函数 `format_proxy_mode(ProxySource) -> &'static str`；在 live request 段落输出来源标签。
   - 输出：日志能判断是否走代理，但不泄漏 URL/凭据。

5. **补单元测试和文档**
   - 输入：可注入 environment lookup。
   - 动作：测试优先级、空变量、`http://127.0.0.1:7890` 接受、无效 URL、安全错误文案、diagnostic source label；更新 README。
   - 输出：不依赖真实网络的确定性单元测试与用户配置说明。

6. **本机验证并更新安装目录**
   - 输入：本机已验证可达的 `http://127.0.0.1:7890`。
   - 动作：运行 `cargo fmt --all -- --check`、`cargo test --lib`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo build --release`；设置 `CODEXPLUS_HTTPS_PROXY=http://127.0.0.1:7890`，执行 `target\release\codexplus.exe --diagnose`，读取 `%TEMP%` 最新日志；成功后复制新的 exe 到 `D:\Program Files\codexplus-windows-x64\`。
   - 输出：日志显示 `Proxy mode : CODEXPLUS_HTTPS_PROXY`，live request 不再报告直连超时；日志不含 URL/token/account ID。

## 必跑命令

```powershell
cargo fmt --all -- --check
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release

$env:CODEXPLUS_HTTPS_PROXY = 'http://127.0.0.1:7890'
.\target\release\codexplus.exe --diagnose
```

检查日志（使用最新生成的 `%TEMP%\codexplus-diagnose-*.log`）：

- 有 `Proxy mode : CODEXPLUS_HTTPS_PROXY`；
- 无 `127.0.0.1:7890`、Bearer token、account_id、代理用户名或密码；
- live request 成功，或至少到达服务端返回正常 401/403，而不是网络超时。

## 验收标准

1. 在本机设置 `CODEXPLUS_HTTPS_PROXY=http://127.0.0.1:7890` 后，`--diagnose` 通过代理请求固定 `wham/usage` endpoint。
2. 未配置变量时维持现有直连行为。
3. 专用变量覆盖标准变量；HTTPS 变量覆盖 HTTP fallback；空变量跳过。
4. 无效代理配置不 panic，返回固定安全消息。
5. `--diagnose` 仅显示代理来源标签，日志/错误不包含原始代理 URL、认证信息、Bearer token 或 account ID。
6. 3xx 仍被拒绝，不能因代理而跟随重定向。
7. 所有测试、clippy 和 release build 通过。

## 失败处理

- 代理地址错误：报告 `Configured HTTP proxy is invalid.`；不尝试直连回退（避免用户显式要求代理却让 token 走不同路径）。
- 代理连接失败：报告 `Network is unavailable.`；依现有网络退避策略重试。
- 服务端认证失败：报告既有 `codex login` 安全提示；不自动执行 Codex CLI。
- 本机代理未运行：不改 endpoint、不写配置、不静默降级直连；通过诊断日志明确显示网络失败。

## 最终回报格式

1. 列出修改文件和代理优先级。
2. 报告本机 `--diagnose` 的安全摘要：凭据是否解析、代理来源、请求结果；绝不粘贴 token/account ID/代理 URL。
3. 列出 fmt/test/clippy/build 结果与新 EXE 安装路径。
4. 明确说明未完成项或仍受内部 `wham/usage` API 变更影响的限制。
