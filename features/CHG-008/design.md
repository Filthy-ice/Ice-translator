Change: CHG-008
Version: v0.2
Spec-Hash: 17c07165a63289435f85c7454210b5bb5d2733263c4690a7b4e32c9e7d5e9012

# CHG-008 · Design

- Change: CHG-008 · v0.1 · 已批准（2026-11-04 会话，`./sdd approve CHG-008` 由用户本人跑；批准范围=本设计七条 REQ，实现只动 `ice-app`）
- Repo: `ice-translator`（分支 `main`）
- 前置：CHG-004（P5）与 CHG-005（P6）都在 IMPLEMENTING，本包只改"关窗的去向 + 唤回的入口 + 托盘本身"，不带 OCR、回填、形态、吸附内容。代码态（2026-11-04）：**502 passed / 0 failed / 5 ignored**（ice-app 349 · ice-core 58 · ice-providers 62 · ice-vision 33）
- 需求侧本轮已先行改判并登记（CHG-001 REQ-004 的口径）：`docs/01-requirements.md` 的 FR-5.2 前半句与新增 FR-5.4、`docs/traceability.md` 两行、`docs/06` 的 TC-5.4-a/b/c 与 W-2.40/W-2.12、`docs/07` 变更登记与 §5 偏差关闭、`docs/02` 的 `system::tray` 行、`docs/03` §4、`docs/05` §8、`project/ROADMAP.md` 插入行

## 实现方案

### 模块划分

新增一个系统模块，改动集中在两个既有位置，其余全部复用。

| 位置 | 本包做什么 | 不做什么 |
|---|---|---|
| `crates/ice-app/src/system/tray.rs`（新建） | `install(mode, ctx, scale)`（幂等装出 `NSStatusItem`，mac 全部门内；`ctx` 存进模块当唤醒源，`scale` 决定 @1x/@2x 档位）、`sync(mode)`（每帧核对回调接线与菜单文案，沿用 `titlebar::sync` 的"问了比记着可靠"）、`take_outcome()`（取出一个 `Outcome::{Reveal, ToggleMode, Settings, Quit}`）、`mark(outcome)`（立意图 + `request_repaint`，回调与无头测试共用这一个入口）、`glyph_bytes(tier)`（纯函数，位图 → PNG 字节）、`menu_titles(mode)`（纯函数，四项文案，第二项给另一侧的名字）、`pop_menu()`（右键就地弹菜单，纯 AppKit，不经帧循环）。`#[cfg(not(target_os = "macos"))]` 那一支只导出同名函数并恒返回 `Unavailable` | 不读 `AppState`、不发 viewport 命令、不碰窗口——回调里只做"记一个意图"或"弹一个菜单" |
| `crates/ice-app/src/mode.rs` | `hide` 的分支：托盘可用 → `CancelClose` + `Visible(false)` + 策略 `.accessory`；不可用 → 保持今天的 `CancelClose` + `Minimized(true)`。`reveal` 的分支：先 `.regular` 再 `Visible(true)`，随后**照旧**重发 `commands_for_mode` + `target_size` + `pending.set(...)` + `apply_topmost` | 不动 `commands_for_mode`、`target_size`、`PendingGeometry` 三段护栏、`clamp_compact_rect`、`sanitize`（D-6/D-10/D-14 那三条护栏是 P5 的成果，本包只是换一个调用时机） |
| `crates/ice-app/src/app.rs` | **泵挂在 `App::logic`（v0.2 的更正）**：`logic()` 每帧调一次 `pump_tray(scale, tray::take_outcome(), ctx, &mut effects)`，里面按 `TrayState` 决定 `install` 还是 `sync`，并把取到的意图 `run()` 进去；`Action::{RevealWindow, OpenSettings}` 各接既有实现（`reveal`、`open_settings`），`ToggleMode`/`Quit` 复用快捷键那两条；隐藏期不许顺手 `request_repaint`（REQ-007） | **不在 `App::ui` 里碰托盘**（隐藏期 `ui()` 一次都不会被调，见那条 FACT）；不新增第二条退出/开设置实现、不改 `quit()` 的落盘顺序、不改 `close_requested`（`app.rs:1304-1313`）那一步的判定 |
| `crates/ice-app/src/views/settings.rs` | 「退出程序」旁边那句说明由 `tray::hint_text(available)` 出，不再写死 | 不加设置项、不动「恢复默认窗口尺寸」 |
| `crates/ice-app/src/state.rs` | `TrayState::{Unknown, Available, Unavailable}` 一个字段（运行态，**不进 config**） | 不加配置字段、不加持久化 |

