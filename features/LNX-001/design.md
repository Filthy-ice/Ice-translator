Change: LNX-001
Version: v0.1
Spec-Hash: fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2

# LNX-001 · Design

基线：`origin/main` @ `aa6c050`（Linux 编译面在本会话修好两处平台门后可用）。规格快照 `fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2`（v0.1 APPROVED，批准人 zhanghaobing，2026-11-24）。

## 实现方案

**入口与总原则**：全部改动落在 `crates/ice-app/src/system/` 的 **Linux 分支**与新文件，加 `crates/ice-app/Cargo.toml` 的 **linux-only 依赖段**，加 `crates/ice-core/src/font.rs` / `crates/ice-vision/src/paint.rs` 里**新增的 linux 字体来源分派**。规格里那条硬约束（"不要动 mac 和 win 的代码"）在本设计里落成一句可执行的判据：**mac 与 Windows 的编译图和运行行为逐字等价**——允许在共享文件里新增 `#[cfg]` 分支与新增函数，禁止改写 mac 分支的常量值、调用顺序、话术和既有 cfg 分支。

### 决策 D-1 依赖只用 lock 里已有的件，且只挂 linux 段

`[target.'cfg(target_os = "linux")'.dependencies]` 新增 **`x11rb = "0.13"`**（Cargo.lock 已因 `arboard 3.7` 与 winit 的 x11 feature 存在 0.13.2）与 **`zbus = { version = "5.13", features = ["blocking"] }`**（lock 已因 `keyring → zbus-secret-service-keyring-store` 与 `accesskit → atspi` 存在 5.13.2）。

- 引新件 = 把新版本塞进 lock 并可能牵动 mac/win 的依赖解析；用已定版的传递依赖，`cargo tree` 在 mac target 上不会多出任何节点。**验收项**：`cargo tree` 前后对比，mac/win 侧节点集合零差异。
- **不引** `ashpd`（封装门户但会带来自己的 tokio/版本面）、**不引** `tray-icon`/`muda`（Linux 后端要 libappindicator+gtk3，为一个托盘拖进整个 GTK）、**不引** `fontconfig`/`fontdb`/`cosmic-text`（度量层现在就是 `owned_ttf_parser` + `ab_glyph_rasterizer`，见 D-5）。
- eframe 0.37 保持 default features（同时带 `x11` 与 `wayland`，`cargo tree` 实证 egui-wgpu + winit 0.30.13），**不改 feature 集**——XWayland 靠启动环境选择后端，不靠裁 feature（裁 feature 会改 mac/win 的编译面）。

### 决策 D-2 `capability()` 的语义原样保留，Linux 只是"有实现"

调研确认 `capture.rs:51-60` 是全仓唯一判定处、且**是公共位**（模块注释 :3-4 明令"视图里出现 cfg(target_os) 就是缺陷"）。它的语义不是"此刻会话能不能用"，而是"**这一档有没有编译进来的实现**"——mac 在 `/usr/sbin/screencapture` 缺席时照样返回 true，运行期失败交给 `Outcome::Failed(ToolMissing)` + `explain()`。

所以 Linux 侧照同一形状办，`capability()` 的四格改成 `capability_of(tier: BackendTier) -> Capability`（纯函数，可无头断言）：

| 格 | mac（不许变） | Win（不许变） | Linux X11 档 | Linux Wayland 原生档 |
|---|---|---|---|---|
| capture | `true` | `false` | `true`（门户实现） | `true`（门户实现） |
| pin | `true` | `true` | `true` | `false`（置顶/移窗不可承诺） |
| hotkey | `true` | `true` | `true`（`XGrabKey`） | `false` |
| ocr | `native_ocr_supported()` | 同左 | `false` | `false` |

- 回归锁：新增断言"**mac 档与 Win 档的这四格取值与 `aa6c050` 的字面常量逐项相等**"，用编译期可见的常量而非运行时判断来表达（mac 走 `cfg!` 的那条路径不能被本包改成探测）。
- **这条同时修正了 Q-1 的批准口径**：`explain()` 里 `Unsupported` 那句在 `aa6c050` 已经写着"这台机器上没有可用的框选截图；**可先用系统截图再点「粘贴」**"——替代入口的话术是既有资产，本包不新增也不隐藏入口，只需让 Linux 的失败走到这句上去。规格里"隐藏入口"那一读作废，写在这里而不是改 spec 文本（行为没变，变的是我对既有资产的理解）。

