# LNX-001 · Linux 移植：mac 已实现能力按 X11/Wayland 分档补齐（OCR 识别本轮不做）

## 原始意图与来源

- **用户 2026-11-24 的原话是本包的直接来源**（逐字）：
  1. "拉取最新代码，**根据 mac 版本的，移植一下 linux 版本的**，linux 版本的代码应该距离 mac 版本的代码比较远了。**当时我开发的时候先紧着 mac 版本的开发**。"
  2. 移植档位："**现在 mac 已经实现的就全部移植过来**"。
  3. OCR 边界："**这一轮不做 OCR**"。
  4. 落账方式："**先立 CHG 规格包**"（批准前不动业务代码）。
  5. 窗口后端："**接受走 XWayland 换功能**"（启动时让 winit 走 X11 后端，换取置顶、任意移窗、贴边收起、层级、认窗）。
  6. 移植边界："**全要，含标注与遮罩链**"（截图取图 + 整屏定格遮罩 + 标注对象模型都进本包）。
- 仓库里已有的 Linux 结论只有**底线那一格**：`docs/traceability.md:74`（NFR-7）写"S 档只要求 Windows/Linux 下 `cargo check` 通过且文本翻译功能可用；OCR 在非 macOS 明确提示不支持"，出处 `docs/01-requirements.md:102`。**"mac 已实现的能力在 Linux 上补齐"从来不是 NFR-7 的承诺范围**，所以本包是新业务判断，走完整规格流程，不算修既有 bug。
- 用户另一句"先紧着 mac 开发"在代码里可复查：`crates/ice-app/src/system/` 下 window.rs 整模块、alpha/background/titlebar/dblclick/attach/shadow/cover/dock/panel/mouse/screen/probe/tray 各文件的非 mac 分支几乎全是 `false` / `()` / `Outcome::Unavailable` / `TrayState::Unavailable`（逐处行号见"事实 9"）。**这些不是待删的桩，是当时有意留的降级口**——本包按事实把它们逐格换成实现或"探测后确实做不到"。
- 与本包无关但同一天发生的事：两处 Linux **编译**断裂（Carbon `link kind framework`、example 里的裸 `use objc2`）已在本会话修好，属于 NFR-7 已批准"Linux 能编译"范围内的平台门补漏，**单独提交，不进本包验收**（见"允许动作"）。

## 目标与非目标

**目标**

1. **mac 已实现的每一格能力，Linux 给出"实现"或"探测后做不到"的确定答案**，SHALL NOT 留"因为当时先写的 mac 所以这里是空的"这种状态。
2. **截图 → 整屏定格 → 框选 → 钉屏 → 标注** 这条链在 Linux 上跑通到"能钉住、能框、能标注、能复制、能保存"；**到"翻译"那一步因 OCR 不在本包而明确止步**（不是失败，是本包边界）。
3. **窗口后端按探测分档**：X11/XWayland 到手全功能（置顶、任意移窗、贴边收起、层级、全局热键）；纯 Wayland 会话下降到"能开能用、贴边与置顶不做承诺"，并且**在设置页说清当前处在哪一档**。
4. **能力判定从硬编码改成探测**：`capture.rs:51-59` 的 `capability()` 现在把 capture/hotkey 写死 false，本包之后它 SHALL 反映"这台机器此刻真的有没有"。
5. **19 条红测试转绿**：`cargo test --workspace` 在 Linux 上目前是 566 passed / 19 failed / 1 ignored，19 条全部落在截图→贴图→标注链——这批就是本包的客观完成判据（不是"我看起来改完了"）。
6. 中文字体与译文图片导出在 Linux 不再依赖苹果字体路径（`ice-core/src/font.rs:17/23` 的 `HEITI_PATH`/`GOTHIC_PATH`）。
7. 三平台编译面继续成立：本包所有新代码 SHALL NOT 让 mac 侧回归（mac 那份实现与证据是已验收的，移植是**加分支**而不是重写公共层）。

**非目标**

