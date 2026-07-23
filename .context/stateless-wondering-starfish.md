# Overlay 可见状态与诊断收尾

## Context

用户已在实际 Codex 窗口中启动 CodexPlus，但没有看到任何 overlay。截图中的窗口处于最大化状态；当前实现同时会在“初次请求失败且没有缓存”时隐藏 overlay，并主动排除最大化窗口。这与用户期望冲突：只要识别到当前 Codex 窗口，就应展示一个明确的状态条，即使尚无可用额度数据也不能消失。

## 实施步骤

1. **保持无数据状态可见**
   - 修改 `src/model.rs::ViewState::is_displayable`，让 `Error { cached: None, .. }` 继续可绘制。
   - 复用现有 `src/ui/render.rs::build_text` / `RenderedRow`，为无缓存的 Loading 和 Error 分别显示稳定的非数值状态文本，例如 `Codex usage · loading…` 与 `Codex usage · unavailable`；不显示伪造的额度数字，也不在标题栏展示敏感网络细节。
   - 保持缓存存在时的 Ready/Stale 行为不变。

2. **允许最大化 Codex 窗口显示状态条**
   - 调整 `src/ui/target_window.rs` 和 `src/ui/overlay.rs` 的资格判定，最大化不再作为硬性隐藏条件；仍在最小化、不可见、非前台、窗口无有效尺寸、overlay 被关闭或标题栏空间不足时隐藏。
   - 复用现有定位逻辑；必要时仅根据实际客户端区域/标题栏几何收紧位置，避免覆盖窗口控制按钮。

3. **让诊断解释“为什么没有显示”**
   - 扩展 `src/diagnose.rs`，调用已有 `target_window` 枚举/前台识别能力，输出安全的目标识别摘要：有效 basename 规则、是否识别到前台窗口、窗口可见/最小化/最大化状态，以及最终可显示结论。
   - 不记录 token、账号、请求头、代理地址/凭据或 API body；窗口标题和路径只保留必要的脱敏/截断信息，便于用户分享报告。

4. **回归测试与文档**
   - 更新 `src/model.rs`、`src/ui/render.rs`、`src/ui/target_window.rs` 的纯逻辑测试：无缓存 Error 可显示且只渲染状态文本；最大化状态允许 overlay；最小化/不可见继续拒绝。
   - 更新 `README.md`：说明空状态条不是额度值；最大化窗口可以显示；列出仍会隐藏的实际条件，并补充 `--diagnose` 的目标识别排查步骤与 `CODEXPLUS_TARGET_BIN` 覆盖方式。

5. **验证**
   - 运行 `cargo fmt --all -- --check`、`cargo check --all-targets`、`cargo test --lib`、`cargo test --release --lib`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo build --release`、`git diff --check`。
   - Windows 手动验证：无 auth/无 cache 时将最大化 Codex 窗口置前，确认看到空状态条；恢复缓存或有效凭据后确认额度行替换状态条；最小化或切走前台时确认隐藏；运行诊断确认识别结论可用于排障。