### 决策 D-3 后端档位单点判定

新增 `crates/ice-app/src/system/backend.rs`（`#[cfg(target_os = "linux")]` 主体 + 非 linux 上恒 `BackendTier::MacOrWindows` 的桩，保证公共调用点不需要散写 cfg）：

- 判定：`DISPLAY` 可连 **且** 本次进程实际以 x11 后端起窗 → `X11`；只有 `WAYLAND_DISPLAY` → `WaylandNative`；都拿不到 → `None`。
- 存在 `OnceLock<BackendTier>` 里，`arm()` 时写一次，**之后不翻转**（REQ-001 的边界）。
- 桌面入口与自启项加 `env WAYLAND_DISPLAY=`（用户 2026-11-24 选定"走 XWayland 换功能"）；两处 `.desktop` 文件在本机 `~/.local/share/applications/` 与 `~/.config/autostart/`，属部署件不进仓库。

### 决策 D-4 XID 取法与"无 reparent"这一实测事实

本机 `xwininfo -root -children` 实测：root 直接挂 **36 个子窗**（coopanion / qoder-cn / wechat 各是 X11 客户端），`Parent window id: 0`——XWayland 下没有 reparent WM，**顶层窗就是 root 的直接子窗**。所以：

- 自家窗口的 XID：`egui::Context::viewport().raw_window_handle()`（贴图/遮罩各自的 viewport 内取）→ `XcbWindowHandle::window()`。根窗用 `Frame::winit_window()` 同路。**这条是 H4 的一部分，T-003 要打出真 XID 并与 `xwininfo` 对表**（winit x11 在某些路径会造子窗，不实测就是猜）。
- 层级：`_NET_WM_STATE_ABOVE`（`x11rb` change_property + send_event）；**不用 `override_redirect`**——那会让窗口拿不到键盘焦点，贴图上的"点一下才译"就废了。

### 决策 D-5 字体：不猜 ttc 面索引，读 name 表选 SC 面

`ice-core/src/font.rs:17/23` 那两个常量的**值不动**（mac 资产）；新增 `linux_font_candidates()` 与分派点 `font_bytes()`（在 `ice-vision/src/paint.rs:47-56` 的 `OnceLock` 填充处加 linux 分支，mac 分支的读取顺序与失败语义不变）：

- 候选路径：`/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc`、`/usr/share/fonts/truetype/noto/*CJK*`、`/usr/share/fonts/**/SourceHanSansSC*`（`fc-match "Noto Sans CJK SC"` 实测命中前者）。
- **.ttc 的面索引按 name 表选**：遍历 `FontCollection` 找 family 含 "SC" 的那一面，取不到 SC 面 = 视为不可用走回落。**不许硬编码 index**——这正是 mac 那次"STHeiti 第 1 面才是黑体-简"的同族坑，而且实测 `fc-match sans:lang=zh` 默认命中的是 JP 面。
- 一个 CJK 字体都没有时：报"缺字体，装 fonts-noto-cjk 后可用"这句可行动的话（REQ-008），而不是让 `VisionError::Font`（`paint.rs:60-63` 唯一出口）冒在链中间。
- 顺带闭合 `docs/traceability.md:74` 记的那格"非 mac `install()` 返回 false"：`fonts.rs:48-69` 的 `definitions()` 是公共位，只加 linux 分支，**不改 mac 返回值与回落顺序**。

### 决策 D-6 截图走门户，X11 抓屏不作为主路径