| 不做 | 归属 / 理由 |
|---|---|
| **本地 OCR 识别**（Vision / Windows.Media.Ocr / Tesseract 任何后端） | 用户 2026-11-24 明说"这一轮不做 OCR"。`native_ocr_supported()` 非 mac 恒 false 这条天花板保持不动（L-6 形态不变），本包只把"取图、钉图、标注"做到 OCR 门前 |
| Windows 侧任何新能力 | 本包只承诺 Linux。`docs/traceability.md:74` 里 Windows 的 `pid_alive()` 那格（`single_instance.rs:75-78`）不在本包 |
| ScreenCast 持续屏幕流 / 录屏 | portal 的 `ScreenCast` 接口在本机可用（事实 2），但本仓要的是"拍一张"，`Screenshot.Screenshot` 够用；引 ScreenCast 要多养 PipeWire 会话与权限生命周期，收益不抵复杂度 |
| 滚动长截图、延时截图、窗口列表一键截 | 与 mac 现状对齐：mac 也没做，本包不发明新需求 |
| 全局热键的 portal `GlobalShortcuts` 正式支持 | 本机权限库里连 `global_shortcuts` 表都还没建（事实 3 反面），且该路径首次使用必弹授权。本包只做 X11 `XGrabKey` 一档，portal 那条记进未决问题 Q-3 |
| 改 `ice-core` 的业务规则、翻译链、Provider 协议 | 依赖方向与 NFR-6/ADR-006 不动；本包代码全在 `ice-app/src/system/` 与其调用点 |
| 撤销或改写 mac 侧已验收行为 | 用户偏好：不许删自研功能。mac 分支保持原样，Linux 走新分支 |
| 主题化、i18n、历史搜索 | 各自归属（#28 / #23），CHG-009 也把它们列为非目标 |

## 分档表（本包的核心口径）

| 能力 | mac 现状（证据） | Linux 本轮档位 | 落地手段 |
|---|---|---|---|
| 截图取图 | `/usr/sbin/screencapture`（`capture.rs:32`） | **实现** | portal `Screenshot.Screenshot(sa{sv}o)` 抓全屏 → 自绘遮罩框选 |
| 整屏定格遮罩 | 遮罩窗（`cover.rs`） | **实现** | 定格帧铺满桌面，框选在帧上画（X11 下真置顶） |
| 钉屏贴图 | `window.rs` 置顶 + 层级 | **实现（X11 档）** | `_NET_WM_STATE_ABOVE` / `_MOTIF_WM_HINTS`；Wayland 原生档降级 |
| 贴边收起 | `attach.rs:71/76`、`cover.rs:157` | **实现（X11 档）** | 任意移窗 X11 可用（`set_outer_position` 在 Wayland 是空函数，winit `wayland/window/mod.rs:273`） |
| 全局热键 | Carbon `RegisterEventHotKey` | **实现（仅 X11 档）** | `XGrabKey`；纯 Wayland 会话 → `HotkeyFailure::Unavailable` 那句能行动的话 |
| 托盘 | `tray.rs` mac 分支 | **实现** | SNI `org.kde.StatusNotifierWatcher`（本机已由 gnome-shell 持有，事实 5） |
| 认窗列表 | `windows.rs:105` CGWindowList | **降级** | X11 只看得到 X11 客户端（实测 2~5 个，事实 6）→ 入口保留但明示"看不到 Wayland 原生窗口" |
| 另存为面板 | `panel.rs:173` NSSavePanel | **实现** | portal `FileChooser.SaveFile`（本机可用，事实 2） |
| 中文字体 | 苹果 STHeiti 面 1 | **实现** | `Noto Sans CJK SC`（事实 7），显式锁族名不吃默认 JP 面 |
| 本地 OCR | Vision | **不做** | 用户明令本轮不做 |

## 事实与假设

标签 F=实测过的事实（带环境/时间/命令），H=假设（未验证，不得当依据验收）。全部为 2026-11-24 本机、HEAD `aa6c050`。

