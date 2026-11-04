# CHG-008 · 菜单栏托盘与关窗收进托盘：FR-5.4 新增、FR-5.2 前半句按用户改判

## 原始意图与来源

- 立项要求（2026-11-04，用户原话，同一轮反馈的第 2 条）："我们的窗口点击关闭时不是缩小到托盘，而是进入程序坞，我不知道别的系统里我们的软件怎么样，但是应该都可以做到右上角托盘图标吧。"（同轮第 1 条"Dock 图标明显大一圈"已在本轮由 `main::app_icon` 的网格留白修掉，登记为缺陷 D-19，**不进本包**——它改的是交接那一步的像素，不是关窗的去向。）
- 三个裁定（2026-11-04 同一轮，用户在三档/四档选项里点的）：**① 关窗后 Dock 磁贴也消失**（三档里的中间档：不是"保留磁贴"，也不是"全程只做菜单栏应用"）；**② 左键唤回、右键出菜单**，v0.1 写的是菜单三项 = `显示窗口` / `精简 ↔ 全量` / `退出程序`（**本条已由 v0.2 改判为四项，见下一节**）；**③ 本轮只做 macOS**，Windows 的 `Shell_NotifyIcon` 与 Linux 的 `StatusNotifierItem` 按 NFR-7 另立待办。
- v0.2 改判（2026-11-04 真机走查轮，用户原话两条）：
  - 「关闭后，点击托盘还是没反应。右键也没反应，**右键起码的显示页面要有吧？打开设置要有吧？**」——② 的菜单由三项变**四项**，在 `退出程序` 前插入 `设置`（`Outcome::Settings` → 既有 `Action::OpenSettings`，兑现复用 `app::open_settings`，不在托盘里写第二条开设置的路）。菜单项数是需求口径，按 workflow 那条"调整规格必须 revise 递增版本"升版重新评审，不用 `replan` 蒙过去。
  - 「我们的这个托盘图标也太辣鸡了……**起码是我们原来图标的简笔画啊**，这啥东西？」——Q-002 由走查定论：形状改成**应用图标那两只气泡的单色线稿**（`assets/icon/icon-1024.png` 的前白后蓝两只，含各自朝外的尖），不再是 v0.1 默认的"窗口轮廓 + 折角"。这仍是 REQ-002 那套"现画 alpha 蒙版 + `setTemplate`"的口径，只换 `glyph()` 的线段表，TC-5.4-a 四条判据一字未动。
  - **左键唤不回的真病因是实现缺陷（D-21），不是行为口径**：v0.1 的取意图那一步挂在 `App::ui`，而窗口 `orderOut` 之后 eframe 走 `!show_ui` 那一支、**根本不调 `ui()`**（见事实表最后那条 FACT）——收进托盘之后意图永远躺在槽里。泵改挂 `App::logic`，行为口径（REQ-003 第①句"左键把窗口叫回来"）一个字没改。走查结论（2026-11-04 用户原话）："**托盘的可以了，没问题。**"
- 需求基线（本轮已按 CHG-001 REQ-004 的口径先改文档再动代码，四处都已落笔）：`docs/01-requirements.md:86` 的 **FR-5.2 前半句重写**（关窗 = `Visible(false)` → `orderOut` + 激活策略 `.accessory`，不再是最小化进 Dock）；`docs/01-requirements.md:88` 的 **FR-5.4 新增（M）**四条口径（②那半句在 v0.2 已同步改成菜单四项）；`docs/01-requirements.md:130` §6 第 5 行重算为 **45 条：Must 31 · Should 12 · Could 2，FR-5 由 ×2 变 ×3**。
- 追踪矩阵：`docs/traceability.md:64`（FR-5.2 行末登记"2026-11-04 用户改判前半句"，并写明 §5 那条"Visible(false) 叫不回来"的老偏差**由此关闭**）；`docs/traceability.md:66`（FR-5.4 新行，实现位置写 `ice-app::system::tray`，状态 `⏳ 待用户批准 CHG-008`）。
- 用例与走查：`docs/06-test-cases.md:127-129` 的 TC-5.4-a（托盘那张位图的无头判据）/ TC-5.4-b（菜单四项的数据，v0.2 随改判同步）/ TC-5.4-c（关窗→收进托盘→唤回的状态机）；`docs/06-test-cases.md:221` 的 W-2.40（真人走查，并声明 W-2.12 的判据由"点 Dock 图标恢复"改成本条，v0.2 已记下"托盘的可以了"这一句）；`docs/06-test-cases.md:170` 的 W-1.25（Dock 磁贴与邻居同大小，属 D-19，只作对照不进本包）。
- 架构与开发文档：`docs/02-architecture.md` 新增 `system::tray` 一行（`install` / `sync` / `Outcome::{Reveal,ToggleMode,Settings,Quit}` / `take_outcome` / `glyph_bytes`，与 `system::titlebar`、`system::dblclick` 同形）；`docs/03-ui-design.md` §4 流程图加了"托盘左键 ──► 唤回窗口；托盘右键 ──► 菜单"；`docs/05-dev-guide.md` §8 加了菜单栏托盘那行（含"新增 feature 必须逐条给编译面证据"这条纪律，v0.2 补上"帧钩子选错 = 隐藏期没人取意图"这条 D-21 教训）。
- 前置状态：CHG-004（P5）与 CHG-005（P6）都还在 IMPLEMENTING，本包**不带任何 OCR、回填、窗口形态、吸附的内容**，只改"关窗的去向 + 唤回的入口 + 托盘本身"。代码态（2026-11-04）：502 passed / 0 failed / 5 ignored（ice-app 349 · ice-core 58 · ice-providers 62 · ice-vision 33），clippy `-D warnings` 与 `cargo fmt --check` 干净，`tools/check_traceability.py` 以 45 ↔ 45 通过。
- 编号来源：`project/ROADMAP.md` 已把 P7/P8 的 CHG-006/007 预留住，插入包按 CHG-001 spec 那条"以新建 ID 为准并回写本表"取 **CHG-008**，ROADMAP 的插入行与"非 macOS 托盘另立待办"已写好。

## 目标与非目标

**目标**：把"关窗"这件事从 2026-10 以来那个**妥协形状**（取消关闭 + 最小化进 Dock）改回它本来该有的形状：托盘那枚图标在进程活着的整段时间都在（REQ-001），它是运行时画出来的一张单色线稿、跟随菜单栏深浅（REQ-002），左键把窗口叫回来、右键出四项菜单且四项各自生效（REQ-003），关窗走"窗口隐藏 + 激活策略 `.accessory`"、唤回走"可见 + `.regular` + 按当前形态整套重发几何"（REQ-004），退出仍是三条可见入口共用**同一条** `Action::Quit`（REQ-005），托盘装不上或不是 macOS 时退回旧语义而不是造出一个"关得掉、叫不回"的窗口（REQ-006），隐藏期间不逐帧续期、CPU 与状态可取证（REQ-007）。

**非目标**（各自有归属，本包不做也不验收）：

