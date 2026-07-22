# CodexPlus 常驻应用核心收尾

## Context

当前工作树已将一次性 MVP 升级为常驻 Win32 应用：隐藏 host window、标题栏 overlay、托盘菜单、后台轮询、持久化设置和自定义代理均已实现。剩余目标是让诊断行为与当前设置一致、修复明确的资源问题、补足可确定性测试，并将 README 从旧 MVP 说明更新为实际产品行为；不扩展到 WinEventHook、全屏识别或新的托盘交互。

## 实施步骤

1. **统一诊断与运行时的代理策略**
   - 修改 `src/diagnose.rs`，加载 `src/settings.rs::Settings` 并通过 `Settings::proxy_policy()` 解析 `ProxyConfiguration`，替代仅从环境变量解析的路径。
   - 保持诊断报告只输出安全的代理来源标签，绝不写入代理地址、凭据、token、账号、请求头或响应体。
   - 复用 `provider::ProxyConfiguration` 与现有 `format_proxy_mode`，让 Tray 的 Diagnose 与后台轮询走同一策略。

2. **修复设置窗口创建失败的资源释放**
   - 修改 `src/ui/settings.rs::open`：若 `CreateWindowExW` 创建失败，立即回收传递给窗口过程的 `SettingsWindowData`；成功时继续由 `WM_DESTROY` 释放。
   - 不改变保存、自定义代理 Credential Manager 存储或 `WM_APP_SETTINGS_CHANGED` 的现有语义。

3. **补充纯逻辑回归测试**
   - 扩展 `src/settings.rs` 的测试，覆盖 schema 不兼容回退、从 Custom 切换至 Environment/Direct 时清除凭据目标元数据，以及序列化不含代理地址或凭据。
   - 在 `src/provider/codex.rs` 保持/补充 policy 解析的安全标签和 Direct/Custom 行为测试；不在单元测试中调用 Windows Credential Manager 或真实网络。
   - 必要时在 `src/ui/app_state.rs` 为刷新调度和手动刷新合并补充确定性测试。

4. **重写 README 以匹配实现**
   - 更新 `README.md` 的启动、使用、代理、诊断、项目结构、限制和测试章节。
   - 准确描述常驻进程、标题栏 overlay、通知区域图标与右键菜单（Refresh、overlay 开关、Settings、Diagnose、Exit）、默认 300 秒及 60–3600 秒刷新范围。
   - 说明 `%LOCALAPPDATA%\CodexPlus\settings.json` 与 `usage-cache.json` 的职责分离；自定义代理 URL 存入 Windows Credential Manager，JSON 不存 URL。
   - 更新目标识别覆盖方式 `CODEXPLUS_TARGET_BIN`、诊断 TEMP 日志及隐私边界；移除“启动后退出”“没有图标”“Host/Tray 未实现”、旧测试数量与未重新测量的体积等过时断言。
   - 保留真实剩余限制：未做自动更新、开机启动、多账号；目标窗口识别仍可通过环境变量覆盖；WinEventHook/细粒度窗口事件响应不在本次范围。

5. **验证**
   - 自动验证：`cargo fmt --all -- --check`、`cargo check --all-targets`、`cargo test --lib`、`cargo test --release --lib`、`cargo build --release`、`git diff --check`。
   - 运行严格 Clippy 并区分既有 lint 基线与本次引入项；不以既有失败误报为本次失败。
   - 提供 Windows 手动验收清单：启动后托盘驻留、右键菜单和 Exit、前台窗口 overlay 显隐、设置跨重启持久化、三种代理模式、诊断日志策略标签及敏感内容脱敏。

## 关键文件

- `src/diagnose.rs`
- `src/settings.rs`
- `src/ui/settings.rs`
- `src/provider/codex.rs`
- `src/ui/app_state.rs`
- `README.md`