- 主路径：zbus blocking 代理 `org.freedesktop.portal.Desktop` / `/org/freedesktop/portal/desktop`，调 `Screenshot(sa{sv}o)`（本机 introspect 实测签名），非交互选项，`Response` 用 `add_match` + `receive_with_timeout` 收，读回 URI → `image::open` → 复用既有 `save_shot`/`classify_grab`（`capture.rs:229/:287`，**公共位不改**）。超时/拒绝 → `Failed`/`Cancelled` 走既有分档。
- 为什么不用 X11 `GetImage`：XWayland 的 root 只合成 X11 客户端，**Wayland 原生应用拍不到**。本机 X11 侧只见 5 个可见窗、`_NET_CLIENT_LIST_STACKING` 只 2 条，而桌面上开着多个 Wayland 原生应用（事实 F6）→ 拍出来是缺块的图。T-002 出一次对比读数（同一帧画面，门户 vs XGetImage 的像素统计），把这个结论钉成事实而不是推断。
- 门户拿不到图时：复用 `explain()` 那句"先用系统截图再点「粘贴」"（D-2），不新增产品入口。

### 决策 D-7 托盘手写 SNI + dbusmenu

`tray.rs:44` 的 `SUPPORTED = cfg!(target_os="macos")` 与 `:805/:810/:814` 三个桩是纯 cfg 骨架（上层 `app.rs:425-431`、`mode.rs:780-786` 的 `TrayState` 分叉全是公共逻辑且已经写全）。新增 `tray_sni.rs`：

- 注册 `org.kde.StatusNotifierItem`（宿主实测在场：由 gnome-shell pid 5481 持有，现场已有两个其它应用挂上）+ 导出 `com.canonical.dbusmenu` 给菜单。
- 成功 → `install()` 返回 `Available`，**整条既有链路自动点亮**（含"关窗收进托盘"那条支路）；失败 → 继续返回 `Unavailable` 走现有最小化降级，话术 `hint_text()` 不动。

### 决策 D-8 全局热键复用同一份 PENDING 与 `Action`

新增 `global_hotkeys_x11.rs`：独立线程持一条 X 连接，对 root 窗 `XGrabKey`（先 `UngrabKey` 再绑，对应 mac 那条 `-9878` 教训的 Linux 版 `BadAccess`），收到 `KeyPress` 只写 `global_hotkeys.rs` 里那同一个 `PENDING: AtomicU8` 并 `ctx.request_repaint()`——**不开第二条分派路径**（该文件文件头"图钉那次两条通道各走各的"就是这条纪律的来源）。`registration_plan()` 与 `HotkeyFailure::explain()` 三条话术**原样复用**，Linux 侧撞车返回 `Taken`。

### 决策 D-9 认窗与屏幕几何按 X11 能给的说，缺的明说

- `screen.rs:181` 的 `probe()` 加 linux 分支：XRandR 实测本机报 **1 块虚拟屏 `DP-1 2560x1440+0+0`**，而 `xwininfo` 里 Wayland 应用的窗坐在 `+0+32`（顶栏高）——**XWayland 把桌面并集报成一整块屏**。结论写进分档表：Linux 档的多屏独立几何拿不到，`state::tests::a_selection_belongs_to_the_screen_it_was_started_on` 在单虚拟屏语义下成立即可（不是靠跳测）。
- `windows.rs:111` 的 `read()` 加 linux 分支：**先 `query_tree(root)` + `_NET_WM_NAME`/`WM_CLASS`/几何**，而不是只读 `_NET_CLIENT_LIST_STACKING`（实测该属性只有 2 条，而 root 下有 36 个子窗——只读 EWMH 列表会把绝大多数窗丢掉）。`topmost_at()`/`clamp_to_screen()` 是公共位，**不改**；只喂给它更真的数据，并按 REQ-009 明示"Wayland 原生应用不在这张表里"。

### 冻结清单（改它＝改 mac，本包不得改）

`capture::capability()` 的 mac/win 取值与 `Outcome::explain()` 话术表；`state::capture_gate()/submit_capture()/apply_capture()`；`jobs.rs` 的 CaptureRun/FreezeRun 注入位；`pin.rs:519-534`、`selection.rs:933-948`、`pill.rs:304-316`、`mode.rs:364-378` 的 `ViewportBuilder` 参数集与每帧 `show_viewport_deferred` 协议；`windows::topmost_at/clamp_to_screen`；`screen::flip/union/select/fallback_area`；`fonts::definitions()` 的 mac 分支；`paint.rs:60-63` 的错误出口形状；以及 `global_hotkeys.rs` 文件头那四条实测硬规矩。

## REQ-001 实现与验证（后端探测与启动形态）