| 不做 | 归属 |
|---|---|
| Windows `Shell_NotifyIcon`、Linux `StatusNotifierItem`(D-Bus)、Wayland 托盘 | NFR-7 待办（`project/ROADMAP.md` 待决事项已列；Wayland 根本没有标准托盘） |
| 图片翻译的擦除、译文回填、覆盖层、导出（FR-2.4~2.9） | P7 = CHG-006 |
| Dock 磁贴尺寸（缺陷 D-19，"大一圈"） | **本轮已改**：`main::app_icon` 按苹果图标网格补留白，与托盘无关，只留 W-1.25 一条真机对照 |
| 打包形态（Info.plist 的 `LSUIElement`、正式 `.app` 的激活策略基准） | P8 = CHG-007；本包只在运行中切策略，不写 plist |
| 系统通知（`NSUserNotification` / UserNotifications） | 不做：需求里没有这一条 |
| 系统级全局快捷键（注册到窗口之外） | 不做：`Cmd+Q` / `Cmd+W` 现在仍是窗口内的 `system::hotkeys`，需求没要求窗口外可用 |
| 登录即启动、启动即隐藏、托盘菜单里加翻译/历史等功能项 | 不做：**本包不新增任何配置字段**，菜单固定四项（v0.2 那第四项 `设置` 只是把**已有**的设置页唤开，不是新页面、不是新字段） |
| 主题包（#28）、界面多语言（#23）、滑块可读性等 P5 遗留待裁定项 | 各自待办 |

本包**不改**文本翻译链路、图片识别链路、密钥边界（ADR-005）、配置持久化格式，也不改形态切换、停靠、置顶、主题那四条已验收的通路——它们只是被"唤回"这一步重新下发几何。

## 事实与假设

