Change: CHG-008
Version: v0.1
Spec-Hash: UNAPPROVED

# CHG-008 · Design

- Change: CHG-008 · v0.1 · 待用户批准（`./sdd approve CHG-008` 由用户本人跑；批准前本包只动规格与设计，不动 `mode::hide`/`reveal` 与任何原生代码）
- Repo: `ice-translator`（分支 `main`）
- 前置：CHG-004（P5）与 CHG-005（P6）都在 IMPLEMENTING，本包只改"关窗的去向 + 唤回的入口 + 托盘本身"，不带 OCR、回填、形态、吸附内容。代码态（2026-11-04）：**502 passed / 0 failed / 5 ignored**（ice-app 349 · ice-core 58 · ice-providers 62 · ice-vision 33）
- 需求侧本轮已先行改判并登记（CHG-001 REQ-004 的口径）：`docs/01-requirements.md` 的 FR-5.2 前半句与新增 FR-5.4、`docs/traceability.md` 两行、`docs/06` 的 TC-5.4-a/b/c 与 W-2.40/W-2.12、`docs/07` 变更登记与 §5 偏差关闭、`docs/02` 的 `system::tray` 行、`docs/03` §4、`docs/05` §8、`project/ROADMAP.md` 插入行

## 实现方案

### 模块划分

新增一个系统模块，改动集中在两个既有位置，其余全部复用。

| 位置 | 本包做什么 | 不做什么 |
|---|---|---|
| `crates/ice-app/src/system/tray.rs`（新建） | `install()`（幂等装出 `NSStatusItem`，mac 全部门内）、`sync(frame)`（每帧核对回调接线现状，沿用 `titlebar::sync` 的"问了比记着可靠"）、`take_outcome()`（取出一个 `Outcome::{Reveal, ToggleMode, Quit}`）、`glyph_bytes(tier)`（纯函数，位图 → PNG 字节）、`menu_titles(mode)`（纯函数，第二项给另一侧的名字）。`#[cfg(not(target_os = "macos"))]` 那一支只导出同名函数并恒返回 `Unavailable` | 不读 `AppState`、不发 viewport 命令、不碰窗口——回调里只做"记一个意图" |
| `crates/ice-app/src/mode.rs` | `hide` 的分支：托盘可用 → `CancelClose` + `Visible(false)` + 策略 `.accessory`；不可用 → 保持今天的 `CancelClose` + `Minimized(true)`。`reveal` 的分支：先 `.regular` 再 `Visible(true)`，随后**照旧**重发 `commands_for_mode` + `target_size` + `pending.set(...)` + `apply_topmost` | 不动 `commands_for_mode`、`target_size`、`PendingGeometry` 三段护栏、`clamp_compact_rect`、`sanitize`（D-6/D-10/D-14 那三条护栏是 P5 的成果，本包只是换一个调用时机） |
| `crates/ice-app/src/app.rs` | 帧首 `tray::install()` 一次 + `tray::sync`；`take_outcome()` 映射成 `Action::{RevealWindow, ToggleMode, Quit}` 走既有 `run()` 分派；`Action::RevealWindow` 新增一条并接 `mode::reveal`；隐藏期不许顺手 `request_repaint`（REQ-007） | 不新增第二条退出实现、不改 `quit()` 的落盘顺序、不改 `close_requested`（`app.rs:1304-1313`）那一步的判定 |
| `crates/ice-app/src/views/settings.rs` | 「退出程序」旁边那句说明由 `tray::hint_text(available)` 出，不再写死 | 不加设置项、不动「恢复默认窗口尺寸」 |
| `crates/ice-app/src/state.rs` | `TrayState::{Unknown, Available, Unavailable}` 一个字段（运行态，**不进 config**） | 不加配置字段、不加持久化 |

