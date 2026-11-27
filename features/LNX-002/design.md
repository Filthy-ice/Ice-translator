# LNX-002 · Design

## 实现方案
待填写：入口、仓库、目标分支、源码位置与提交基线；每个需求对应设计小节。
不改变已批准业务行为；改变行为必须 revise 并重新批准。

## REQ-001 实现与验证
待填写：调用链、状态流、复用点、外部契约、异常传播、幂等与并发。

## 影响与回滚
- 涉及仓库：ice-translator（ice-app）；新增/改动集中在 `system/portal_shortcuts.rs`、
  `system/global_hotkeys.rs`、`system/capture.rs`（能力表）与 `docs/`。
- 兼容性：mac/Windows 依赖段零改动；X11 档行为不变；Wayland 档热键从不可用变为可用。
- 回滚：把 `global_hotkeys::apply_carbon` 的 Linux 分支改回 `x11_hotkeys::apply(config)`，
  删除 `system/portal_shortcuts.rs` 与其 mod 注册即可；无配置/数据迁移。

## 补充（2026-11-27）：非沙箱二进制需先向宿主 portal 注册 app id

- **动机**：非 Flatpak 的裸二进制直接 `CreateSession` 会得到
  `NotAllowed: An app id is required`——门户从 X11 window property / host portal
  推断 app id，失败即拒。真机首跑就是这一条（`global_hotkeys.rs` 会话建不起来）。
- **接口**：绑定前先调 `org.freedesktop.host.portal.Registry.Register("ice-translator")`
  （`version 1`，本机在场），把应用名登记给宿主 portal；对应桌面文件
  `~/.local/share/applications/ice-translator.desktop`（已存在）。
- **回滚**：删掉 `create_session` 前的那一次 `Registry.Register` 调用即可；
  已在真机验证注册后 `CreateSession`/`BindShortcuts` 全通（`门户全局热键已绑定 hotkeys=3`，
  桌面/右键菜单/任意焦点下 F1/F2/F3 均可用，用户已验收）。

## 验证层次
待填写：需要的本地/Mock/跨仓契约/沙箱/业务验收/生产观察；所需外部授权单独记录。