| 标签 | 结论 | 来源 / commit / 环境 / 时间 |
|---|---|---|
| FACT | **今天关窗走的是妥协形状**：`Action::HideWindow` 由 `app.rs:1304-1313`（`close_requested() && !quitting`）和 `Cmd+W` 两条路进 `mode::hide`，而 `mode::hide`（`mode.rs:748-755`）发的是 `CancelClose` + `Minimized(true)`——不是封存设计写的 `Visible(false)`。恢复也不是事件驱动，是 `RevealGuard`（`mode.rs:765-822`）拿"几何回来了 / 拿到焦点"两条启发 + 800ms 静默期判的 | `crates/ice-app/src/mode.rs:743-848`、`app.rs:204`、`app.rs:1304-1313`、`system/hotkeys.rs:41`；查阅 2026-11-04 |
| FACT | **当初退让的唯一原因是"叫不回来"**：eframe 0.37 不把 macOS 的 `Reopen` 传上来，`Visible(false)` 之后点 Dock 没有反应，加上单实例锁让"再开一个"直接退出。托盘自带左键动作，**唤醒入口不再依赖系统那条回调**，所以封存设计那一步现在可以照做——这就是 `docs/traceability.md:64` 说"§5 老偏差按 FR-5.4 关闭"的机制依据 | `mode.rs:745-747` 的注释原文；`docs/07-acceptance.md` §5 偏差第 2 行（2026-11-04 标注关闭）；查阅 2026-11-04 |
| FACT | **winit 0.30.13 与 eframe 0.37.0 里没有任何托盘能力**：全树搜 `NSStatusItem` / `statusItem` / `StatusBar` / `tray` 只命中 winit 自己那三个 generated 绑定文件（`objc2-app-kit` 的 `NSStatusBar.rs` 等，是 winit 依赖里躺着但没人调），winit 的 macOS 平台层没有一处创建过状态栏项。结论：**托盘只能应用侧自己调 AppKit**，与已有的六处 objc2 直调同源，不引 `tray-icon` 这类第三方（它会一次拖进 Win/Linux/macOS 三套绑定，与本仓 `default-features = false` 的最小依赖面纪律相反） | `~/.cargo/registry/src/…/winit-0.30.13/src/platform_impl/macos/` 与 `eframe-0.37.0/src/` 全文 grep；2026-11-04 核对 |
| FACT | **要用的 AppKit API 在绑定里全都有，签名逐条读出来了**（objc2-app-kit 0.3.2，本机 registry 缓存，无需联网）：`NSStatusBar::systemStatusBar() -> Retained<NSStatusBar>`（`NSStatusBar.rs:32-34`，无额外 feature 门）、`statusItemWithLength(CGFloat) -> Retained<NSStatusItem>`（`:36-39`，门是 `NSStatusItem` + `objc2-core-foundation`）、`NSVariableStatusItemLength: CGFloat = -1.0`（`:13`）、`removeStatusItem(&NSStatusItem)`（`:41-44`）；`NSStatusItem::setMenu(Option<&NSMenu>)` / `menu(mtm)`（`NSStatusItem.rs:66-75`，门 `NSMenu`）、`button(mtm) -> Option<Retained<NSStatusBarButton>>`（`:76-86`，门 `NSButton`+`NSControl`+`NSResponder`+`NSStatusBarButton`+`NSView`）、`isVisible/setVisible`（`:98-104`）；`NSMenu::new(mtm)`（`NSMenu.rs:464-466`，类是 `MainThreadOnly`）、`addItem(&NSMenuItem)`（`:204-207`，门 `NSMenuItem`）、`setAutoenablesItems(bool)`（`:330-332`）；`NSMenuItem::initWithTitle_action_keyEquivalent(Allocated, &NSString, Option<Sel>, &NSString)`（`NSMenuItem.rs:84-94`，`unsafe`）、`setTarget(Option<&AnyObject>)`（`:355-360`，注释明写 **weak property**）、`setAction(Option<Sel>)`（`:368-371`）；`NSButton::setImage(Option<&NSImage>)`（`NSButton.rs:446-450`，门 `NSImage`）；`NSImage::initWithData(Allocated, &NSData) -> Option<Retained<Self>>`（`NSImage.rs:192-194`）、`setTemplate(bool)`（`:442-449`）、`size()/setSize(NSSize)`（`:244-249`）；`NSApplication::sharedApplication(mtm)`（`NSApplication.rs:488`）、`currentEvent() -> Option<Retained<NSEvent>>`（`:900-903`，门 `NSEvent`）、`activate()`（`:569-571`）与 `activateIgnoringOtherApps(bool)`（`:552-555`，已标 deprecated） | 上列 file:line 逐条读自 `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/objc2-app-kit-0.3.2/src/generated/`；2026-11-04 核对 |
| FACT | **切换激活策略要多开一个想不到的 feature**：`NSApplication::setActivationPolicy(NSApplicationActivationPolicy) -> bool` 整个方法 `#[cfg(feature = "NSRunningApplication")]`（`NSApplication.rs:746-755`），而 `NSApplicationActivationPolicy` 那三个常量（`Regular=0` / `Accessory=1` / `Prohibited=2`）也定义在 `NSRunningApplication.rs:47-56`。本仓现在没开这个 feature——"关掉 Dock 磁贴"这一步在类型层面挂在 `NSRunningApplication` 后面，不开就**方法都看不见**，只剩"为什么我明明调了磁贴还在" | `objc2-app-kit-0.3.2/src/generated/{NSApplication.rs:746-755,NSRunningApplication.rs:47-56}`；2026-11-04 核对 |
| FACT | **feature 增量的完整清单（8 个 app-kit + 1 个 foundation）**：`NSStatusItem`、`NSStatusBar`、`NSStatusBarButton`、`NSResponder`（`button()` 那条 cfg 要它，本仓目前没开）、`NSMenu`、`NSMenuItem`、`NSImage`、`NSRunningApplication`；`objc2-foundation` 侧补 `NSData`（`NSImage::initWithData` 的参数类型）。已有的 `NSButton`/`NSControl`/`NSView`/`NSEvent`/`NSApplication`/`block2`/`objc2-core-foundation` 复用。代价与理由必须写进 `crates/ice-app/Cargo.toml` 那段注释里（第 42-60 行是变更 K 撤掉九个模块后立下的纪律：每个开关给一条编译面证据），`docs/05-dev-guide.md` §8 那一行已经先把这条口径记下了 | `crates/ice-app/Cargo.toml:37-97` 现有清单与注释；上两行的 cfg 实测；2026-11-04 核对 |
| FACT | **`orderOut` / `makeKeyAndOrderFront` 就是 winit 的 `set_visible` 那两个动作**：`window_delegate.rs:904-909` 里 `set_visible(true)` → `makeKeyAndOrderFront(None)`，`false` → `orderOut(None)`；`is_visible()`（`:911-914`）直接返回 `Some(window().isVisible())`。两个连带结论：① 唤回**自带"成为 key window"**，不需要额外发 `Focus`；② 隐藏/可见这件事有**硬证据**可查，不像 `is_minimized()` 那样在 macOS 上被本项目认定为运行时不可靠（`mode.rs:757-764` 那条 emilk/egui#3494 的教训） | `~/.cargo/registry/src/…/winit-0.30.13/src/platform_impl/macos/window_delegate.rs:904-914`；`crates/ice-app/src/mode.rs:757-767`；2026-11-04 核对 |
| FACT | **隐藏期间"托盘点一下就把帧循环叫醒"这条链在 eframe 0.37 里是成立的，而且是它自己专门做了的**：`run.rs:26-28` 有 `INVISIBLE_WINDOW_REPAINT_INTERVAL = 100ms`，注释原话"invisible windows don't receive `RedrawRequested` events, so we throttle their repaints to avoid busy-looping **while still processing viewport commands like `Visible(true)`**"；`run.rs:224-250` 的 `check_redraw_requests` 碰到 `is_invisible_or_minimized(window)` 就**直接** `run_ui_and_paint(..., PassMode::Full)` 而不是等 `RedrawRequested`。回调侧只需 `ctx.request_repaint()`（`glow_integration.rs:373` 把它变成 `UserEvent::RequestRepaint` 投进 EventLoopProxy，会唤醒 `ControlFlow::Wait`）→ 下一拍 `update()` 跑起来 → 取走托盘意图 → 发 `Visible(true)`。**这条是本包新语义的机器可验前提，不是乐观假设** | `eframe-0.37.0/src/native/{run.rs:22-36,run.rs:205-265,glow_integration.rs:373}`、`winit_integration.rs:16-18`；2026-11-04 核对 |
| FACT | **winit 抢不走我们的策略**：它只在 `applicationDidFinishLaunching` 里、且**仅当没人显式给过激活策略且应用未打包**时把策略设成 `.regular`（`app_state.rs:119` / `:131`），一次性的。我们在运行中切 `.accessory` / `.regular` 不与它打架。打包后的 `.app` 由 Info.plist 定基准，运行中同样可切（那条属 P8，本包不写 plist） | `winit-0.30.13/src/platform_impl/macos/app_state.rs:119`、`:131`；2026-11-04 核对 |
| FACT | **左右键分开的原料在绑定里齐了**：`NSControl::sendActionOn(NSEventMask) -> NSInteger`（`NSControl.rs:270-273`，门 `NSEvent`）可以把动作登记到某个事件掩码上，`NSEventMask::{LeftMouseUp, RightMouseUp}` 两个位在 `NSEvent.rs:202-207` 有常量；分叉依据用 `NSApplication::sharedApplication(mtm).currentEvent()`（`NSApplication.rs:900-903`）再读事件类型（`NSEventType::RightMouseUp = Self(4)`，`NSEvent.rs:27-28`）。**这是经典配方，绑定层没有缺件** | 上列 file:line；2026-11-04 核对 |
| FACT | **"入口只立意图、帧循环兑现"这个模子在本仓已有两个成品**：`system::titlebar.rs` 用 `define_class!` 注册一个只有方法体的 `NSObject` 子类（`:82-106`），action 只做 `CLICKED.store(true)`（`:34-56`），帧循环 `take_click()` 后置 `Effects::toggle_mode`；`:18-20` 那条注释写死了另一半教训——**`NSControl.target` 是 `unsafe_unretained`，强引用必须由我们自己持有**，否则点一下就是访问已释放对象。`NSMenuItem.setTarget:` 的绑定注释同样明写它是 weak property。托盘照这个模子做，target 那个 `Retained` 存在模块内的 `thread_local!` 里活到进程结束 | `crates/ice-app/src/system/titlebar.rs:1-210`；`objc2-app-kit-0.3.2/src/generated/NSMenuItem.rs:352-360`；2026-11-04 核对 |
| FACT | **托盘位图不需要任何新依赖**：workspace 的 `image = { version = "0.25", default-features = false, features = ["png", "jpeg"] }`（`Cargo.toml:27`）里 PNG **编码**是开着的（`ice-app` 的 `image.workspace = true` 已经用它解图标）。形状 → `RgbaImage`（RGB 纯黑 + alpha 承载轮廓）→ PNG 字节 → `NSData` → `NSImage::initWithData` → `setTemplate(true)` → `button.setImage`，全程只动已有的两个 crate | `Cargo.toml:27`、`crates/ice-app/Cargo.toml:24`、`crates/ice-app/src/main.rs` 的 `app_icon()`；2026-11-04 核对 |
| FACT | **五条既有测试的前提会随语义作废**：`mode_cancels_close_and_minimizes`（实为 `mode.rs:1280-1287` 的 `hide_cancels_close_and_minimizes`，断言 `Minimized(true)`）、`app.rs:1483-1495` 的 `hide_window_keeps_the_process_and_the_content`（同样断言 `Minimized(true)`）、`mode.rs:1298-1350` 那四条 `RevealGuard` 判恢复的测试（前提是"只能靠几何/焦点猜"）。新语义下"回来了"是托盘告知的，不再靠猜——**这几条按新语义重写，不是删掉**：进程不退、内容原样、命令幂等这三条主张一个字都不放 | 上述 file:line；`docs/06-test-cases.md:129`（TC-5.4-c 已写明"随本条重写"）；2026-11-04 核对 |
| FACT | **v0.1 的泵挂错了钩子：窗口不可见时 eframe 根本不调 `App::ui`**（D-21 的真病因，2026-11-04 走查查出）。`PassMode::Full` 自己的定义就是"跑 `App::logic`，**并且只有 viewport 可见时**才跑 ui 与绘制"（`winit_integration.rs:34-36`）；`glow_integration.rs:692-720` 里 `show_ui = mode == PassMode::Full && (is_visible || …)`，`!show_ui` 那一支走 `update_logic_only` → `epi_integration.rs:395` **只调 `app.logic(ctx, &mut self.frame)`**。三处连带：① `logic()` 里 `Frame` 拿得到，`winit_window().scale_factor()` 那条档位来源不用换；② 可见帧同样是先 `logic` 再 `ui`（`epi_integration.rs:357`），所以泵挂 `logic` 是"一帧一次"，不是两次；③ 取走意图后发的 viewport 命令在隐藏期照样被处理（`glow_integration.rs:735-738`），而 `request_repaint` 对不可见窗节流到 100ms（`run.rs:376-383`）——**泵是意图驱动的事件，不是心跳** | `eframe-0.37.0/src/native/{winit_integration.rs:34-36, glow_integration.rs:692-720 与 :735-738, epi_integration.rs:357 与 :395, run.rs:376-383}`；实现见 `crates/ice-app/src/app.rs` 的 `logic` / `pump_tray`；2026-11-04 核对 |
| FACT | **菜单一常驻挂在状态项上，左键就不再送 action**（Q-001 的实现落点）：所以只用 `popUpStatusItemMenu:` 就地弹一次，弹出后立刻 `setMenu(None)` 并核对接线。这个方法自 10.14 标 deprecated，但"改挂 `menu` 属性 + 左键走 `performClick:`"那条替代正好踩回同一个坑——菜单挂着就吃掉左键，而 spec 的口径是左键唤回（`NSStatusItem.rs:300-304` 仍导出它）。两件事必须同批做对：**"出菜单"要在 ObjC 回调里同步发生**（登记成旗子等下一帧，等于交给一个可能永远不来的读者——D-21 的右键那一半就是这个）；**四个 `Retained` 句柄要先从 `thread_local!` 克隆出来、释放借用**，再进 AppKit 那个阻塞的菜单追踪循环，否则追踪期间重进同一个 `RefCell` 是双重借用 panic | `objc2-app-kit-0.3.2/src/generated/NSStatusItem.rs:300-304`；`crates/ice-app/src/system/tray.rs` 的 `clicked` 与 `pop_menu`；2026-11-04 核对 |
| FACT | **`Action` 里现在没有"唤回"这一条**：`system/hotkeys.rs:14-19` 的枚举有 `ToggleMode` / `HideWindow` / `Quit` / `ClearInput` 等，`app.rs:197-215` 的 `run()` 逐条分派；`reveal` 目前只由 `RevealGuard` 在帧循环里直接调。新语义要让它成为**与快捷键同一条通道**的动作，否则"入口只立意图"这条纪律在托盘这里破了 | `crates/ice-app/src/system/hotkeys.rs:14-45`、`app.rs:196-215`、`mode.rs:829-848`；2026-11-04 核对 |
| FACT | **（v0.1 的 HYPOTHESIS，2026-11-04 真机已定论）`sendActionOn` + `currentEvent` 分得开左键与右键**：托盘装上后左键一下就把窗口叫回来、右键出菜单且不唤回，用户原话"托盘的可以了，没问题"。停在这里的边界：**Ctrl+左键没有单独走查**，它按 `currentEvent().r#type()` 落到右键那一支，理论上与右键同一条路；两条退路（两键都出菜单 / 常驻 local monitor）都没用上，停止条件 1 未触发 | 上面两条 FACT；W-2.40 走查记录（`docs/06-test-cases.md`）；2026-11-04 |
| HYPOTHESIS | **`.accessory` 下唤回要额外强制激活才真的到最前**——v0.2 的实现选了强制那一支，但**归因没定论**：`set_policy(true)` 在切回 `.regular` 之后调 `activateIgnoringOtherApps(true)`（`#[allow(deprecated)]`）。选它的理由写在 `tray.rs` 那段注释里：非 deprecated 的 `activate()` 文档原话是"does not guarantee that the app will be activated at all"，是**协作式**的，别的应用正前台时系统可以直接拒绝；唤回是用户主动点的，没有比这更明确的授权。而 v0.1 那次"点了托盘没反应"的真病因是泵挂错钩子（上面那条 FACT），**不是 `activate()` 不够——所以"非 deprecated 的那条到底够不够"这条既没被证明也没被证伪**，留着：真机若出现"抢焦点打断用户"的观感问题，先试着换回 `activate()`，两种结果都可接受，属观感不是结构，落 REQ-004 的实现选项而不设停止条件 | `NSApplication.rs:569-571`（`activate`）、`:552-555`（`activateIgnoringOtherApps` 已标 deprecated，替代的 `NSRunningApplication.activate(from:)` 要 macOS 14 + `NSApplicationActivationReason`，我们的窗口不靠系统唤起，故局部放行旧那一条）；W-2.40 走查"托盘的可以了"覆盖的是**唤回成了**，没覆盖"协作式那支够不够"；2026-11-04 |
| HYPOTHESIS | **隐藏期间 CPU < 1%（NFR-2 / FR-5.2 验收那句）在 `orderOut` 下仍然成立**：现在的证据是 eframe 自己那两处兜底（`sleep_if_invisible_or_minimized` 睡 10ms、`INVISIBLE_WINDOW_REPAINT_INTERVAL` 节流）加上"我们隐藏时不许逐帧 `request_repaint`"这条纪律；但**托盘的 action 回调本身会请求一次重绘**，所以"关窗后放十分钟再采样"这件事只有真机能定论（W-2.40 + REQ-007 取证）。若采样明显高于 1%，查的是"谁在续期"而不是把判据调低 | `eframe-0.37.0/src/native/winit_integration.rs:20-30`、`run.rs:26-28`；`docs/01` NFR-2 与 FR-5.2 验收列；2026-11-04 |