`backend.rs` 出 `BackendTier` + `fn current()`；`app.rs` 启动处调用一次（**调用点是公共位**，只加一行 linux 门内的 `backend::arm()`）。验证：无头断言"档位→能力表"那张纯函数表；真机断言 H4（去掉 `WAYLAND_DISPLAY` 后 winit 落 x11、Vulkan 仍在呈现、帧率不塌）——取数方式是从进程日志与 `xwininfo` 对 XID，不看截图（本机 GUI 验证一律走数值）。

## REQ-002 实现与验证（截图取图走门户）

`capture.rs:272` 那个恒 `Unsupported` 的桩换成 `#[cfg(target_os="linux")] fn capture_linux()`；`grab_screen()`（`:210`）同形。调用链 `state.rs:1117 submit_capture` → `Job::Freeze` → `jobs.rs:167/358 submit_freeze` → `apply_frozen` → `app.rs:1432 freeze_texture` **一行不改**，只换最底下的取图像函数。异常传播沿用既有 `Failure` 五格 + `explain()`。验证分两层：假 DBus（脚本化的 `Response`）证状态机与超时；真机证 H1（我们的 app id 能不能拿到图）——两者不可互替，`evidence.md` 注明是哪种。

## REQ-003 实现与验证（整屏定格与遮罩框选）

`cover.rs:88/:135/:156` 三处桩接 D-4 的 `_NET_WM_STATE_ABOVE` 与 `_MOTIF_WM_HINTS`/`ConfigureWindow`；`selection.rs` 的 builder 参数不动（遮罩铺满 `screen::union`，D-9 之后是那一块虚拟屏）。回归靶子：`app::tests::capture_never_hides_our_own_window`（`app.rs:5264` `Idle != Capturing`）——"发起前先藏自己"那步在 Linux 上必须真的发生。

## REQ-004 实现与验证（钉屏贴图）

`window.rs` 整模块是 mac 门（`mod.rs:28-29` 也门了一次）→ 平行新增 `window_x11.rs`，**不打开 mac 那个门**。`pin.rs:526` 的 `with_always_on_top()` 保持不动（X11 下由 XWayland 映射到 `_NET_WM_STATE_ABOVE`，够用），额外用 x11rb 补 mac 才有、而 eframe 没给的那几格：贴边收起、层级微调、阴影（`shadow.rs:36`）。WM 不尊重 `_ABOVE` 时按 Q-2 口径**显示"当前桌面置顶不保证"、入口不隐藏**。

## REQ-005 实现与验证（标注对象链）

`annotate.rs`（2088 行）**调研未发现平台门**，`selection.rs`（3064 行）同理——本 REQ 的 Linux 阻塞点是上游依赖：截图取不到（REQ-002）+ 置顶不生效（REQ-004）+ 字体报 `VisionError::Font`（REQ-008）。三者通了之后，那批 pin/anno/crop/generation 红测试应自然转绿；**若某条断言 genuinely 依赖 mac 语义（Q-4），处理方式是"两档断言"而不是 `#[cfg]` 跳过**，并在 `docs/07` 记行为差异。

## REQ-006 实现与验证（全局热键）

见 D-8。`global_hotkeys.rs:259/:347` 那两个非 mac 桩是接入点；`arm()`（:227）保持"存 `CONTEXT` + 装 handler"的形状，handler 换成 x11 线程。H5 实测：默认裸 F1/F2/F3 在本机能否抢到（GNOME Shell 自己占了 F1），抢不到 SHALL 给出可用默认值建议。

## REQ-007 实现与验证（托盘）

见 D-7。**H2 未闭合前第一条任务是真机冒烟**（图标出现 + 一次菜单点击命中），不是先写实现。

## REQ-008 实现与验证（字体）

见 D-5。验证：无头断言"给定 ttc 字节流，选出的面 family 含 SC"；真机断言译文图片导出（`jobs::tests::a_compose_job_returns_one_whole_picture_and_no_intermediate_frames`，`jobs.rs:1150`"成品要落在这张贴图身上"）转绿。

## REQ-009 实现与验证（认窗分档）