`system::tray` 与 `system::titlebar`、`system::dblclick` 是同一个形状的第三个实例：**AppKit 回调只记意图，帧循环负责兑现**。这个形状不是为了整齐，是因为 `define_class!` 出来的方法体里既拿不到 `AppState` 也不该拿——拿不到的东西就不许假装能拿。

**两处例外，都是 v0.2 落定的**：
1. **兑现点必须是 `App::logic`，不是 `App::ui`**。窗口 `orderOut` 之后 eframe 走 `!show_ui` 那一支，`ui()` 一次都不被调（`glow_integration.rs:692-720`），把泵挂在 `ui()` 里 = 收起之后永远没人取意图（缺陷 D-21，用户 2026-11-04"关闭后，点击托盘还是没反应"）。`logic()` 在可见帧里也照跑（`epi_integration.rs:357`，就在 `ui()` 之前），所以一帧一次、两种状态都覆盖，`install` 需要的后备比例因子也从 `logic` 拿得到的那个 `Frame` 上读。
2. **"弹菜单"不记意图，直接在回调里做**。它不需要 `AppState`，是纯 AppKit；登记成旗子等下一帧，等于把一件原生的事塞进一条可能不跑的通道里。代价是要处理好追踪循环的重入（见 REQ-003 那段）。

### 依赖增量（八个 app-kit 开关 + 一个 foundation 开关）

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
装（一次，幂等）
install(mode, ctx, scale) ──► Retained<NSStatusItem>（强引用由模块内 thread_local 持有，
                                   setTarget: 是 weak，一放手就是野指针）
glyph(tier) ─► RgbaImage ─► PNG ─► NSData ─► NSImage ─► setTemplate(true) ─► button.setImage
menu_titles(mode) ─► NSMenu 四项（第二项按当前形态给**另一侧**的名字）

每帧核对 + 兑现（都在 `App::logic`，窗口可见与不可见都跑 —— v0.2 的更正）
sync(mode)：按钮的 target/action 不是我们那对就重接；菜单 title 与 menu_titles 不一致就重写
pump_tray(scale, take_outcome(), ctx, effects)：
    ├ Reveal     → Action::RevealWindow → mode::reveal
    ├ ToggleMode → Action::ToggleMode   （与 Cmd+Shift+M 落同一条）
    ├ Settings   → Action::OpenSettings → reveal + app::open_settings（第三个可见入口，不是第二条实现）
    └ Quit       → Action::Quit         （与设置页那颗、Cmd+Q 落同一条）

点击（时机由 AppKit 决定，回调里只做两件事）
button action ─► currentEvent().r#type()
    ├ RightMouseUp → pop_menu()      ← 同步弹，**不等下一帧**；追踪循环返回后 setMenu(None) 并核对接线
    └ 其它         → mark(Reveal) + request_repaint()
菜单四项的 action ─► mark(对应意图) + request_repaint()

关窗：红灯 / Cmd+W → Action::HideWindow → mode::hide(TrayState)
        Available  → CancelClose + Visible(false) + setActivationPolicy(.Accessory)
        Unavailable→ CancelClose + Minimized(true)            ← 今天这条路，一字不改地留着
唤回：托盘 → Action::RevealWindow → setActivationPolicy(.Regular) + activate
        → Visible(true) → Focus → 重发本形态几何 → capture 尺寸槽