## 行为与验收

### REQ-001: 托盘在进程活着的整段时间常驻，且只装一次（FR-5.4 ①）

WHEN 应用启动完成、第一次真正画帧时
THEN 系统 SHALL 在**主线程**上装出托盘项（`NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength)`），SHALL NOT 等到用户第一次关窗才装——需求说的是"进程活着就在"，而关窗之后才有的图标满足不了"常驻"。

WHEN 后续每一帧再走到安装点
THEN 系统 SHALL 认出"已经装过"（模块内记着那枚 `Retained<NSStatusItem>`）并**什么都不做**；`install()` SHALL 是幂等的，重复调用不产生第二枚图标。判据：SHALL NOT 出现"菜单栏里两枚一模一样的图"这类缺陷（`system::titlebar` 每帧核对现状而不缓存"我接过了"是同一条纪律的另一面，这里的情况相反——对象是我们自己造的，所以记着它才对）。

WHEN 安装失败（拿不到 `MainThreadMarker`、`statusItemWithLength` 给了空按钮、系统状态栏不可用）
THEN 系统 SHALL 记一次 WARN（只记错误码与原因，不含任何用户内容），SHALL NOT panic，SHALL NOT 退出进程；且下一帧 SHALL 还能重试。托盘不可用时关窗的去向由 REQ-006 接住。

WHEN 非 macOS 编译面
THEN `system::tray` 的对外函数 SHALL 存在但恒返回"不可用"，SHALL NOT 引用任何 objc 类型；无头断言落在"非 mac 不引用符号 + 返回不可用"这一层（与 `system::titlebar::sync` 的那条 `#[cfg(not(target_os = "macos"))]` 兜底同一个形状）。

边界：本包不新增任何配置字段（装不装、装哪儿、菜单几项都是代码里的常量与默认）；托盘项在 `quit` 路径上不主动 `removeStatusItem`（进程退出即回收，多这一步只会多一个"退出顺序"的新缺陷面）。
验证层次：幂等与失败降级有无头断言（对纯 Rust 的那层状态机测，不碰 AppKit）；"菜单栏真的出现了图标"只有真机能定论 = W-2.40。

### REQ-002: 托盘图是运行时画的单色线稿，跟随菜单栏深浅（FR-5.4 ③、缺陷 D-5 的教训）

WHEN 生成托盘位图
THEN 系统 SHALL 现画一张 **alpha 承载轮廓、RGB 为纯黑**的位图（菜单栏那一档的高度），经 PNG 编码交给 `NSData` → `NSImage::initWithData` → `setTemplate(true)`；SHALL NOT 把彩色 Dock 那枚方砖缩小了事（18pt 下形状读不出，且彩色不跟随深浅色），SHALL NOT 用文本字符当图标（D-5 的豆腐块教训：界面上所有装饰图案都是现画线稿，`views::pin_toggle`、`views::theme_switch` 是既有先例）。

