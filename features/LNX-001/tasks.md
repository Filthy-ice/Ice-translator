Change: LNX-001
Version: v0.1
Spec-Hash: 73873bc2b37854d01f6f79faf9fdc94813c8003b9aaad9bb6c49e164d85d87cc

# Tasks（由 tasks.json 生成，勿直接修改）
- [x] T-001 门户截图取证（先证 H1，再写实现）：新增 `examples/portal_shot_probe.rs`（linux 门，非 mac 才编译），用 zbus blocking 走 `org.freedesktop.portal.Desktop` → `Screenshot(sa{sv}o)` 非交互 → 收 `Response` → 读 URI → 打印尺寸与像素统计（均值/max/min）。留盘读数：是否出图、URI 形态、耗时、首次是否弹授权 | ice-translator | REQ-002
  Code/Test: crates/ice-app/examples/portal_shot_probe.rs
- [x] T-002 取图路径对照取证：同一帧画面下门户 vs X11 `GetImage` 的像素统计对比，把「XWayland 的 root 拍不到 Wayland 原生应用」从推断钉成实测数字（本机 X11 侧可见窗 5 个 / `_NET_CLIENT_LIST_STACKING` 2 条 / root 子窗 36 个，已有基线） | ice-translator | REQ-002, REQ-009
  Code/Test: crates/ice-app/examples/grab_source_compare.rs
- [x] T-003 XID 与后端取证（H4）：启动时打印 winit 实际后端、`ctx.viewport().raw_window_handle()` 拿到的 XID，与 `xwininfo -root -children` 的窗对表确认无 reparent、XID 可直接操作；同时确认 Vulkan/wgpu 呈现未因走 X11 后端而丢 | ice-translator | REQ-001, REQ-004
  Code/Test: crates/ice-app/src/system/window_x11.rs
- [x] T-004 新增 `system/backend.rs`（linux 主体 + 非 linux 恒等桩）：`BackendTier{X11,WaylandNative,None}` 在 `arm()` 时算一次存 `OnceLock`、之后不翻转；配 `capability_of(tier) -> capture::Capability` 纯函数表（design D-2 那四格），mac/win 取值与 `aa6c050` 的字面常量逐项相等 | ice-translator | REQ-001, REQ-011
  Code/Test: crates/ice-app/src/system/backend.rs
- [x] T-005 `Cargo.toml` 新增 `[target.'cfg(target_os = "linux")'.dependencies]`：`x11rb = "0.13"`、`zbus = { version = "5.13", features = ["blocking"] }`（两者已在 Cargo.lock，不新增版本）。验收：`cargo tree` 前后对比，**mac 与 Windows 侧节点集合零差异**（design D-1） | ice-translator | REQ-012
  Code/Test: crates/ice-app/Cargo.toml
- [x] T-006 字体来源分派（design D-5）：`ice-core/src/font.rs` 新增 linux 候选路径常量与 `font_bytes()` 分派（**不改 `HEITI_PATH`/`GOTHIC_PATH` 的值与 mac 读取顺序**）；`.ttc` 的面索引**按 name 表找含 SC 的那一面**，取不到 SC 视为不可用；无候选时报「装 fonts-noto-cjk 后可用」这一句可行动的话，而不是 `VisionError::Font` 冒在链中间 | ice-translator | REQ-008
  Code/Test: crates/ice-vision/examples/cjk_face_probe.rs
- [x] T-007 `capture.rs` 的 linux 取图实现（design D-6）：`grab_screen()`（:210 桩）与 `capture()`（:272 桩）换成门户实现；`classify`/`classify_grab`/`save_shot`/`temp_dir`/`Outcome`/`explain()` **一行不改**（公共位）；调用链 `submit_capture → Job::Freeze → submit_freeze → apply_frozen → freeze_texture` 零改动 | ice-translator | REQ-002, REQ-003
  Code/Test: crates/ice-app/src/system/capture.rs
- [x] T-008 `screen.rs::probe()` 的 linux 分支（design D-9）：XRandR / `_NET_WORKAREA` 给真几何，取代 `fallback_area`；写清「XWayland 把桌面并集报成一块虚拟屏（实测 DP-1 2560x1440+0+0，Wayland 窗坐在 +0+32）」这一档的多屏限制，`flip/union/select/fallback_area` 不动 | ice-translator | REQ-003
  Code/Test: crates/ice-app/src/system/screen.rs
- [x] T-009 `cover.rs` 三处桩接实现（:88 `covers:false` / :135 `Focus{false,false}` / :156 空 `step_back`）：`_NET_WM_STATE_ABOVE` + `ConfigureWindow` + `_MOTIF_WM_HINTS`；**不用 override_redirect**（会丢键盘焦点）；WM 不尊重 `_ABOVE` 时按 Q-2 口径显示「当前桌面置顶不保证」而入口不隐藏 | ice-translator | REQ-004
  Code/Test: crates/ice-app/src/system/cover.rs
