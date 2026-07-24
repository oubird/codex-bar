# CodexPlus · 仓库简介

Windows 单 EXE 小插件，在 Codex 桌面应用标题栏最小化按钮左侧贴一条
鼠标穿透的透明中文 overlay，显示剩余百分比和重置倒计时，例如
`5小时 58% 1时18分   周限额 75% 5天14时`。
托盘菜单、Tooltip、设置窗口、错误状态均为中文；技术诊断日志保持英文。

读 Codex CLI 已登录的 `auth.json`，只向 `wham/usage` 发请求；
不抓网页、不读 `OPENAI_API_KEY`、不改登录态。
最大化、全屏、被其他窗口覆盖时跟随 Codex 自动隐藏；离线保留 last-good；
服务端只返回周限额时仍兼容显示。
自定义代理地址保存在本机 `settings.json` 中，设置窗口会再次显示。

Rust 编写，约 500 KB，MIT。