`system::tray` 与 `system::titlebar`、`system::dblclick` 是同一个形状的第三个实例：**AppKit 回调只记意图，帧循环负责兑现**。这个形状不是为了整齐，是因为 `define_class!` 出来的方法体里既拿不到 `AppState` 也不该拿——拿不到的东西就不许假装能拿。

### 依赖增量（批准前要先看清这一段）

`crates/ice-app/Cargo.toml` 现在这串 feature 是变更 K 之后回到最小集合的结果（注释里写明了为什么不开 default：default 会打开约 283 个模块，连带把 `objc2-cloud-kit`/`core-video`/`core-text`/`metal` 等六个 Apple 绑定 crate 拖进编译图）。本包要加**九个开关**，每一个都有编译面证据，逐个写进那段注释：

| feature | 开在哪 | 为什么必须有它 |
|---|---|---|
| `NSStatusBar` | app-kit | `systemStatusBar()` 与 `statusItemWithLength:` 的家（`NSStatusBar.rs:32-39`） |
| `NSStatusItem` | app-kit | 状态项本体；`statusItemWithLength:` 的返回类型、`setMenu:`、`button()` 都挂在它里面（`NSStatusItem.rs:36-86`） |
| `NSStatusBarButton` | app-kit | 左/右键接线要拿到那枚按钮；`button()` 的 cfg 里明确列了它（`NSStatusItem.rs:76-86`） |
| `NSResponder` | app-kit | 同上一条 cfg 的一环——`NSStatusBarButton : NSButton : NSControl : NSView : NSResponder`，父类链缺它就编译不出 `Deref` |
| `NSMenu` | app-kit | `setMenu:` / `menu()` 的门（`NSStatusItem.rs:66-75`）与 `NSMenu::new(mtm)`、`addItem:` |
| `NSMenuItem` | app-kit | `addItem:` 的门 + `initWithTitle:action:keyEquivalent:` |
| `NSImage` | app-kit | `NSButton::setImage:` 的 cfg 门（`NSButton.rs:446-450`）与 `setTemplate:` |
| **`NSRunningApplication`** | app-kit | **最容易漏的一个**：`setActivationPolicy:` 整个方法 `#[cfg(feature = "NSRunningApplication")]`（`NSApplication.rs:746-755`），三个策略常量也定义在 `NSRunningApplication.rs:47-56`。不开就没有"关掉 Dock 磁贴"这件事 |
| `NSData` | foundation | `NSImage::initWithData:` 的参数类型（`NSImage.rs:192-194`） |

**不新增任何 crate 名**：`objc2-app-kit` 与 `objc2-foundation` 都已在依赖里；`image` 的 PNG 编码已由 workspace feature 打开（`Cargo.toml:27`）；`block2`/`objc2-core-foundation` 已在。**不引 `tray-icon` 一类第三方**（spec 的"不允许"里写死了）。

### 数据流（一条链，两个方向）

```
装（一次，幂等）        每帧核对                 点击（AppKit 线程时机）        帧循环兑现
install() ──► Retained  sync() ──► 按钮的        button action ──► 读           take_outcome()
  的 NSStatusItem        target/action 是不是     currentEvent().type()          │
                       我们那对（不是就接）        ├ RightMouseUp → 临时挂菜单     ├ Reveal    → Action::RevealWindow → mode::reveal
glyph_bytes() ─► PNG   弱引用由 WEIRD 强引用      └ 其它 → 只立 Reveal 意图        ├ ToggleMode→ Action::ToggleMode（与 Cmd+Shift+M 同一条）
  ─► NSData ─► NSImage                            + request_repaint()             └ Quit      → Action::Quit（与设置页那颗、Cmd+Q 同一条）
  ─► setTemplate(true)
  ─► button.setImage

关窗：红灯 / Cmd+W → Action::HideWindow → mode::hide(TrayState)
        Available  → CancelClose + Visible(false) + setActivationPolicy(.Accessory)
        Unavailable→ CancelClose + Minimized(true)            ← 今天这条路，一字不改地留着
唤回：托盘 → Action::RevealWindow → setActivationPolicy(.Regular) → Visible(true) → 重发本形态几何 → capture 尺寸槽
```