```

`request_repaint()` 那一步是整条链的承重墙：窗口 `orderOut` 之后不再收 `RedrawRequested`，但 eframe 0.37 在 `run.rs:224-250` 会**直接**给不可见窗跑一次 `update()`（它自己的注释写着目的是"确保 `Visible(true)` 这类 viewport 命令仍被处理"），节流到 100ms（`run.rs:376-383`）。所以回调里那一次 `request_repaint` 足够把帧循环叫醒，而不需要常驻事件监视。**v0.2 把这一句改准**：那一次 `update()` 里 `show_ui` 仍按可见性判 false，走的是 `update_logic_only`，**跑 `logic()`、不绘制**——这正是泵必须挂在 `logic` 的原因，也是"隐藏期一次重绘烧的是 logic 而不是画帧"的依据。

---

## REQ-001 实现与验证

**做**：`install()` 内部先查模块级 `thread_local!` 里那枚 `Retained<NSStatusItem>`；有就直接返回上次的 `Outcome`，没有才走 `NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength)`。装成功 → `state.tray = Available`；拿不到 `MainThreadMarker` 或按钮为空 → `Unavailable` + 一次 WARN（只有 `stage`、`reason`、错误码一类计数）。装失败**不写死**：`install()` 每帧被调，条件凑齐就补装一次，装成功那一帧起 `Available`。

**为什么不是"关窗时才装"**：FR-5.4 第①句是"进程活着就在"。

**验证**：无头测 `install` 的状态迁移（`TrayState` 那个纯状态机：Unknown → Available/Unavailable，重复 install 不改变 Available）与非 mac 分支恒 `Unavailable`；真机判"菜单栏真的有一枚"（W-2.40 第一步）。

## REQ-002 实现与验证

**做**：`glyph_bytes(tier: Tier) -> Vec<u8>` 完全不碰 AppKit——按档位（@1x 高 22、@2x 44）开一张 `RgbaImage`，全部像素 RGB 写 0、alpha 由一组线段按点到线段距离算覆盖；`image` 编码成 PNG 字节。mac 侧再走 `NSData` → `NSImage::initWithData` → `setTemplate(true)` → `button.setImage`，并把 `NSImage` 的 `size` 设成点尺寸（22×22），这样视网膜下用的是那张 44px 的位图。

**形状（v0.2 已定论）**：应用图标那两只气泡。线段表就三样东西——两只 `Bubble`（`FRONT` 左上、`BACK` 右下，各带 2.4 点圆角）+ `tails()` 那四条尾巴 + 一个 `OVERLAP_GAP = 0.8` 的外扩遮挡参数。画序是**先后再前**：先画 `BACK`，但凡是落在 `inflated(FRONT, OVERLAP_GAP)` 内的像素一律不刷（两只都画满成一个"8"字就没了前后关系），再画两条尾巴，最后整只 `FRONT` 压上来。坐标用 AppKit 那套（原点左下、y 向上、22×22 点），`to_image(p, scale) = (x·scale, (22 − y)·scale)` 翻一次成图像坐标。**这套表可以整块换掉，TC-5.4-a 那四条判据不动**——这也是 spec 一开始就说的"改形状只动 `glyph()`"。

**为什么不复用 Dock 那张**：彩色方砖在 18pt 下形状读不出，而且 `setTemplate` 的语义就是"用 alpha 当蒙版、由系统上色"，给它一张彩图等于既要形状又要颜色，系统只能按 alpha 走，结果是一团灰。

**验证（TC-5.4-a，四条全无头）**：① 边长 = 档位高度（@1x 22 / @2x 44）；② 所有 alpha>0 的像素 RGB 为 (0,0,0)；③ 四角 alpha = 0；④ 非透明包围盒水平居中（左右留白差 ≤1px）、上下各留 ≥1px。"认得出"只有真机（W-2.40）。

## REQ-003 实现与验证

**做**：`define_class!` 一个只有方法体的 `IceTranslatorTrayTarget`（父类 `NSObject`），五个选择器：`iceTrayClicked:`（按钮的左/右键）与 `iceTrayShowWindow:` / `iceTrayToggleMode:` / `iceTraySettings:` / `iceTrayQuit:`（菜单四项）。除右键那一支以外，方法体只做一件事——`mark(Outcome::X)`：把意图写进 `AtomicU8`（一个槽，不是四个 bool：一次回调只可能立一个意图），再 `request_repaint()` 把睡着的事件循环叫醒。接线：`button.setTarget(&target)` + `setAction(sel!(iceTrayClicked:))` + `sendActionOn(LeftMouseUp | RightMouseUp)`。**target 的强引用存在 `thread_local!`**，理由与 `titlebar.rs:18-20` 那条完全一样（`setTarget:` 是 weak/unsafe_unretained，我们一放手就是野指针）。

**泵的位置（D-21 的落点）**：`take_outcome()` 由 `app.rs` 的 `logic()` 每帧调一次，映射成 `Action` 后走既有 `run()`。**不是 `ui()`**——隐藏期 `ui()` 一次都不被调，v0.1 正是挂在那里，于是"托盘点了没反应"。`install` 需要的 `scale_factor` 也从 `logic(ctx, frame)` 那个 `Frame` 上读（`frame.winit_window()` 取不到时按 1.0 取 @1x，非 mac 分支根本不进这一段）。

菜单四项在装的时候一次性建好（`NSMenu::new` + 四条 `initWithTitle_action_keyEquivalent`，`keyEquivalent` 全给空串），`setAutoenablesItems(false)`；`Quit`、`ToggleMode`、`Settings` 三项各走同一个 target 的不同选择器，第一项 `显示窗口` 与左键立**同一个** `Reveal` 意图。右键那一支不常驻挂菜单：**直接在回调里** `item.popUpStatusItemMenu(&menu)`，返回后立刻 `setMenu(None)` 并核对按钮接线还在不在（`is_ours` 不对就重 `hook`）——菜单一挂上去左键就不走 action（Q-001），而"先记旗子、下一帧再弹"在隐藏期等于永远不弹。`popUpStatusItemMenu:` 自 10.14 标 deprecated，替代方案（挂 `menu` 属性 + 左键 `performClick:`）正好踩回 Q-001 那个坑，故 `#[allow(deprecated)]` 局部放行并在这里写明理由。**追踪循环是阻塞的**，所以弹之前要把 `item / menu / button / target` 那四个 `Retained` 从 `thread_local!` 里克隆出来、**先释放 `RefCell` 的借用**，否则循环期间重进同一个 `TRAY.with(..)` 就是双重借用 panic。