| 标签 | 结论 | 来源 / commit / 环境 / 时间 |
|---|---|---|
| F1 | 会话是 Wayland 上有 XWayland：`XDG_SESSION_TYPE=wayland`、`WAYLAND_DISPLAY=wayland-0`、`DISPLAY=:0`，GNOME Shell 50.1 | 本机 env + `gnome-shell --version`，2026-11-24 |
| F2 | xdg-desktop-portal 在 `/org/freedesktop/portal/desktop` 上暴露 `Screenshot.Screenshot(sa{sv}o)`、`ScreenCast`(CreateSession/SelectSources/Start/OpenPipeWireRemote)、`GlobalShortcuts`(CreateSession/BindShortcuts/ListShortcuts/ConfigureShortcuts)、`Clipboard`、`Background`、`FileChooser.OpenFile/SaveFile/SaveFiles`、`Settings` | `busctl --user introspect org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop --xml`，2026-11-24 |
| F3 | 门户截图路径**在本机被真实走通过**：权限库 `screenshot/screenshot` 表已存在且有条目（某应用 = "yes"）。同一张权限库里 `global_shortcuts/global_shortcuts` 与 `screen/screen`、`background/background` **均无表项** | `busctl … PermissionStore Lookup ss screenshot screenshot`，2026-11-24 |
| F4 | GNOME 私有截图总线**不可用**：`org.gnome.Shell.Screenshot` 是独立总线名，`Screenshot` 方法签名在 GNOME 50 是 `(bbs)`（不是文档里的 `bsb`），调用返回 **Access denied** | `busctl --user call org.gnome.Shell.Screenshot /org/gnome/Shell/Screenshot …`，2026-11-24 |
| F5 | SNI 托盘宿主在场：`org.kde.StatusNotifierWatcher` 由 gnome-shell(pid 5481) 持有，现场已挂着两个其它应用的 StatusNotifierItem | `busctl --user list`，2026-11-24 |
| F6 | X11 视角窗口面窄：`_NET_CLIENT_LIST_STACKING` 只 2 个、`xdotool search --onlyvisible` 5 个，而桌面实际开着多个 Wayland 原生应用 → **认窗在 XWayland 下结构性看不全** | 本机实测，2026-11-24 |
| F7 | 中文字体可用：`fc-match "Noto Sans CJK SC"` → `NotoSansCJK-Regular.ttc`；裸 `fc-match sans:lang=zh` 命中的是 JP 面（**必须显式锁族名**，同 mac 那次"STHeiti 第 1 面才是黑体-简"的教训） | `fc-match`，本机 40 个中文字体家族 |
| F8 | Linux 编译面现在成立（本会话修好两处平台门缺口后）：`cargo test --workspace` = **566 passed / 19 failed / 1 ignored**，19 条全部集中在截图→贴图→标注链（`app::tests::capture_never_hides_our_own_window` 断言 `Idle != Capturing`；`state::tests::an_empty_freeze_is_a_failure_never_a_cancellation` 在 `state.rs:2790` panic"该发得出"；其余为 state/jobs 层 pin/anno/crop/generation 系列） | `/tmp/ice_test_linux2.log`，HEAD `aa6c050`，2026-11-24 |
| F9 | 非 mac 分支的降级口逐处可数：`window.rs:12`（整模块 mac 门）、`alpha.rs:34`→false、`background.rs:66/94/120` no-op、`titlebar.rs:214`/`dblclick.rs:287` `Outcome::Unavailable`、`attach.rs:71/76` false/`()`、`shadow.rs:36` false、`cover.rs:89/136/157` covers:false / Focus{false,false} / `step_back` 空、`tray.rs:799/804/809/814` `TrayState::Unavailable`、`dock.rs:77` false、`panel.rs:173` false、`mouse.rs:35/63` None、`screen.rs:153/:182` 非 mac `probe()` 返回 None、`probe.rs:139/157` None、`capture.rs:32`（screencapture）与 `:211/:273` `Outcome::Unsupported`、`capability()` `:51-59` 硬编码 capture/hotkey=false、`windows.rs:105` 非 mac `Vec::new()`、`src/fonts.rs:49` + `ice-core/src/font.rs:17/23` 苹果字体路径 | `git show aa6c050` 起逐文件读，2026-11-24 |
| F10 | 无头/纯数学那批**本来就跨平台**，不需要移植：`layout.rs`（纯数学）、`composite.rs:582`/`paint.rs:617` 的 cfg 只在 test 模块、`clipboard.rs`（arboard）、`secrets.rs`（keyring + `zbus-secret-service-keyring-store`）、`single_instance.rs:62`（`cfg(unix)` `kill(pid,0)`）、`hotkeys.rs`（纯 egui 窗口内按键） | 同上 |
| H1 | portal 截图**首次调用会弹一次授权**，且返回的是 URI 而非像素；本会话只证明了"路径存在 + 权限表建过"，**没证明我们自己的 app id 拿得到图** | 未验证 → REQ-002 要求实机跑一次并留读数 |
| H2 | 我们锁的 `tray-icon`/` muda` 版本在 SNI 下能出图标与菜单 | 未验证（mac 走的是 NSStatusItem）→ REQ-007 首条任务先做真机冒烟 |
| H3 | gnome-keyring / Secret Service 在本会话可用（`impl.portal.Secret` 在，但服务是否应答未查） | 未验证 → REQ-010 先查再改口径 |
| H4 | 去掉 `WAYLAND_DISPLAY` 后 eframe/winit 稳定落到 X11 后端且不丢 wgpu/Vulkan 呈现 | 未验证（本机 RTX 5060 Ti 走 Vulkan，与后端选择正交但必须实测）→ REQ-001 第一验收项 |
| H5 | X11 `XGrabKey` 抓到的键能覆盖默认 F1/F2/F3（浏览器/Shell 未必让） | 未验证 → REQ-006 冲突分支要有实测读数 |

