# CodexPlus · 仓库简介

Windows 单 EXE 小插件，在 Codex 桌面应用标题栏右侧贴一条鼠标穿透的紧凑条，
显示 5 小时与周限额的已用百分比和重置倒计时。

读 Codex CLI 已登录的 `auth.json`，只向 `wham/usage` 发请求；
不抓网页、不读 `OPENAI_API_KEY`、不改登录态。
最大化、全屏、非前台时自动隐藏；离线保留 last-good。

Rust 编写，约 476 KB，MIT。