第二项的文案是 `menu_titles(mode)` 的纯函数结果（全量 → `精简`，简洁 → `全量`），切换形态后菜单要跟着改口：`sync()` 每帧核对那串 `title`，不一致才重写（同 `titlebar` 核对 tooltip 的做法）。

**验证（TC-5.4-b，无头）**：`menu_titles` 两态各一组断言（`["显示窗口", "精简", "设置", "退出程序"]` / `["显示窗口", "全量", "设置", "退出程序"]`，并断言两支都不含"悬浮窗"）；四个意图"取走即清零、连点只兑现一次"（复用 `one_click_is_handed_over_exactly_once` 的形状）；意图 → `Action` 的映射表（`Quit` → 既有的 `Action::Quit`、`Settings` → 既有的 `Action::OpenSettings`，都不许是第二条实现）；**`pump_tray` 在 `state.hidden == true` 时交进意图，窗口要真的回来**（`an_intent_handed_over_while_hidden_still_opens_the_window`，v0.2 新增——D-21 的回归锁）。左右键真的分得开只有真机（W-2.40，失败走停止条件 1；2026-11-04 已定论）。

## REQ-004 实现与验证

**做**：`mode::hide(ctx, state, tray)` 按 `TrayState` 二选一地发命令；`mode::reveal(ctx, state, pending, tray)` 可用时先 `setActivationPolicy(.Regular)` 再 `Visible(true)`，随后**沿用今天这一段**：`state.hidden = false`、`state.filled = false`、`commands_for_mode` + `target_size` 全量重发、`pending.set(...)`、`apply_topmost`、`request_repaint`。策略切换调用点在 `system::tray`（`set_policy(available: bool)` 一个函数），`mode` 只决定何时调，因为策略是原生对象的操作、`mode` 是纯命令编排——分层不让 `mode.rs` 出现 objc 类型。

`Q-004` 那一支在 v0.2 落到**强制激活**：`set_policy(regular = true)` 内部切完 `.regular` 就调 `activateIgnoringOtherApps(true)`（`#[allow(deprecated)]`，理由写在 `tray.rs` 那段——非 deprecated 的 `activate()` 是协作式的，不保证真的激活），所以调用次序是 `set_policy(true)` → `Visible(true)` → `ViewportCommand::Focus`。**`Focus` 必须排在 `Visible(true)` 之后、同一批命令里**：egui 对 `Focus` 的说明是"窗口不可见或最小化时没有效果"，顺序反过来是空操作。`Unavailable` 那一支同样补一发 `Focus`，排在 `Minimized(false)` 之后。真机若嫌抢焦点讨厌，删的是 `set_policy` 里那一句、不是 `Focus`。

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