## 行为与验收

### REQ-001: 后端探测与启动形态

WHEN 应用在 Linux 启动 THEN 系统 SHALL 判定当前处在 **X11 档**（`DISPLAY` 可用且 winit 实际用了 x11 后端）还是 **Wayland 档**（只有 `WAYLAND_DISPLAY`），并把该判定作为**本进程唯一一份**能力真值来源；桌面入口与自启项 SHALL 以清掉 `WAYLAND_DISPLAY` 的形式启动（用户 2026-11-24 选定"走 XWayland 换功能"）。

- 正常路径：X11 档下置顶、任意移窗、贴边收起、全局热键四项 SHALL 全可用；设置页 SHALL 显示当前档位一行。
- 异常路径：拿不到 X11（纯 Wayland 且未装 XWayland）时 SHALL 落 Wayland 档，能力表按"分档表"降级，SHALL NOT 出现"设置页显示成功、按下去没反应"（`global_hotkeys.rs` 文件头那条最难查的症状，Linux 侧同样要防）。
- 边界：档位判定 SHALL 在启动时做一次并缓存，之后不得因为某个调用失败而翻转档位（翻转会让"为什么这次不一样"无法解释）。
- 验收读数：H4 的实测记录（后端名、Vulkan 是否仍在呈现、CPU/GPU 无异常）落 `evidence.md`。

### REQ-002: 截图取图走门户，且失败要说话

WHEN 用户发起截图 THEN 系统 SHALL 调用 portal `Screenshot.Screenshot` 抓一张全屏帧并读出像素；`capture.rs:32` 的 `/usr/sbin/screencapture` SHALL 只留在 mac 分支。