WHEN 真机在深色与浅色菜单栏之间切换（含自动主题）
THEN 这枚图 SHALL 由系统按模板色自动翻深浅，SHALL NOT 出现"深色菜单栏上一团黑看不见"。

形状（Q-002 已由 2026-11-04 走查定论，见未决问题）：**应用图标那两只气泡的单色线稿**——前一只（原图左上带「文」的白气泡）完整画出，后一只（右下带「A」的蓝气泡）只画它落在前一只外扩范围之外的轮廓，两只之间留一条 0.8 点的缝（都画满就成"8"字，前后关系读不出来），各带一条朝外的尖（前一只尖朝左下、后一只朝右下，与 `icon-1024.png` 同向）。22 点那格放不下「文」和「A」，但放得下它们的形状。v0.1 默认的"窗口轮廓 + 往里收的折角"已被用户否掉（"起码是我们原来图标的简笔画啊，这啥东西"）。形状改动只动一个纯函数（`glyph()` 的线段表：两只 `Bubble` + 四条尾巴），TC-5.4-a 的四条判据不跟着改。

WHEN 无头检查这张位图（TC-5.4-a）
THEN 断言 SHALL 落在四个可自动算的点上：① 边长是菜单栏那一档（@1x 高 22px，@2x 44，随给定档位走）；② 每个非透明像素的 RGB 是纯黑（模板图靠 alpha，出现彩色就是错的）；③ 四角透明；④ 内容包围盒水平居中（左右留白差 ≤1px）且上下各留 ≥1px。

边界：`glyph_bytes()` SHALL 是与 AppKit 无关的纯函数（`ice-app` 侧可测，非 mac 也能编译通过并测），`system::tray` 的 mac 模块只负责把它的字节交给 `NSImage`。
验证层次：上面四条是无头的；"一眼认得出这是什么"只有真机能定论 = W-2.40。

### REQ-003: 左键唤回、右键出四项菜单，四项各自生效且只立意图（FR-5.4 ②、FR-5.2 ③）

WHEN 用户左键点托盘
THEN 系统 SHALL 把窗口叫回来（走 REQ-004 的 reveal 那一步），SHALL NOT 弹菜单。

WHEN 用户右键点托盘（含 Ctrl+左键）
THEN 系统 SHALL **在回调里同步弹出**四项菜单，SHALL NOT 直接唤回窗口。左右两种事件 SHALL 由**同一个** action 回调分叉（`sendActionOn` 登记两个掩码 + `currentEvent().type()` 判读，见事实表那条 FACT），SHALL NOT 为右键另装一个常驻事件监视器，也 SHALL NOT 把菜单常驻挂在状态项上（一挂上去左键就不再送 action，Q-001）；弹出用 `popUpStatusItemMenu:`，追踪循环返回后立刻 `setMenu(None)` 并核对接线。"把菜单弹出去"这一件 SHALL NOT 登记成旗子等帧循环兑现——它是纯 AppKit，不需要 `AppState`，而窗口收起来时 eframe 只跑 `App::logic`（D-21 的右键那一半就丢在"等一个可能永远不来的读者"上）。

WHEN 菜单出现
THEN 四项 SHALL 依次是 `显示窗口` / `精简 ↔ 全量` / `设置` / `退出程序`；第二项 SHALL 给出**另一侧**形态的名字——当前全量就写 `精简`（点下去切到简洁窗），当前简洁就写 `全量`（点下去切回全量窗），SHALL NOT 写"悬浮窗"（变更 N 的口径：那三个字已从全部界面退掉）。第二项的文案 SHALL 由一个纯函数按当前形态算出来（TC-5.4-b 断言的就是这个函数 + 四项的动作映射）。

WHEN 用户点了菜单里任意一项
THEN 回调 SHALL **只记一个意图**（模块内那一个 `AtomicU8` 槽取一个值），由 **`App::logic`** 取走后映射成 `Action`（`ToggleMode` / `Quit` / `OpenSettings` / `RevealWindow`），SHALL NOT 在 ObjC 回调里直接改窗口、发命令或读 `AppState`——这条与 `system::titlebar` 同形，理由也一样：回调发生的时机由 AppKit 决定，把它接进帧循环才能保证"同一件事只有一个兑现点"。**取走那一步 SHALL NOT 挂在 `App::ui`**：窗口 `orderOut` 之后 `ui()` 一次都不会被调（见事实表那条 FACT），挂在 `ui()` 等于"只在窗口已经看得见的时候才监听唤醒入口"，收进托盘之后那一点永远没人取。

WHEN 一帧内托盘被连点多次
THEN 同一种意图 SHALL 只兑现一次（`take_outcome` 取走即清零），与 `one_click_is_handed_over_exactly_once` 那条既有判据同理。

WHEN 用户点 `设置`
THEN 窗口收着 SHALL 先唤回、再把设置页开起来；兑现 SHALL 复用已有的 `app::open_settings`（工具栏那颗与提示条「去设置」走的是同一个入口），走 `Action::OpenSettings`（新增的是**第三个可见入口**，不是第二条实现）。SHALL NOT 在托盘里另写一条开设置的路——这条与"退出只有一条"同一纪律。

WHEN 用户点 `退出程序`
THEN SHALL 与设置页那颗、`Cmd+Q` 走**完全同一条** `Action::Quit`（先记回几何 → 强制落盘 → 关窗 → 释放单实例锁），SHALL NOT 开第二条退出实现。托盘这颗立的是"第三个看得见的入口"，不是"第二条路径"。

边界：菜单项不做动态禁用（`setAutoenablesItems(false)` + 四项恒可用），因为"没有窗口可显示"这个状态在本应用不存在（窗口只是隐藏，进程活着）。
验证层次：四项的文案/顺序/动作映射、"取走即清零"、"收起期交来的意图照样兑现"有无头断言；"右键真的出菜单、左键真的不菜单"只有真机能定论 = W-2.40（停止条件 1 兜住失败；2026-11-04 已定论"托盘的可以了，没问题"）。

### REQ-004: 关窗 = 窗口隐藏 + 磁贴消失；唤回 = 可见 + 磁贴回来 + 按形态重发几何（FR-5.2 前半句改判）

WHEN 用户点红灯或按 `Cmd+W`
THEN 系统 SHALL 在同一帧里发 `CancelClose`（否则 eframe 直接结束 run loop，这与今天一致）+ `Visible(false)`，并 SHALL 把激活策略切成 `.accessory`；SHALL NOT 再发 `Minimized(true)`——窗口不再进 Dock（用户裁定 ①）。

WHEN 关窗动作完成
THEN 进程 SHALL 仍然活着、输入框与译文等内容 SHALL 原样保留（这两条是 FR-5.2 的既有主张，`hide_window_keeps_the_process_and_the_content` 重写时保留其断言），Dock 里 SHALL NOT 再有磁贴，菜单栏那枚 SHALL 还在。

WHEN 用户从托盘唤回（左键或菜单 `显示窗口`）
THEN 系统 SHALL 先把激活策略切回 `.regular`、再发 `Visible(true)`（后者在 winit 里就是 `makeKeyAndOrderFront`），两支都 SHALL 在同一批命令里**紧跟一发 `ViewportCommand::Focus`**（`ViewportCommand::Focus` 的文档写明"窗口不可见或最小化时没有效果"，所以只能排在 `Visible(true)` / `Minimized(false)` **之后**，顺序反过来是空操作），随后 SHALL 按 `config.ui.mode` 整套重发 `commands_for_mode` + `target_size` 并给 `PendingGeometry` 上护栏（D-6/D-10 那两条护栏**不回退**），并 SHALL 重新 `capture` 尺寸槽；`state.hidden` SHALL 置回 false。