- [x] T-010 平行新增 `system/window_x11.rs`（**不打开 `window.rs:12` 那个整模块 mac 门**）：取 XID、置顶/层级、alpha（对应 `alpha.rs:34` 恒 false 那格）、以及 `background.rs:66/:94/:120`、`dock.rs:77`、`mouse.rs:34/:62` 各桩的 linux 实现；`pin.rs:519-534` 的 builder 参数集一字不改 | ice-translator | REQ-004, REQ-005
  Code/Test: crates/ice-app/src/system/window_x11.rs
- [x] T-011 全局热键 linux 实现（design D-8）：新增 `global_hotkeys_x11.rs`，独立线程持一条 X 连接对 root 窗 `XGrabKey`（先 `UngrabKey` 再绑，对应 mac 那条 `-9878` 的 `BadAccess` 版），命中只写 `global_hotkeys.rs` 里**同一个** `PENDING` 原子位 + `request_repaint()`；`registration_plan()`/`HotkeyFailure::explain()` 三条话术原样复用，泵仍挂在 `logic()` | ice-translator | REQ-006
  Code/Test: crates/ice-app/src/system/global_hotkeys.rs
- [x] T-012 托盘 linux 实现（design D-7）：新增 `tray_sni.rs`，注册 `org.kde.StatusNotifierItem` + 导出 `com.canonical.dbusmenu`；`tray.rs:44` 的 `SUPPORTED` 与 `:805/:810/:814` 三个桩改判；成功返回 `Available` 后既有链路（`app.rs:425-431`、`mode.rs:780-786`、`hint_text()`）**一字不改**自动点亮；失败继续 `Unavailable` 走现有最小化降级 | ice-translator | REQ-007
  Code/Test: crates/ice-app/src/system/tray_sni.rs
- [x] T-013 `windows.rs::read()` 的 linux 分支：**先 `query_tree(root)` + `_NET_WM_NAME`/`WM_CLASS`/几何**，不得只读 `_NET_CLIENT_LIST_STACKING`（实测该属性只 2 条而 root 下 36 个子窗）；`topmost_at`/`clamp_to_screen`/`Snapshot::take`/`candidate` 是公共位不改；界面那句免责声明 SHALL 有实测依据（「Wayland 原生应用不在这张表里」） | ice-translator | REQ-009
  Code/Test: crates/ice-app/src/system/windows.rs
- [x] T-014 `panel.rs:173`（NSSavePanel 桩）改走 portal `FileChooser.SaveFile`（本机 introspect 实测在）；`dblclick.rs:287`、`titlebar.rs:214` 两处 `Outcome::Unavailable` 按 X11 能做到的程度补齐，做不到 SHALL 保留 `Unavailable` 并把原因写进注释与本包 evidence | ice-translator | REQ-004, REQ-005
  Code/Test: crates/ice-app/src/system/portal.rs
- [x] T-015 凭据/剪贴板/单实例三格**只验证不重写**（design 与调研 F10）：确认 `org.freedesktop.secrets` 有 owner（H3）、arboard 与 `kill(pid,0)` 在 Linux 实跑可用，结论写进 evidence 并标注「无需移植」。查不到 Secret Service 时 SHALL 明说「密钥不落盘」，SHALL NOT 回退成明文配置文件 | ice-translator | REQ-010
  Code/Test: crates/ice-app/src/system/single_instance.rs
- [x] T-016 红转绿与收口：F8 那 19 条（app 2 / jobs 1 / state 16，全在截图→定格→贴图→标注链）在 Linux 全绿；`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全绿 + `cargo check --target aarch64-apple-darwin` 通过；`cargo tree` 证明 mac/win 无新节点。任何一条靠 `#[cfg]` 跳测或删测「转绿」SHALL 判为未完成 | ice-translator | REQ-012, REQ-005
  Code/Test: docs/05-dev-guide.md
- [x] T-017 文档与账本同步：`docs/05-dev-guide.md` §8（字体那行「非 mac `install` 返回 `false`」、托盘那行「非 mac 恒 Unavailable」）、`docs/traceability.md:74`（NFR-7 那格的 Linux 口径）、`docs/07-acceptance.md` 验收结论；本机桌面入口与自启项加 `env WAYLAND_DISPLAY=` 并重新构建安装到 `~/IceTranslator-linux-x86_64/` | ice-translator | REQ-001, REQ-008, REQ-007
  Code/Test: docs/traceability.md