- 正常路径：抓到的帧 SHALL 交给整屏定格（REQ-003），不自动翻译、不自动切页（沿用 CHG-009 那条行为定义）。
- 异常路径：门户缺席 / 用户拒授权 / 超时 → `Outcome::Unsupported` 换成带下一步的话（不许静默）；返回的 URI SHALL 读完即释放，SHALL NOT 把用户桌面内容落到历史目录之外。
- 权限：首次授权弹窗 SHALL 由门户自己管（不自己存图到临时目录、不进日志，NFR-3 同族）。
- 幂等：连按两次 SHALL 是两次独立定格，后一次不覆盖前一次的进行中状态（CHG-009 的 generation 语义在 Linux 侧同样成立——这正是那 19 条红测试里 `generation` 系列在测的东西）。
- 外部契约证明分层：**Mock**（假 DBus）能证明我们的状态机与超时分支；**本地真机**能证明 H1（我们的 app id 是否拿得到图）；两者不可互替，验收记录须注明是哪一种。

### REQ-003: 整屏定格与遮罩框选

WHEN 全屏帧到手 THEN 系统 SHALL 铺一扇无边框、盖满桌面并集的单子窗口显示这一帧，并在帧上完成拖拽框选（`cover.rs:89/136/157` 的 `covers:false` / `Focus{false,false}` / 空 `step_back` SHALL 换成实现）。

- 桌面并集 SHALL 用 `system::screen::union` 同一条公式，且 `screen.rs:153` 的 `probe()` 在 Linux SHALL 给出真几何（X11 `XRRGetMonitors` / `_NET_WORKAREA`），不再只走 `fallback_area(monitor_size)`——这是 `docs/traceability.md:74` 早就记下的缺口。
- 异常路径：多屏且跨屏框选 SHALL 允许（并集口径本来就允许）；单帧铺不满桌面时 SHALL 报缺屏而不是留黑块。
- 回归：我们自己不许出现在截下来的图里（CHG-009 目标 8）——Linux 侧"发起截图前先藏自己"这条时序（`background.rs` / `window.rs`）必须在 19 条红测试的 `capture_never_hides_our_own_window` 上被机器抓住。

### REQ-004: 钉屏贴图的窗口动作

WHEN 框选确认 THEN 系统 SHALL 把选区钉成一扇浮在屏幕上的窗：置顶、可拖到任意位置、可贴边收起、可 esc 关闭（`window.rs:12` 整模块的 mac 门 SHALL 打开到 X11 档）。

- 手段限定：X11 档用 `_NET_WM_STATE_ABOVE` + `MoveWindow`/`set_outer_position`；SHALL NOT 用需要合成的假设（GNOME 有合成器，但判定依据写的是 X11 协议可用，不是"应该有合成"）。
- 异常路径：窗口管理器不尊重 `_ABOVE` 时 SHALL 在设置页显示"当前桌面置顶不保证"，而不是把失败咽掉（`attach.rs:71/76`、`shadow.rs:36` 同理）。
- 边界：Wayland 档下置顶与移窗 SHALL 走"能开能用、不承诺位置"那一格（winit 的 `set_outer_position` 在 Wayland 是空函数——`wayland/window/mod.rs:273`/`:430` 为据）。

### REQ-005: 标注对象链（用户明令"含标注与遮罩链"）

WHEN 贴图已钉住、用户选中它 THEN 系统 SHALL 让空格唤出工具条、支持画笔/箭头/马赛克/文字对象的增删改与撤销，并把对象模型与坐标在 Linux 上与 mac 完全一致（同一份 `Action` 分派，不开第二条执行路径——`global_hotkeys.rs` 文件头"图钉那次两条通道各走各的，坏在细节里"的教训适用于本 REQ）。

- 正常路径：选中态、复制图像本体、拖选文字复制（文字层未就绪时不许假装能选）三格 SHALL 与 mac 同行为；**文字层在本轮因 OCR 不做而恒为"未就绪"**，界面话术 SHALL 说"这台机器上暂不识别文字"而不是无声。
- 异常路径：标注绘制所需的字体缺失时 SHALL 走 REQ-008 的回退链而不是 `VisionError::Font`（`ice-vision/paint.rs:52/76` 硬编码苹果路径 → Linux 上译文图片导出/贴图绘制现在必死，这一格是本包必须修的）。
- 回归：`state.rs:2790` 那条"空定格是失败不是取消"必须在 Linux 转绿。