WHEN 唤回时窗口本来就是可见的（用户从没关过，或重复点）
THEN reveal SHALL 是幂等的：多发一次 `Visible(true)` 与同一组几何不改变窗口现状，SHALL NOT 把用户手动缩放过的尺寸写坏（这条正是 `PendingGeometry` 三段护栏负责的事）。

WHEN 隐藏前窗口是停靠的、置顶的或铺满的
THEN 唤回 SHALL 按当前形态的既有语义把这些判定重放回去（沿用今天 `reveal` 的处理：`filled` 标记随之复位、`apply_topmost` 再下发一次），SHALL NOT 新增"停靠状态"这个配置项。

WHEN `Reopen` / Dock 点击
THEN 系统 SHALL NOT 依赖它们。`RevealGuard` 那套"靠几何/焦点猜回来了没有"的启发 SHALL 在 macOS + 托盘可用的路径上退役（不再需要猜）；SHALL NOT 把它留成第二条并行的唤醒路径——两条都能唤回 = 两条都能各自出缺陷。

验证层次：命令序列（有 `CancelClose` + `Visible(false)`、**没有** `Minimized(true)`）、策略切换的调用次序、reveal 的幂等与几何重发有无头断言（`egui::Context` 的命令队列可取，沿用 `mode.rs` 测试里那个 `queued()` 手法）；磁贴真的消失/回来只有真机能定论 = W-2.40。

### REQ-005: 设置页那句说明跟着改口（FR-5.2 ③"退出必须有看得见的入口"一字未改）

WHEN 用户打开设置页的「窗口」分区
THEN 「退出程序」那颗旁边的说明 SHALL 写"红灯/`Cmd+W` 是收进托盘，不是退出"；SHALL NOT 留着"收进后台/最小化"这类与新语义不符的旧话（模块里过时的说明文字与界面撒谎是同一类缺陷，P6 对 `views/image.rs` 那条注释的要求同理）。

WHEN 托盘可用与不可用两种情况
THEN 这句说明 SHALL 与真实行为一致：托盘可用时说"收进托盘"；不可用时（REQ-006 的降级）说"收进后台"。SHALL NOT 在不存在的去向面前写死一句话——这条用"文案由同一个判定给出"来实现，而不是两处分支各写各的。

边界：不改设置页其它项，不动「恢复默认窗口尺寸」那颗（FR-3.6/D-10 的救济，与本轮无关）。
验证层次：文案由纯函数按"托盘可用否"分档，有无头断言；观感走 W-2.26（既有那条退出入口的走查顺带核对）。

### REQ-006: 托盘装不上或不是 macOS 时退回旧语义，绝不出现"关得掉、叫不回"（NFR-7 的兜底纪律）

WHEN 非 macOS 平台，或 macOS 上 REQ-001 的安装最终没成功
THEN 关窗 SHALL 退回今天这套"取消关闭 + 最小化"（`CancelClose` + `Minimized(true)`）与 `RevealGuard` 的恢复判定，SHALL NOT 只发 `Visible(false)` 而没有别的唤醒入口。这条是本包最硬的一条否定式判据：**新语义必须整套生效或整套不生效**，混合状态（隐藏了但只有 Dock 能叫、而 Dock 磁贴又被 `.accessory` 关掉）就是"用户没法重启软件"那个老问题（2026-10-27 用户原话）的复现。

WHEN 系统进入降级态
THEN 界面 SHALL 给一条按提示条规则淡出的说明（沿用 P5 变更 I/L 的提示条，时长只写常量名），SHALL NOT 弹报错窗；SHALL NOT 因此改变关窗键的语义（用户按红灯还是"收起"，只是收起的去向回到 Dock）。

WHEN 无头断言
THEN 判据 SHALL 是"选定哪条路径就发哪组命令"（`TrayState::{Available, Unavailable}` 驱动 `mode::hide` 的命令分支），SHALL NOT 靠 `cfg!(target_os)` 在 hide 里散成三处 if。

边界：本条只承诺"不锁死"，不承诺非 macOS 有托盘（那是 NFR-7 待办）；Wayland 连标准托盘都没有，这条事实已写进 `docs/01` FR-5.4 第④句与 ROADMAP 待决事项。
验证层次：命令分支有无头断言（两种 `TrayState` 各一组）；真机 macOS 上"拔不掉托盘"这条不可构造，故降级路径的实测只有交叉平台验证 = P8。

### REQ-007: 隐藏期间不逐帧续期，资源与唤醒链可取证（NFR-2、FR-5.2 验收列）

WHEN 窗口处于隐藏态
THEN 帧循环 SHALL NOT 主动逐帧 `request_repaint()`（提示条淡出、动画心跳在隐藏期没有观众）；SHALL 只在托盘意图到达时重绘一次。这条的机器依据是事实表那条 FACT：eframe 对不可见窗的重绘请求会**直接跑一次 `update()`**（`run.rs:224-250`），不再等 `RedrawRequested`——D-21 查清之后这一支的具体内容要写准：它是 `PassMode::Full`，但 `show_ui` 仍按可见性判 false，于是走 `update_logic_only`，**跑的是 `logic()` 而不绘制**。所以"顺手续期"烧的是每帧的 logic（含托盘泵、剪贴板轮询、可见性看守），省不掉但也不该有；判据不变：**隐藏期不许有心跳**，唤醒只由托盘那一下决定（`the_tray_branch_leaves_no_second_wakeup_path` 锁的就是这一条，它同时锁"托盘档退役 `RevealGuard` 那条猜测路径"）。

WHEN 用户关窗后把应用放着不动再采样
THEN CPU SHALL < 1%（NFR-2 与 FR-5.2 验收列那句）。取证方式：`./sdd record` 导入脱敏的 `ps`/采样观察值，SHALL NOT 把自动推断写成实测。

WHEN 托盘的 action 回调发生
THEN 回调 SHALL 调 `request_repaint()` 唤醒帧循环（这是"左键点一下就回来"的那一步，机制依据见 `run.rs` 那条 FACT）；SHALL NOT 在回调里直接发 viewport 命令。

WHEN 隐藏期间发生形态切换、置顶、主题等**其它**路径
THEN 系统 SHALL NOT 让它们在隐藏态悄悄改窗口（隐藏时用户看不到，回来时版式对不上）。本包不新增这类路径，只在测试里锁"隐藏态不接受来自界面的模式命令"这条既有主张不回退（若与现有实现冲突，按 REQ-004 的 reveal 重发几何兜住，不额外加机制）。

边界：日志只允许 `outcome`、`policy`、`hidden`、错误码与耗时一类计数，SHALL NOT 记录任何窗口内容或用户文本（NFR-2/3）。
验证层次：不续期有无头断言（隐藏帧序列里 `request_repaint` 的次数与意图对应）；CPU 数字与"点一下真的回来了"只有真机能定论 = W-2.40 + REQ-007 的取证条目。

## 未决问题