`request_repaint()` 那一步是整条链的承重墙：窗口 `orderOut` 之后不再收 `RedrawRequested`，但 eframe 0.37 在 `run.rs:244-250` 会**直接**给不可见窗跑一次 `run_ui_and_paint`（它自己的注释写着目的是"确保 `Visible(true)` 这类 viewport 命令仍被处理"），节流到 100ms（`run.rs:26-28`）。所以回调里那一次 `request_repaint` 足够把帧循环叫醒，而不需要常驻事件监视。

---

## REQ-001 实现与验证

**做**：`install()` 内部先查模块级 `thread_local!` 里那枚 `Retained<NSStatusItem>`；有就直接返回上次的 `Outcome`，没有才走 `NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength)`。装成功 → `state.tray = Available`；拿不到 `MainThreadMarker` 或按钮为空 → `Unavailable` + 一次 WARN（只有 `stage`、`reason`、错误码一类计数）。装失败**不写死**：`install()` 每帧被调，条件凑齐就补装一次，装成功那一帧起 `Available`。

**为什么不是"关窗时才装"**：FR-5.4 第①句是"进程活着就在"。

**验证**：无头测 `install` 的状态迁移（`TrayState` 那个纯状态机：Unknown → Available/Unavailable，重复 install 不改变 Available）与非 mac 分支恒 `Unavailable`；真机判"菜单栏真的有一枚"（W-2.40 第一步）。

## REQ-002 实现与验证

**做**：`glyph_bytes(tier: Tier) -> Vec<u8>` 完全不碰 AppKit——按档位（@1x 高 22、@2x 44）开一张 `RgbaImage`，全部像素 RGB 写 0、alpha 由一组线段（窗口轮廓 + 往里收的折角）按点到线段距离算覆盖；`image` 编码成 PNG 字节。mac 侧再走 `NSData` → `NSImage::initWithData` → `setTemplate(true)` → `button.setImage`，并把 `NSImage` 的 `size` 设成点尺寸（22×22），这样视网膜下用的是那张 44px 的位图。

**为什么不复用 Dock 那张**：彩色方砖在 18pt 下形状读不出，而且 `setTemplate` 的语义就是"用 alpha 当蒙版、由系统上色"，给它一张彩图等于既要形状又要颜色，系统只能按 alpha 走，结果是一团灰。

**验证（TC-5.4-a，四条全无头）**：① 边长 = 档位高度（@1x 22 / @2x 44）；② 所有 alpha>0 的像素 RGB 为 (0,0,0)；③ 四角 alpha = 0；④ 非透明包围盒水平居中（左右留白差 ≤1px）、上下各留 ≥1px。"认得出"只有真机（W-2.40）。

## REQ-003 实现与验证

**做**：`define_class!` 一个只有方法体的 `IceTranslatorTrayTarget`（父类 `NSObject`），方法 `iceTrayClicked:` 里做三件事——读 `NSApplication::sharedApplication(mtm).currentEvent()` 的事件类型、把对应意图写进 `AtomicU8`（一个足够表达三态的槽，不是三个 bool：一次回调只可能立一个意图）、`request_repaint()`。接线：`button.setTarget(&target)` + `setAction(sel!(iceTrayClicked:))` + `sendActionOn(LeftMouseUp | RightMouseUp)`。**target 的强引用存在 `thread_local!`**，理由与 `titlebar.rs:18-20` 那条完全一样（`setTarget:` 是 weak/unsafe_unretained，我们一放手就是野指针）。