### REQ-006: 全局热键（仅 X11 档）

WHEN 用户在别的应用前台按 F1/F2/F3 THEN 系统 SHALL 通过 X11 `XGrabKey` 命中并交回**既有那份** `Action` 分派（与 `hotkeys.rs` 共用枚举，不开第二条路径）。

- 泵的位置：与 mac 同规矩——帧循环收起时也要取得走（`take_action()` 挂在 `logic()` 的 D-21 理由在 Linux 同样成立）。
- 改键 SHALL 先解绑旧的（mac 侧 `-9878` 的教训）：X11 侧重复 grab 返回 `BadAccess`，同一格重复绑定 SHALL 先 `UngrabKey` 再绑，且这一条 SHALL 有纯函数级断言（`registration_plan` 那份跨平台复用，不改语义）。
- 异常路径：`BadAccess`（键被别人抓了）→ `HotkeyFailure::Taken` 那句"换一个组合"；Wayland 档 → `Unavailable` 那句"只能用图片页那颗按钮"。`explain()` 现有三条话术 SHALL 原样复用，新增失败形态 SHALL NOT 挤掉"下一步"这个要求。
- 边界（H5）：默认裸 F1/F2/F3 在本机能否抢到 SHALL 有实测读数；抢不到 SHALL 给出可用默认值建议，而不是保留一个按下去没反应的默认。

### REQ-007: 托盘

WHEN 应用在 Linux 运行 THEN 系统 SHALL 注册 `org.kde.StatusNotifierItem`（宿主在场，事实 5），菜单项与 mac 对齐，"关窗收进托盘"那条支路 SHALL 从 `TrayState::Unavailable`（`tray.rs:799-814`）换成真实现。

- 异常路径：宿主不存在（其它 DE）时 SHALL 回落现有最小化支，并让 `tray()` 状态在设置页可读——不许"以为在托盘里其实进程没了"。
- H2 未证实前，本 REQ 的第一条任务是真机冒烟（图标出现 + 一次菜单点击命中），不是先写实现。

### REQ-008: 字体与译文图片导出

WHEN Linux 需要渲染中文 THEN 系统 SHALL 按 `fc-match`/fontconfig 解析家族（首选 `Noto Sans CJK SC`，事实 7），SHALL NOT 假设任何苹果字体路径存在（`ice-core/src/font.rs:17/23`、`src/fonts.rs:49`、`ice-vision/paint.rs:52/76`）。

- 面选择：与 mac 同一条纪律——命中家族后 SHALL 校验落到 SC 面（`fc-match "Noto Sans CJK SC"` 与裸 `sans:lang=zh` 结果不同，实测如此）。
- 异常路径：一个中文字体都没有时 SHALL 报"缺字体，装 fonts-noto-cjk 后可用"这一句可行动的话，而不是 `VisionError::Font` 冒在链中间。
- 边界：`docs/05` §8 字体那行（"非 mac `install` 返回 `false`"）SHALL 随本 REQ 改写，两边不一致以 `docs/01-requirements.md` 为准并同步 Feature。

### REQ-009: 认窗列表分档（诚实降级）

WHEN 用户要"选一扇窗" THEN 系统 SHALL 用 X11 `_NET_CLIENT_LIST_STACKING` + `WM_CLASS`/`_NET_WM_NAME` 给出列表，并 SHALL 明示"Wayland 原生应用不在这张表里"（事实 6：本机 X11 侧只见 2~5 个）。

- 非目标重申：不做屏幕共享式全窗识别（那要 ScreenCast 门户 + PipeWire，见非目标表）。
- 异常路径：空列表 SHALL 是"这张表看不到当前桌面的窗口"这句话，而不是 `Vec::new()` 之后界面一片空白（`windows.rs:105` 现状）。

### REQ-010: 凭据、剪贴板与单实例

WHEN Linux 首次保存 API Key THEN 系统 SHALL 走 Secret Service（keyring 已带 `zbus-secret-service-keyring-store`，事实 10）。