| id | 问题 | 阻塞 | 结论 |
|---|---|---|---|
| Q-001 | 左键与右键怎么在**一个** NSStatusItem 上分开？`setMenu:` 一挂上去，左右键都会出菜单，左键就不再走 action | 否 | **已决（实现选择，不改行为口径），v0.2 把"弹"的落点写实**：不常驻挂菜单，改用 `button.setTarget/setAction` + `sendActionOn(LeftMouseUp\|RightMouseUp)`，回调里读 `NSApplication.currentEvent().type()` 分叉：右键 → **在回调里同步** `popUpStatusItemMenu:` 就地弹一次，返回后立刻 `setMenu(None)` 并核对接线；左键 → 只立唤回意图。v0.1 那句"临时把菜单弹出去"当时打算登记旗子交给帧循环去弹，那是 D-21 的另一半（隐藏期没人读旗子），已在实现里去掉。**真机已定论**：左键唤回、右键出菜单都成立（2026-11-04"托盘的可以了，没问题"），停止条件 1 未触发 |
| Q-002 | 托盘图长什么样？"能认出"这件事只有用户看得到 | 否 | **已决（2026-11-04 走查定论，v0.2）：应用图标那两只气泡的单色线稿**——v0.1 默认的"窗口轮廓 + 折角"被否掉（"起码是我们原来图标的简笔画啊，这啥东西"）。画法是前一只气泡完整 + 后一只只画外扩 0.8 点之外的轮廓 + 两条朝外的尖，纯函数 `glyph()` 的线段表换成两只 `Bubble` 和 `tails()`，**TC-5.4-a 那四条判据一字未动**（边长随档位、非透明像素纯黑、四角透明、包围盒水平居中且上下留白）。备选那两条（图钉、彩标剪影）不再考虑 |
| Q-003 | 菜单第二项到底写"精简"还是"切换到精简"？ | 否 | **已决**：只写形态名（`精简` / `全量`），与全量窗那颗模式按钮、简洁窗那格同一套字（变更 N 之后界面里已经没有"悬浮窗"三个字）。菜单是短语境，动词由第一项的 `显示窗口` 承担 |
| Q-004 | 唤回时是否需要 `NSApp.activate()`？ | 否 | **v0.2 改口（实现用的是强制那一支）**：`set_policy(true)` 里调 `activateIgnoringOtherApps(true)`（`#[allow(deprecated)]`），不是 v0.1 默认的 `activate()`。理由：`activate()` 是协作式的，文档原话"does not guarantee that the app will be activated at all"，而唤回是用户主动点的。**归因要说老实话**：v0.1 那次"点了没反应"后来查明是泵挂错钩子（见事实表 D-21 那条 FACT），不是 `activate()` 不够，所以"非 deprecated 的那条到底够不够"这一条**既没被证明也没被证伪**；替代它的 `NSRunningApplication.activate(from:)` 要 macOS 14 + `NSApplicationActivationReason`，我们的窗口不靠系统唤起，没有收益。真机若出现"抢焦点打断用户"，先换回 `activate()` 试——两种结果都可接受，属观感不是结构，登记 REQ-004 的 HYPOTHESIS 而不设停止条件 |
| Q-005 | `.accessory` 期间主菜单（应用名那排）会消失，`Cmd+Q` 还有效吗？ | 否 | **已核实不冲突**：本应用的快捷键从来不走 App 主菜单，`Cmd+Q` / `Cmd+W` 由 `system::hotkeys` 在 egui 输入里分类（`hotkeys.rs:41-42`）。隐藏态窗口收不到键，此时退出入口是托盘菜单那颗（这正好是"第三个可见入口"要成立的场景）。真机若发现某档系统下快捷键行为与推断不符，按停止条件 4 |
| Q-006 | 打包后的 `.app`（P8）Info.plist 要不要写 `LSUIElement`？ | 否 | **本包不碰**：运行中切策略已满足"关窗后磁贴消失"。`LSUIElement` 会把应用变成"永远不进 Dock"，与用户选的中间档（唤回时磁贴回来）不符。这条写进 `docs/05` §8 与 ROADMAP 待办，属 P8 |
| Q-007 | v0.2 新增的第四项叫什么、点它时窗口还收着怎么办？ | 否 | **已决（实现选择）**：文案就叫 `设置`——菜单是短语境，同排已有 `显示窗口` 那个动词形状，不必写成"打开设置"；位置排在 `退出程序` **之前**，把"改变世界"的三项和"结束进程"那一项隔开。窗口收着时 SHALL 先唤回再开设置页（`Action::OpenSettings` 在 `app::run` 里就是 `reveal` + `effects.open_settings`），兑现复用 `app::open_settings`，与工具栏那颗、提示条「去设置」同一个入口——托盘不许是第二条开设置的路 |

## 允许动作与停止条件

**批准即允许**：本地实现与验证；在 `crates/ice-app/Cargo.toml` 给 `objc2-app-kit` 开那 8 个 feature（`NSStatusItem`/`NSStatusBar`/`NSStatusBarButton`/`NSResponder`/`NSMenu`/`NSMenuItem`/`NSImage`/`NSRunningApplication`）、给 `objc2-foundation` 开 `NSData`，并在注释里逐条给编译面证据；新增 `crates/ice-app/src/system/tray.rs`（含 `define_class!` 的 target、`thread_local!` 的强引用、`glyph()`/`glyph_bytes()` 纯函数）；给 `Action` 加 `RevealWindow` 与（v0.2）`OpenSettings` 两条并接进 `app::run` 的分派，两条都**只转接给既有实现**（`reveal`、`open_settings`），不开第二条路；把托盘泵挂在 `App::logic`（v0.2 的实现更正）并把 `install` 需要的后备比例因子从 `frame.winit_window()` 取进来；改写 `mode::hide`/`mode::reveal` 与 `TrayState` 分支；重写 `hide_cancels_close_and_minimizes`、`hide_window_keeps_the_process_and_the_content` 与那四条 `RevealGuard` 测试（保留"进程不退、内容原样、命令幂等"三条主张）；改设置页那句说明的文案函数；按 REQ-005 回写 `docs/traceability.md` 的 FR-5.2/FR-5.4 两行、`docs/01` FR-5.4 ② 那半句（三项→四项）、`docs/02` 的 `system::tray` 行、`docs/03` §4、`docs/05` §8、`docs/06` 的 TC-5.4-a/b/c 结论列与 W-2.40、W-2.12 的判据改口、`docs/07-acceptance.md` 变更登记与 §5 偏差第 2 行的关闭确认；真机跑一次 `ps`/CPU 采样与托盘走查；阶段验收后 commit + push。

**不允许**：未经登记就改需求基线（含把 FR-5.4 的四条口径自己放宽，或把菜单项数改成四项以外）；**自验界面不许用全屏 `screencapture`**——2026-11-04 那一轮全屏截图把用户另一块屏上的第三方内容一起扫进了临时图，本包的走查一律由用户在自己机器上看（临时图当时已删）；在托盘不可用时只发 `Visible(false)`（REQ-006 的混合状态）；把"关窗"改成"退出进程"或反过来——红灯与 `Cmd+W` 是收起、`Cmd+Q` 与那颗 `退出程序` 是退出，两条语义不许混；为托盘引入新的第三方 crate（`tray-icon` 一类）；把托盘图换成彩色方砖或用文本字符当图标；把窗口内容、输入文本、识别文本或任何用户数据写进日志、证据或提交（只允许 `outcome`/`policy`/`hidden`/错误码/耗时一类计数）；未经确认执行 `rm`；改动用户自己的文件；伪造 `./sdd record` 的运行时/人工证据（磁贴消失、深浅色可见、点一下真的回来，这些我只能由你定论）；为了过门禁修改已记录的证据件；把 `hide_window_keeps_the_process_and_the_content` 直接删掉而不是按新语义重写。