菜单三项在装的时候一次性建好（`NSMenu::new` + 三条 `initWithTitle_action_keyEquivalent`，`keyEquivalent` 全给空串），`setAutoenablesItems(false)`；`Quit` 与 `ToggleMode` 两项的 action 走同一个 target、不同的选择器，第三项 `显示窗口` 与左键立**同一个** `Reveal` 槽。右键那一支不常驻挂菜单：只有右键成立时才临时 `statusItem.setMenu(&menu)` + `button.performClick()`，出完菜单立刻 `setMenu(None)`——菜单一挂上去左键就不走 action（Q-001）。

第二项的文案是 `menu_titles(mode)` 的纯函数结果（全量 → `精简`，简洁 → `全量`），切换形态后菜单要跟着改口：`sync()` 每帧核对那串 `title`，不一致才重写（同 `titlebar` 核对 tooltip 的做法）。

**验证（TC-5.4-b，无头）**：`menu_titles` 两态各一组断言；三个意图"取走即清零、连点只兑现一次"（复用 `one_click_is_handed_over_exactly_once` 的形状）；意图 → `Action` 的映射表（`Quit` 映射到的必须是既有的 `Action::Quit`，不许新增第二条退出实现）。左右键真的分得开只有真机（W-2.40，失败走停止条件 1）。

## REQ-004 实现与验证

**做**：`mode::hide(ctx, state, tray)` 按 `TrayState` 二选一地发命令；`mode::reveal(ctx, state, pending, tray)` 可用时先 `setActivationPolicy(.Regular)` 再 `Visible(true)`，随后**沿用今天这一段**：`state.hidden = false`、`state.filled = false`、`commands_for_mode` + `target_size` 全量重发、`pending.set(...)`、`apply_topmost`、`request_repaint`。策略切换调用点在 `system::tray`（`set_policy(available: bool)` 一个函数），`mode` 只决定何时调，因为策略是原生对象的操作、`mode` 是纯命令编排——分层不让 `mode.rs` 出现 objc 类型。

`Q-004` 那句 `NSApp.activate()` 放在 `set_policy(regular)` 之后、`Visible(true)` 之前；真机若抢焦点讨厌，删这一句就够，不影响其它判据。

`RevealGuard`：托盘可用路径上**退役**（不再需要猜"回来了没有"），代码留在 `TrayState::Unavailable` 那一支继续兜旧语义。`app.rs` 的心跳调用按 `TrayState` 走一条分支，不留两条并行唤醒路径。

**验证（TC-5.4-c 的一半，无头）**：命令序列断言——`Available` 时发的队列里 SHALL 有 `CancelClose` 与 `Visible(false)`、SHALL NOT 有 `Minimized(true)`；`Unavailable` 时 SHALL 反之；`reveal` 两次与一次的命令队列相同（幂等）；`reveal` 后 `state.hidden == false` 且尺寸槽被 `capture` 过的既有断言不回退。磁贴真的消失/回来只有真机（W-2.40）。

**重写的既有测试**：`mode_cancels_close_and_minimizes`（`mode.rs:1280`）、`hide_window_keeps_the_process_and_the_content`（`app.rs:1483`）按新语义各写两支（可用/不可用），"进程不退、内容原样"这两条断言原样搬过去；那四条 `RevealGuard` 测试（`mode.rs:1298-1350`）保留但改挂在 `Unavailable` 支上。

## REQ-005 实现与验证

**做**：设置页那句说明改成 `tray::hint_text(TrayState)`：`Available` → "红灯/Cmd+W 是收进托盘，不是退出"，`Unavailable` → "红灯/Cmd+W 是收进后台，不是退出"。文案在 `system::tray` 里（与降级语义同处），`views/settings.rs` 只调用。

**验证**：无头断言两支的文案不同且都不含"最小化"；`Cmd+Q`/设置页那颗/托盘那颗三条入口共指 `Action::Quit` 由 REQ-003 的映射表断言覆盖。观感走 W-2.26 复核。

## REQ-006 实现与验证