- 密钥 SHALL NOT 进配置、日志、命令行参数、截图（ADR-005、NFR-3；mac 是钥匙串 `IceTranslator`/`ai-api-key`，Linux 侧 SHALL 用同一对标识语义，且本包任何证据文件 SHALL NOT 出现密钥值）。
- H3（Secret Service 是否应答）先查再定口径：查不到应答服务时 SHALL 明确"这台机器没有安全存储，密钥不落盘"而不是回退成明文配置文件。
- 剪贴板（arboard）与单实例（`cfg(unix)`）按事实 10 已跨平台，本 REQ 只要求**验证**并在证据里记"无需移植"，不许顺手重写。

### REQ-011: 能力探测取代硬编码

WHEN 界面问"这台机器能不能截图/全局热键/托盘" THEN 系统 SHALL 回答**当前会话实测结果**，SHALL NOT 读 `capability()` 里 `:51-59` 那两条写死的 false。

- 边界：探测结果 SHALL 是纯数据（可无头断言"档位→能力"这张表），SHALL NOT 需要真调 X11 才能被测试覆盖（19 条红转绿不能靠把测试关掉）。
- 回归：`native_ocr_supported()` 保持 `cfg!(target_os = "macos")`，本 REQ SHALL NOT 把它顺带改掉（用户明令本轮不做 OCR）。

### REQ-012: 门禁与三平台编译面

WHEN 本包收口 THEN `cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` SHALL 在 Linux 全绿，且 mac 侧 SHALL 至少 `cargo check --target aarch64-apple-darwin` 通过（不能把 mac 的门拆了补 Linux）。

- 判据：**F8 那 19 条红测试全部转绿**是完成线；任何一条靠 `#[cfg]` 跳过或删除来"转绿"SHALL 视为未完成（用户偏好：不许删自研功能）。
- 平台门纪律：新代码的 `#[cfg]` 组合 SHALL 在 Linux 与 mac 各自编译通过（本会话两次断裂的根因都是"顶层 use / link 属性没 gate"，`cargo check --all-targets` 是廉价防线）。

## 未决问题

阻塞项同步在 `change.json.questions`（Q-1/Q-2 阻塞，Q-3/Q-4 不阻塞）。

- **Q-1（阻塞·业务）**：本轮"截图取图"若门户首次授权弹窗在你机器上被拒或拿不到图（H1），是接受"截图入口在 Linux 隐藏 + 说明原因"，还是接受改走"你手动框选系统截图工具、我们读剪贴板"那种替代入口？两种是可不同的产品行为，我不替你选。
  - **批准时的口径（2026-11-24：用户批准本规格但未逐条回答这两问，由我按"对该未知免疫"选定，答错了改这里而不是改代码）**：走**隐藏截图入口 + 说明原因**，**不做**剪贴板替代入口。理由：替代入口要新增一条"读剪贴板判断这是不是截图"的产品行为，那是新需求而不是移植。反过来说——**若实测 H1 成立（门户给我们的 app 出图），这两问根本不会触发，本条自动作废**。
- **Q-2（阻塞·业务）**：走 XWayland 之后，托盘图标与窗口外观由 XWayland 主题呈现（可能与原生 Wayland 应用观感不同）。观感换功能是你已点头的方向，但**若置顶在某个 WM 下不生效**，你要的是"设置页显示不保证"还是"该功能入口直接隐藏"？
  - **批准时的口径（同上）**：**显示"当前桌面置顶不保证"，入口不隐藏**。理由：这与 REQ-001/REQ-004、以及 `global_hotkeys.rs` 文件头那条"最难查的症状＝设置页显示成功、按下去没反应"是同一条纪律——功能可能失效但必须可见；隐藏入口会让人以为我们从没做过这件事。
- **Q-3（不阻塞）**：全局热键要不要在二期上 portal `GlobalShortcuts`（接口在场、事实 2；本机权限库从未建表、事实 3 反面，且首次要弹窗）。本轮先只做 X11。
- **Q-4（不阻塞）**：19 条红测试里若有 genuinely 依赖 mac 语义的断言（读完实现才能判断），处理方式是"改成两档断言"还是"记入 Linux 档的行为差异并写进 `docs/07`"。