**停止条件**（触发即记 `SPEC_DEVIATION` 退回评审，不自行改需求或降级 Must）：

1. **右键与左键在真机上分不开**（`sendActionOn` 没登记成功、右键不触发 action、或菜单一挂左键就不走 action）：FR-5.4 第②句"左键唤回、右键出菜单"是本包的核心口径，实现层的两种退路（两键都出菜单 / 常驻 local monitor）**都改变行为或引入新的常驻监听**，得你选。**——未触发**（2026-11-04 走查：左键唤回、右键出菜单都成立；两条退路一条没用上）。
2. **`.accessory` 切过去后 Dock 磁贴没消失，或切回 `.regular` 磁贴不回来**：这是用户裁定 ① 的直接落空。可能原因是打包/未打包形态下系统对运行中切策略的反应不同（事实表只证明了 winit 不会抢回去，没证明磁贴一定消失）。此时选项是"保留磁贴、只隐藏窗口"（回到三档里的第一档，需你改判）或"改 `NSApp.setActivationPolicy` 的时机/次序"——第一条是需求改判，我不会自己选。
3. **托盘项在真机上根本装不出来**（`systemStatusBar` 返回的对象没有可用按钮、`statusItemWithLength` 出的项不可见、或绑定层与本仓 objc2 版本有未预见的编译冲突）：FR-5.4 的 Must 在本机等于没有。选项是"换 `NSStatusItem` 长度/显式 `setVisible(true)`"、"引第三方 `tray-icon`（违背最小依赖面与本包'不允许'那一条）"——第二条要重开依赖决策，得你批准。
4. **隐藏态唤回的链路在某个 macOS 版本上不成立**（回调里 `request_repaint()` 叫不醒 `ControlFlow::Wait`，或 eframe 那套"直接给不可见窗跑一次 `update()`"在别的系统版本行为不同——v0.2 已把措辞改准：跑的是 `logic()`，`ui()` 与绘制仍按可见性跳过）：那 `Visible(false)` 就不能用。此时按 REQ-006 的降级路径整体退回"取消关闭 + 最小化"，并回来登记 FR-5.2/FR-5.4 两条的重新裁定。**——未触发**：`Visible(true)` 在隐藏期被处理、窗口真的回来，2026-11-04 走查定论。
5. **唤醒只能靠 `Reopen` 才能可靠回来，而托盘意图那条通道有我们没预料到的时序问题**（回调线程、菜单跟踪模式里 action 不返回等）：托盘的语义就要重设计，属设计变更，回来对齐 `docs/02` 那行。**——v0.2 部分命中并已就地解决**：菜单追踪循环确实是阻塞的（追踪期间会重进同一个 `thread_local!`），但那是实现层的 `RefCell` 双重借用，把四个 `Retained` 先克隆出来、释放借用再弹就够了，托盘语义没变，不必重设计。真正"没预料到"的是另一件事：泵挂在 `App::ui` 而隐藏期根本不调 `ui()`——它不是时序问题，是帧钩子选错，登记为缺陷 D-21 并写进事实表那条 FACT。
6. **隐藏期间 CPU 实测明显高于 1%** 且查明是"续期"之外的原因（例如 eframe 对不可见窗的 100ms 直接绘制本身在空闲时也烧）：NFR-2 的这条数不改；要么把重绘彻底让给意图驱动（可能需要不依赖 `update()` 的唤醒方式），要么 FR-5.2 的"隐藏常驻"与 NFR-2 冲突，需你在两条之间定优先级。
7. **走查发现唤回后的几何不对**（尺寸槽被历史值写坏、停靠边缘的窗口回来时弹到屏幕外、简洁窗变成全量尺寸）：与 P5 的 D-6/D-10/D-14 同一类缺陷。改版式与护栏属设计变更，回来对齐 `docs/03` 与 `docs/02` 的既有护栏定义。

## 时间估算

- 开发 4~6 小时。分块：`Cargo.toml` feature 增量与编译面证据约 0.5 小时（8+1 个开关，漏一个就是"方法看不见"这类查半天的坑，`NSRunningApplication` 那条已经预见到）；`glyph()` + `glyph_bytes()` 与其四条无头判据约 1 小时（画线段本身快，卡在"18pt 下认不认得出"要来回看）；`system::tray` 的 mac 侧约 1.5~2.5 小时（不确定性全在这块：`define_class!` 的 target 强引用、`sendActionOn` 与 `currentEvent` 的分叉、菜单四项的 `initWithTitle_action_keyEquivalent` + weak target，四条停止条件里三条落在这里）；`mode::hide`/`reveal` 改道 + `Action::RevealWindow`/`OpenSettings` + `TrayState` 分支约 1 小时；那五条既有测试的重写约 0.5 小时（主张要留住，只换前提）。预计新增 18~26 项测试、约 450~700 行（含 `tray.rs` 整文件与测试）。
- **实际（v0.2 收口时补记，估算错了要写在明处）**：第一轮的估时**没算走查返工**，而本包真正的成本全在那儿——`菜单三项` 画法是猜的（用户否掉，重画成应用图标的双气泡）、泵挂错帧钩子（`App::ui` → `App::logic`，D-21，两轮才查准，第一轮我错怪了 `activate`）、菜单少了「设置」（v0.2 的改判就来自这里）。三轮走查合计比 4~6 小时那一档多出一倍上下。教训不在工时，在前提：**"能不能唤醒"这件事只有把窗口真的收起来才暴露得出来**，无头测试当时全绿而链路是断的——所以 v0.2 把 `an_intent_handed_over_while_hidden_still_opens_the_window` 这类"收起期"断言补成了硬判据。
- 文档回写约 0.8 小时（`docs/traceability.md` 两行、`docs/03` §4、`docs/05` §8、`docs/06` 三条 TC 结论列 + W-2.12 判据改口 + W-2.40、`docs/07` 变更登记与 §5 偏差关闭）。`docs/01` 本轮已写完，FR-5.2/FR-5.4 的文字实现时不动。
- 验证 1~1.5 小时：`cargo test --workspace` + `clippy --workspace -D warnings` + `fmt --check` + `cargo tree -p ice-app` 证明没把 default 拖进来的那六个 Apple 绑定 crate + `check_traceability.py`（45 ↔ 45）+ `./sdd scan`；真机走查一轮（需要你配合约 10~15 分钟：关窗看 Dock 与菜单栏、左键唤回、右键菜单四项各点一次、深浅色菜单栏各看一次图标、隐藏十分钟采一次 CPU）。**2026-11-04 走查进度**：托盘那一组（左键唤回 + 右键出菜单 + 菜单能用）你已定论"托盘的可以了，没问题"；磁贴真的消失/回来、唤回后尺寸原样、深浅色两档都看得见、隐藏十分钟的 CPU 这四件**没有单独定论**，V-010 与 W-2.40 据实记成部分覆盖，不写成全过。
- 外部等待：你的 spec 批准一轮（本包一条 `./sdd approve CHG-008`——批准不能自批）、W-2.40 走查、验收签名一轮；不计入开发工时。
- 依据：本包的无头可测比例低于 CHG-005（识别链那半边全是纯函数，这里的核心是一块只能真机看的原生接线），但**不确定性比 P6 小**：绑定逐条核对过、eframe 那条"不可见窗仍处理 `Visible(true)`"的机制在源码里读到了实现而不是注释。风险集中在停止条件 1~3，全部落在同一台机器上可当场定论，一旦跑通就没有长尾。
