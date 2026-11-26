# LNX-002 · Wayland 档全局热键走门户 GlobalShortcuts（X11 档保持 XGrabKey）

## 原始意图与来源

- **用户 2026-11-26 原话**："F1 在桌面（Wayland 焦点）按下没反应，点 Qoder（XWayland）就正常"；
  并给出目标："Wayland 会话下改用 xdg-desktop-portal 的 GlobalShortcuts 注册全局热键；
  X11 会话或门户不可用时回落到现有 XGrabKey。"
- 根因（已定位，非猜测）：LNX-001 的 Linux 热键实现是 `XGrabKey` 抓 root
  （`system/global_hotkeys.rs` 的 `x11_hotkeys` 子模块）。**mutter 只在焦点是 X11 窗口时
  才把按键交给 XWayland**，所以焦点在 Wayland 窗口（DING 桌面、GNOME 原生应用）时，
  grab 收不到那一下。置顶/截图/停靠都与此无关。

## 事实（本机实测，2026-11-26）

- F1：`org.freedesktop.portal.Desktop` 的 `org.freedesktop.portal.GlobalShortcuts` 版本 **1**，
  introspect 列出 `CreateSession / BindShortcuts / ListShortcuts / ConfigureShortcuts` 与
  `Activated / Deactivated / ShortcutsChanged` 信号；GNOME 后端在场。
- `BindShortcuts` 首次调用会弹**一次系统授权框**（门户自己管，应用不保存权限）。
- 仓库已有 `zbus 5.13`（blocking）与 `x11rb`；不引 `ashpd`（依赖图纪律：mac/Windows 段零改动）。

## 目标与非目标

**目标**
1. 档位为 `WaylandNative` 时：用门户 `GlobalShortcuts` 注册三条热键（capture / pin_last / reopen），
   用户点授权框后生效；命中时与现有 `PENDING` 原子位 + `ctx.request_repaint()` 同一条分派。
2. 档位为 `X11` 时：保持 `XGrabKey`（LNX-001 已实现），行为不变。
3. 门户缺席/拒绝/超时：如实返回 `Unavailable`（界面那句"只能用图片页那颗按钮"仍是正确出口），
   不静默。

**非目标**
- 不做"按应用区分"的快捷键、不做 `ConfigureShortcuts` 的 UI；
- 不改 mac/Windows 任何分支；不引入新依赖。

## 行为与验收

### REQ-001: 触发串格式化
`HotkeyConfig` 的三项（`capture` / `pin_last` / `reopen`）SHALL 被格式化成 XDG shortcuts
触发描述（`F1`、`CTRL+SHIFT+F1`、`ALT+F2`、`LOGO+F3` 等）：
- 修饰键顺序固定 `LOGO+CTRL+ALT+SHIFT`，只列已启用的；
- 主键：`F1..F12` → `F1..F12`，字母 → 大写单字符，数字 → 原字符；
- `is_unbound` 的一项跳过；解析失败的一项跳过并记一条 warn（与现有 `registration_plan` 同口径）。

### REQ-002: 注册与分派
- 先 `CreateSession`（`handle_token` + `session_handle_token`，均为字母数字），再
  `BindShortcuts(session, [(id, {description, preferred_trigger})], "", {})`；
- 三个 id 固定为 `capture` / `pin_last` / `reopen`，与 `tray`/`Action` 的分派一致；
- 收到 `Activated(session_handle, shortcut_id, timestamp, options)` 时只写
  `PENDING` 并 `ctx.request_repaint()`（与 mac/LNX-001 同一条纪律）；
- 会话与订阅挂在**一条常驻连接**上（教训来自 LNX-001：连接随调用释放会让第二次起全部失败）。

### REQ-003: 回退
- `backend::current() == X11` → `XGrabKey` 路径（现状）；
- `WaylandNative` → 门户路径；门户连不上/`CreateSession` 失败 → `Err(Unavailable)`；
- `NoBackend` → `Err(Unavailable)`。

### 验收
- V-001：`cargo test -p ice-app` 全绿，含新增单测（触发串、id→动作、回退判定、去重）。
- V-002（真机，用户执行）：焦点在 DING 桌面按 F1 进入框选；F2/F3 同验；XWayland 焦点下仍正常；
  首次会弹一次授权框。

## 回滚
单文件开关：把 `global_hotkeys::apply` 的 Linux 分支改回 `x11_hotkeys::apply(config)` 即可；
`portal_shortcuts` 模块整体删除不影响其它文件。