**做**：`state.hidden == true` 时 `logic()` 里三件事——不发 `request_repaint()`（提示条心跳、动画续期在隐藏期一律停）、不重算布局相关的脏标记、只保留"取托盘意图"这一条唤醒源。回调里的 `request_repaint()` 是**唯一**允许的续期来源。泵挂在 `logic` 不违反这条：`take_outcome()` 读的是 `AtomicU8`，槽空时**什么都不发**，所以每帧多跑一次泵不等于每帧多一次重绘；托盘档还有一条配套——`RevealGuard` 那条靠几何/焦点猜的唤醒路径整体退役，否则"隐藏期不许有心跳"和"两条都能唤回"会互相打脸（`the_tray_branch_leaves_no_second_wakeup_path` 一次锁住这两件事）。

**验证**：无头测"隐藏帧序列里 `request_repaint` 次数 == 意图数"（用 `egui::Context` 的 repaint 计数或把 `update()` 直接喂几帧数命令）；`< 1%` 这条数只有真机能定论——取证方式：关窗后静置十分钟，`ps -o %cpu` 采样与 `nettop` 一样脱敏进 `./sdd record` 的附件（V-009），采不到就写"未采到"，不推断。

---

## 影响与回滚

- **影响面**：`ice-app` 一个包（新模块 `system::tray.rs` + `mode.rs` 的 `hide`/`reveal` 两支 + `app.rs` 的帧首与 `Action::RevealWindow` + `state.rs` 一个运行态字段 + `views/settings.rs` 一句文案 + `Cargo.toml` 九个 feature）。`ice-core` / `ice-providers` / `ice-vision` **零改动**，配置 schema **零改动**。
- **既有行为不回退清单**：形态切换（含标题栏绿键改道）、边缘停靠、置顶、主题三态、失焦半透明、双击标题栏铺满、单实例锁、`Cmd+Q` 落盘顺序、提示条淡出。D-6/D-10 的 `PendingGeometry` 三段护栏必须继续生效——`reveal` 重发几何正是它的消费者之一。
- **回滚**：`TrayState` 恒为 `Unavailable` 即整体退回今天的行为（隐藏 = 最小化、唤醒 = `RevealGuard`），代码不需要额外开关；`Cargo.toml` 那九个 feature 撤掉就编译不过 `tray.rs`，所以按 `#[cfg(target_os = "macos")]` 整文件门住，非 mac 面零影响。

## 验证层次

- **无头能定论的（V-001/V-002）**：位图四条判据、菜单四项的文案与动作映射、意图取走即清零、**收起期交来的意图照样兑现**（v0.2 补的那条回归锁）、`TrayState` 两支的命令序列（含"不发 `Minimized(true)`""不发 `Visible(false)`"这两条否定式）、`reveal` 幂等、隐藏期不续期、非 mac 编译面不引用 mac 符号。
- **只有真机能定论的（V-009 + V-010）**：菜单栏里真的有一枚、深浅色下都看得见、左键唤回与右键菜单**分得开**、Dock 磁贴真的消失与回来、唤回后内容与尺寸原样、隐藏十分钟的 CPU 数字。**2026-11-04 覆盖到的**：左键唤回、右键出菜单、菜单可用（"托盘的可以了，没问题"）；**没单独定论的**：磁贴消失/回来、深浅色两档、唤回后尺寸原样、十分钟 CPU——V-010 据实记成部分覆盖。
- **只有用户能定论的（V-010）**：图标"认得出是这个应用"（v0.2 已定论：改成应用图标的双气泡之后用户认可托盘这一组）、菜单四项的 wording 顺不顺、这套关窗语义是不是你想要的。
- **不进本包验证的**：非 macOS 的托盘（NFR-7 待办）、打包形态下的激活策略基准（P8）、`Reopen` 事件那条路（本包不依赖它）。