**做**：`TrayState` 是唯一分叉点，`hide`/`reveal`/心跳/文案四个消费者都读它，`cfg!(target_os)` 只出现在 `system::tray` 内部（决定它能不能变成 `Available`）。装不上时界面给一条提示条（既有 `state.toast` + `TOAST_LIFETIME` 规则，不带给关闭按钮、不弹窗）。

**为什么这条最硬**：混合状态 = 窗口隐藏了、磁贴被 `.accessory` 关了、唤醒入口又没装上，用户就只有"杀掉进程"一条路——这正是 2026-10-27 用户那句"我没有办法重启软件"。所以判据是**否定式**的：SHALL NOT 出现 `Visible(false)` 与 `Minimized(true)` 同时不发、或发了前者而托盘 `Unavailable`。

**验证**：无头测四种 `(TrayState, 动作)` 组合的命令队列；`cfg` 面断言"非 mac 编译面不引用 `tray.rs` 的 mac 模块任何符号"。真机上 macOS 无法构造"拔不掉托盘"，故跨平台降级实测属 P8。

## REQ-007 实现与验证

**做**：`state.hidden == true` 时 `update()` 里三件事——不发 `request_repaint()`（提示条心跳、动画续期在隐藏期一律停）、不重算布局相关的脏标记、只保留"取托盘意图"这一条唤醒源。回调里的 `request_repaint()` 是**唯一**允许的续期来源。

**验证**：无头测"隐藏帧序列里 `request_repaint` 次数 == 意图数"（用 `egui::Context` 的 repaint 计数或把 `update()` 直接喂几帧数命令）；`< 1%` 这条数只有真机能定论——取证方式：关窗后静置十分钟，`ps -o %cpu` 采样与 `nettop` 一样脱敏进 `./sdd record` 的附件（V-009），采不到就写"未采到"，不推断。

---

## 影响与回滚

- **影响面**：`ice-app` 一个包（新模块 `system::tray.rs` + `mode.rs` 的 `hide`/`reveal` 两支 + `app.rs` 的帧首与 `Action::RevealWindow` + `state.rs` 一个运行态字段 + `views/settings.rs` 一句文案 + `Cargo.toml` 九个 feature）。`ice-core` / `ice-providers` / `ice-vision` **零改动**，配置 schema **零改动**。
- **既有行为不回退清单**：形态切换（含标题栏绿键改道）、边缘停靠、置顶、主题三态、失焦半透明、双击标题栏铺满、单实例锁、`Cmd+Q` 落盘顺序、提示条淡出。D-6/D-10 的 `PendingGeometry` 三段护栏必须继续生效——`reveal` 重发几何正是它的消费者之一。
- **回滚**：`TrayState` 恒为 `Unavailable` 即整体退回今天的行为（隐藏 = 最小化、唤醒 = `RevealGuard`），代码不需要额外开关；`Cargo.toml` 那九个 feature 撤掉就编译不过 `tray.rs`，所以按 `#[cfg(target_os = "macos")]` 整文件门住，非 mac 面零影响。

## 验证层次

- **无头能定论的（V-001/V-002）**：位图四条判据、菜单三项的文案与动作映射、意图取走即清零、`TrayState` 两支的命令序列（含"不发 `Minimized(true)`""不发 `Visible(false)`"这两条否定式）、`reveal` 幂等、隐藏期不续期、非 mac 编译面不引用 mac 符号。
- **只有真机能定论的（V-009 + V-010）**：菜单栏里真的有一枚、深浅色下都看得见、左键唤回与右键菜单**分得开**、Dock 磁贴真的消失与回来、唤回后内容与尺寸原样、隐藏十分钟的 CPU 数字。
- **只有用户能定论的（V-010）**：图标"认得出是这个应用"、菜单三项的 wording 顺不顺、这套关窗语义是不是你想要的。
- **不进本包验证的**：非 macOS 的托盘（NFR-7 待办）、打包形态下的激活策略基准（P8）、`Reopen` 事件那条路（本包不依赖它）。