## 允许动作与停止条件

- **平台边界（用户 2026-11-24 追加的硬约束，逐字）**："**不要动 mac 和 win 的代码，你的改动只能涉及到 linux 版本的代码**。"
  本包的读法（写死在此，越界即停）：
  1. **行为影响面 SHALL 只在 Linux**。mac 与 Windows 分支的既有逻辑、常量、事件类型、话术、默认值 SHALL NOT 被改写、重排或删除；`windows.rs` 那个名字指的是"窗口列表"能力，与 Windows 平台无关，本包不碰它的 Win 侧口径。
  2. **允许两类"物理上改了共享文件"的编辑**：(a) 新增 `#[cfg(target_os = "linux")]` / `#[cfg(not(target_os = "macos"))]` 分支；(b) 给 mac 专属代码补平台门，让它不再在 Linux 编译。**两类的判据都是"mac 与 Win 上编译产物与行为逐字等价"**，证据是 `cargo check`/`cargo test` 在 mac target 侧的结果，而不是"我觉得没改到"。
  3. **公共层（`ice-core` 的业务规则、`layout.rs` 等纯数学、状态机语义）SHALL NOT 为 Linux 单独改行为**。需要分档时，用**新增**的探测/能力入口表达，而不是把公共逻辑改成 Linux 形状。
  4. Windows 平台在本包的档位是**维持现状**（NFR-7 的 `cargo check` 通过即可）；发现 Linux 改动会连带影响 Win 侧编译时，停下来交回用户，不自行重排。
- 批准本规格 = 允许在 Linux 上做设计、拆任务、实现与本机验证（含重启本机 GUI 实例、装本机字体/D-Bus 探测）。
- **不含**：任何部署、远端仓库写（push 需单独确认）、`docs/` 基线之外的口径改动、删除任何文件或旧证据（删前逐条问）、以及把密钥/截图证据提交进公开仓库（本仓公开：截图 SHALL 裁到仅含本应用窗口，逐张确认，`docs/03-ui-design.md` §5）。
- 与本包分开落的两件事（已在会话中改好、**待你点头即提交**）：`crates/ice-app/src/system/global_hotkeys.rs` 的 Carbon 平台门（36 行纯新增，0 删除）、`crates/ice-app/examples/screen_probe.rs` 的 objc2 顶层 use 门。它们是 NFR-7"Linux 能编译"范围内的修编译，不是新业务判断，所以不进 LNX-001 账本。
- 停止条件：外部契约推翻前提（例如门户拒给我们出图、`XGrabKey` 在本机被 Shell 全占）→ 记 SPEC_DEVIATION 退回 SPEC_REVIEW，不自行动业务写路径。

## 时间估算

依据：`F9` 那批降级口共 16 个文件点、`F2/F5` 已确认两条主路径可用（省掉选型调研）、H1~H5 五项要真机实测（每项含你在机器前的配合时间）。

- 开发：X11 窗口/贴边/热键 + 托盘 + 字体 + 认窗（成熟路径，无未知）≈ **3.5~5 小时**；截图门户链 + 遮罩框选 + 标注对象链接通（跨 `capture`/`cover`/`state`/`jobs` 四层，红测试集中在这里）≈ **4~6 小时**。合计 **7.5~11 小时**。
- 验证与落账（无头测试转绿、真机逐项留读数、`docs/05`/`docs/07`/traceability 同步）≈ **2~3 小时**。
- 外部等待（不计入开发工时）：门户首次授权、热键冲突试按、SNI 观感确认——需你在机器前，合计 **20~40 分钟**。
- 风险敞口：H1 不成立（门户不给图）会把 REQ-002/003 推向 Q-1 的替代形态，追加 **2~3 小时**；OCR 若下轮要做，另立 Feature，本包不预留。