见 D-9 第二条。`Snapshot::take()`/`candidate()` 是公共位不改。验证：本机 `query_tree` 得到的窗数与 `xwininfo -root -children` 的 36 对表，并核对"Wayland 原生应用确实不在表里"这句话是否为真（不能报一个没依据的免责声明）。

## REQ-010 实现与验证（凭据/剪贴板/单实例）

调研已证这三格**本来就跨平台**（`libc` 挂在 `cfg(unix)` 段、arboard 用 lock 里的 x11rb、keyring 带 zbus Secret Service store）→ 本 REQ 只验证并留读数，**不许重写**。H3：先确认 Secret Service 是否应答（查 `org.freedesktop.secrets` 有没有 owner）；查不到就明说"这台机器没有安全存储，密钥不落盘"，SHALL NOT 回退成明文配置文件。

## REQ-011 实现与验证（探测取代硬编码）

`capability_of(tier)` 那张表（D-2）+ `Settings` 页一行"当前档位：X11 / Wayland 原生 / 无"。表是纯数据，无头可断言，不要求真调 X11。

## REQ-012 实现与验证（门禁与编译面）

`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 在 Linux 全绿；`cargo check --target aarch64-apple-darwin` 通过；**新增一条本包专属防线**：`cargo tree` 对比 mac/win 侧节点集合无变化（证明 D-1 没把新依赖漏进公共段）。完成线是 F8 那 19 条全绿。

## 影响与回滚

- 涉及文件：`Cargo.lock`、`crates/ice-app/Cargo.toml`（新 linux 依赖段）、`crates/ice-app/src/system/{backend.rs,tray_sni.rs,global_hotkeys_x11.rs,window_x11.rs}`（新增）、`{capture.rs,tray.rs,global_hotkeys.rs,cover.rs,screen.rs,windows.rs,shadow.rs,attach.rs,panel.rs,mouse.rs,alpha.rs,background.rs,dock.rs}` 的**非 mac 分支**、`crates/ice-core/src/font.rs` 与 `crates/ice-vision/src/paint.rs` 的新增分派点。`state.rs`/`jobs.rs`/`app.rs`/`pin.rs`/`selection.rs`/`annotate.rs` 目标上**零改动**（若实现中发现必须改，先停下记 SPEC_DEVIATION，因为它们是 mac 也走的公共位）。
- 兼容性：配置文件 `config.toml` 的快捷键字段沿用 `HotkeyConfig` 同一份解析（`is_unbound`/`parse_combo` 是公共位，不改语义）；Linux 首次运行若默认 F1 抢不到，SHALL 只改填值不改区间（用户既有规矩："设为默认"≠抬上限）。
- 数据/密钥：截图产物进 Linux 的专有历史目录（`capture.rs:299 temp_dir` 已有平台分支位），API Key 走 Secret Service，**不落配置、不进日志**。
- 回滚：本包所有新增文件与 linux 段可整块删掉即回到"Linux 编译通过但功能桩"的状态（`aa6c050` 的行为）；mac 与 Win 无回滚需求，因为没有行为变化。回滚条件：任一条转绿测试是靠删测试/跳测达成的，或 `cargo tree` 显示 mac/win 侧节点变化。

## 验证层次

| 层 | 能证明什么 | 不能证明什么 |
|---|---|---|
| 无头单测（`capability_of` 表、ttc 面选择、`registration_plan`） | 纯逻辑与档位映射 | 门户/X11 是否真给东西 |
| 假 DBus / 脚本化 `Response` | 我们的状态机、超时、取消、幂等 | 我们的 app id 是否被授权（H1） |
| 真机 X11 对表（`xwininfo`/`xrandr`/`xdotool` 数值） | XID、置顶属性、窗数、几何 | Wayland 原生应用的可见性（结构性拿不到） |
| 真机门户调用 | H1（能否出图）、SNI 图标与菜单（H2） | 其它 DE 的行为 |
| 现场按键 | H5（默认键是否抢到） | —— 无法软件自证，每轮需人在机器前 |
| 业务验收 | 截图→定格→框选→钉住→标注→复制/保存这条链的手感 | OCR（本包边界外） |

外部授权需求：门户首次截图授权需你在机器前点一次；不需要 sudo；不碰 gsettings 代理设置。
