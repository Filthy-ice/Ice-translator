Change: CHG-004
Version: v0.7
Spec-Hash: 00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

# CHG-004 · Design

- Change: CHG-004 · v0.3 · `spec_hash 235deaf136a3d2a0f9d8585b72f05f2b8948470f70d9e26629c5f5872091fa9c`
- Repo: `ice-translator`（分支 `main`）
- 前置：CHG-003 已 ACCEPTED（`f123a8b`），代码态 246 passed / 0 failed / 3 ignored

## 实现方案

### 模块划分

| 文件 | 动作 | 内容 |
|---|---|---|
| `crates/ice-app/src/mode.rs` | 扩 | `MIN_COMPACT_SIZE`、`commands_for_mode` 改（按形态给最小尺寸 + 两种形态都允许缩放 + 全量也下发 `OuterPosition`，即 REQ-004 的瞬时切换）、`snap`/`dock`/`reveal_rect`/`released`/`fit_into_area`/`dim_alpha` 等纯函数（v0.5：`cycle_edge` 下线）、`DragSettler` 拖动/缩放静止判定、`window_level`（停靠态强制 Floating） |
| `crates/ice-app/src/views/compact.rs` | **新** | 简洁窗版式：把手条、输入行、译文定高 `ScrollArea`、动作行（全量 / 复制 / 设置 / 机翻\|AI / 置顶；v0.5 起正常字号、放不下换行，无停靠指示）。P4 暂由 `app.rs::compact` 私有函数代管，本包迁出（`docs/02-architecture.md` §2 的 `views::compact` 从"—"变实体） |
| `crates/ice-app/src/views/mod.rs` | 改 | `engine_picker` 提为 `pub(crate)` 供简洁窗复用；`Effects` 加 `start_drag`、`open_settings` 两个字段 |
| `crates/ice-app/src/system/screen.rs` | **新** | 屏幕可用区探针：macOS 走 `NSScreen.visibleFrame`（`cfg(target_os = "macos")` + unsafe），其余平台回退 `viewport().monitor_size`；坐标翻转是纯函数，可无头单测 |
| `crates/ice-app/src/system/clipboard.rs` | 扩 | `Watcher`：纯判定（新内容 / 自己写的 / 焦点占用 / 空文本 → 该不该填）+ 一个薄 `poll()`（impure，只调 `arboard::get_text`） |
| `crates/ice-app/src/system/hotkeys.rs` | 扩 | `Key::Escape → Action::ClearInput`（只在简洁模式生效，全量模式 Esc 仍不绑定任何动作，与 P4 一致） |
| `crates/ice-app/src/state.rs` | 扩 | `submit()` 里插自动反向钩子；`auto_reverse_target` 纯函数；`clear_input()` |
| `crates/ice-app/src/app.rs` | 改 | 删掉私有 `compact()`、接 `views::compact`；切换后回读几何并立即落盘（REQ-004）；`pending_geometry` + `MODE_GEOMETRY_GRACE` 挡住 D-6 的过渡几何回写；拖动/缩放静止后的吸附与落盘；停靠状态机 `dock_shown`/`dock_left_at`/`dock_latched` 与每帧 `watch_dock`（唤出/收回/闸门/层级，REQ-003）；剪贴板轮询挂点；失焦 alpha；`initial_viewport` 加 `with_transparent(true)` |

### 依赖变更

只加 macOS 目标依赖，且三个 crate **已经在 `Cargo.lock` 里**（arboard 3.7 与 eframe/winit 传递引入），因此不需要联网、不新增外部供应链面：

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-app-kit = { version = "0.3", features = ["NSScreen"] }
objc2-foundation = { version = "0.3", features = ["NSGeometry", "NSArray", "NSString"] }
```

`crates/ice-app/Cargo.toml` 末尾本来就有这条预留注释（"P5 若 egui 的 WindowLevel 不足以实现置顶，再引入 AppKit 绑定"）；实际触发原因不是置顶（P4 的 `WindowLevel` 够用），而是**屏幕可用区**——见下一节。

### 一处 Spec 事实更正（不改变任何 REQ）

REQ-003 写的是"屏幕可用区 SHALL 取去掉菜单栏与 Dock 的那一个（egui 的 `ctx.available_rect` 不够，用 `MonitorWorkArea`/viewport 的 monitor 信息）"。写规格时假设 egui 有可用区 API，实测源码后确认**没有**：

- `ViewportInfo` 只有 `monitor_size: Option<Vec2>`（整块屏幕的逻辑尺寸，含菜单栏与 Dock），没有任何 work area 字段；`inner_rect`/`outer_rect` 是窗口自己的（`egui-0.37.0/src/data/input/viewport_info.rs:28-66`）。
- winit 0.30 的 `MonitorHandle` 也只暴露 `size()`/`scale_factor()`/`position()`，没有 `work_area`。

行为不变（仍然吸附到"去掉菜单栏与 Dock"的可用区），改的是手段：macOS 上用 `NSScreen.visibleFrame`（AppKit 定义就是"去掉菜单栏与 Dock 的那块"），通过 `objc2-app-kit` 读取；非 macOS 平台回退到 `monitor_size` 并把上边界内缩 25pt 当菜单栏近似，同时在证据里记明"非 macOS 的可用区是近似值，精确判定属 P8 的跨平台核查（NFR-7）"。

坐标系换算必须与 winit 一致，否则吸附位置会整体偏移一个屏幕高度：winit 的 `outer_position` 用 `flip_window_screen_coordinates`（`winit-0.30.13/src/platform_impl/macos/monitor.rs:401-409`）——`y_topleft = 主屏高度 - rect.height - rect.origin.y`，x 不变，且**主屏高度取 `CGDisplay::main()` 而不是 `NSScreen.mainScreen`**（注释明写：屏幕坐标是相对主显示器的，与窗口当前在哪块屏无关）。我们用 `NSScreen.screens()[0].frame().height` 等价取得主屏高度（AppKit 的 screens 数组第 0 个就是主显示器），避免再引 `objc2-core-graphics`。

选哪块屏：把每块屏的 `frame` 与 `visibleFrame` 都换算到同一坐标系，取**包含窗口中心**的那一块的 `visibleFrame`；一块都不含（窗口被拖到屏幕外）则退回 `mainScreen`。这段选择逻辑是纯函数，输入是 `Vec<(frame, visible)>` + 窗口 rect，输出是可用区 rect。

### 三条 HYPOTHESIS 的最小实测（已完成，2026-10-22）

探针 `crates/ice-app/examples/viewport_probe.rs`（一次性 example，不进正式产物；v1 测三条假设，v2 加测"30fps 降频"与"窗内补间"两条替代路径，v3 把窗内补间做成双向对称并加一个"瞬移"对照）。判定书：`features/CHG-004/evidence/T-001-hypothesis-verdicts.md`，原始日志 `T-001-probe.log` / `T-001-probe-v2.log` / `T-001-probe-v3.log`。

| # | 实测 | 结果 | 处置 |
|---|---|---|---|
| H1 | 无边框 + `Resizable(true)` 能否边缘缩放 | **成立**：拖角时 inner 以 ~16ms 间隔连续变化 20+ 台阶（651×437 → 831×549），用户驱动的缩放本身是 60fps | 按 REQ-001 原方案：走 AppKit 原生边缘缩放，不自绘手柄 |
| H2 | 逐帧 `InnerSize` 补间是否平滑 | **不成立**：60fps 与 30fps 各三次，每 200ms 只有 4~5 次请求落到窗口上，且三次复现整个事件循环停摆 ~1.0s（1006/1026/1086ms）。用户判定"一卡一卡的，ppt 了"。设计里写的第一级降级（降到 30fps）同样不成立 | 停止条件 3 触发 → 已记 `SPEC_DEVIATION`，Spec v0.2 曾把机制换成"窗内补间" |
| H3 | `with_transparent(true)` 是否有副作用 | **成立**：无黑底/花屏/圆角异常，失焦后画面无变化。用户判定"第三项通过" | 按 REQ-007 原方案：窗口级半透明，不降级成内容级 `Ui::set_opacity` |
| C（单向） | 窗内补间：动画期窗口几何不动、只插值内容矩形、收尾一条 `InnerSize` | **成立**：204ms / 14 帧 / 节奏 16.5ms±1ms（真 60fps），面板尺寸单调无回跳，端点误差 0px。用户判定"补间现在很丝滑" | v0.2 曾采纳；但成立前提是**动画全程窗口不 resize**，只能做"大 → 小"单向 |
| C（双向，v3） | 起点把窗口撑成两端 rect 的外接矩形（1160×636）再插值 | **不成立**：三次跑分别 1027 / 1028 / 1042ms，内容只画出 3~4 帧；同一次运行里"外接矩形恰好等于当前 rect"（窗口尺寸没变）的那次 13 帧满速跑完 203ms → 停摆真因是 **surface resize 本身**，与命令频率无关 | 用户要求双向对称，而双向做不到 → 停止条件 8 触发 |
| JUMP（v3 对照） | 单发 `InnerSize` + `OuterPosition` | **成立**：发命令到几何落定 **20ms**（09:24:51.289 → 51.309），无停摆无中间态 | 用户决定"算了不要动画了，都做成瞬间的" → FR-3.9 改需求为瞬时切换（Spec v0.3、REQ-004 重写） |

第四条 HYPOTHESIS（剪贴板轮询的空闲开销）不在这次探针范围内，留到 REQ-008 实现后用 `ps -o time` 30 秒采样验证（同 CHG-003 V-010 的手法）。

### 并发与心跳策略（谁挂 `ui`、谁挂 `logic`）

P4 的确证结论继续适用：窗口隐藏期间 eframe 走 `update_logic_only`，只调 `App::logic`，`App::ui` 一帧都不跑。

| 机制 | 挂点 | 理由 |
|---|---|---|
| 形态切换（瞬时） | `App::ui`（一次性排命令，不续重绘） | 只在可见期发生；隐藏期只改配置里的目标形态（REQ-004 边界），`reveal()` 时按目标形态一次性设好几何 |
| 拖动/缩放静止判定与吸附 | `App::ui` | 依赖 `viewport().outer_rect` 的逐帧比较，隐藏期没有拖动 |
| 剪贴板轮询 | `App::logic`（读剪贴板）+ `App::ui` 里续 `request_repaint_after(700ms)` | Q-005 定为"只在可见时即翻"，所以实际续期发生在 `ui`；挂 `logic` 是为了在恢复可见的那一帧就能立刻读到最新剪贴板，不必等下一次重绘 |
| 恢复看护（`RevealGuard`） | `App::logic`（P4 已有） | 不动 |
| 空闲策略 | 三者都不活跃时**不**发任何 `request_repaint_after` | NFR-2：P4 实测 30 秒 0.000s CPU 必须保持；`clipboard_watch` 默认关闭就是为此 |

## REQ-001 实现与验证

`commands_for_mode` 的 Compact 分支相对 P4 改三处（完整函数体见 REQ-004 一节，两节共用同一份实现）：

```rust
UiMode::Compact => vec![
    ViewportCommand::Decorations(false),
    ViewportCommand::Resizable(true),                // 用户 2026-10-22 决定：简洁窗要能鼠标缩放（P4 是 false）
    ViewportCommand::MinInnerSize(MIN_COMPACT_SIZE), // 按形态取下限，解掉全量留下的 640×420
    ViewportCommand::InnerSize(clamped.w/h),         // 约束改过之后才设尺寸
    ViewportCommand::OuterPosition(rect.x/y),
]
```

顺序是正确性的一部分：`InnerSize` 走 `window.request_inner_size`，平台不允许时直接返回当前尺寸并忽略请求（`egui-winit-0.37.0/src/lib.rs:1969-1985`），所以"改约束"必须排在"设尺寸"之前——W-2.1 的根因就是 P4 切简洁时从没改过那条 640×420 的约束。Full 分支同理下发 `MinInnerSize(MIN_FULL_SIZE)`：两个形态各自带自己的下限，谁也不把值留给对方。

不用 `MinInnerSize(Vec2::ZERO)`：它等价于 `set_min_inner_size(None)`，确实能解开 W-2.1，但同时把下限一起解掉了，320×130 就只剩 `sanitize()` 在加载时夹一次——用户把窗口拖到 200×60 时没有任何东西拦着，得等写回配置、下次启动才被夹回来。按形态取下限则由 AppKit 的原生约束逐帧执行，根本拖不下去（spec v0.3 REQ-001 的澄清）。`MaxInnerSize` 本包不下发：需求是"不设上限"，而 egui/winit 的默认就是无上限，再发一条 `Vec2::INFINITY` 是多余命令。

`MIN_COMPACT_SIZE = Vec2::new(320.0, 130.0)`（v0.5 由 320×96 抬上来，Q-011：动作行不再压缩字号，96px 装不下一行正常按钮 + 任何译文）；`clamp_compact_rect(WindowRect) -> WindowRect` 把小于下限的宽高抬到下限（配置被手改坏也不拒绝启动）。`ice-core::config::sanitize()` 用同一组常量夹这两个下限（REQ-001 边界）：现有实现夹的是 `compact_rect.w.max(320.0)` / `h.max(64.0)`，**64 → 96 是 v0.4 改的，v0.5 再改成 130**，两个下限必须始终与 `MIN_COMPACT_SIZE` 同源，否则配置里能存下原生约束不允许的值。单测覆盖"手改配置成 100×40"。

**D-6：切换过渡期的几何不能写回配置**（v0.5 新增，事实表那条 D-7 的成因②）。`mode::apply` 只下发一组一次性命令（REQ-004），`InnerSize` 是异步请求，而 `app.rs:575` 的 `mode::capture` 每帧把实时几何写回**当前形态**的配置槽——切换那一帧之后 `mode` 已经是 Compact、窗口还是全量大数据，于是 `compact_rect` 被写成 642×422，而没有任何人会再发一次缩小请求，简洁窗就此永久停在大数据上（走查实证：2026-10-23 盘上 `compact_rect` 的宽高与 `full_rect` 一字不差；反证：全新启动不切形态时目标尺寸不会被改写）。实现：`IceApp` 记 `pending_geometry: Option<(Vec2 /*目标内尺寸*/, Instant)>`，由 `apply`/`toggle` 在算出目标形态命令时一起写下；`capture` 在它仍是 `Some` 时**只写位置轴、不写尺寸轴**，直到实时内尺寸命中目标（±2px）或超过 `MODE_GEOMETRY_GRACE`（1s，取探针实测 20ms 的 50 倍余量）后清空。清空后行为与现在一致，用户手动缩放照样写回。

版式：`views/compact.rs` 用 `Panel::top`(把手 10px + 输入行 34px) + `Panel::bottom`(动作行) + `CentralPanel`(译文区)。**动作行不再 `exact_size`**：v0.5 起它是 `horizontal_wrapped` 的自然高度（一行约 24px，放不下变两行），译文区吃剩余高度——窗口被拉大时多出来的高度全给译文区（REQ-001 的"缩放到任意合法尺寸三块都不被裁"）。选换行而不是压字号的理由写在 spec REQ-005：无头测试用的内置字体没有中文字形，量出来的文字宽度必然偏小（P4 的 D-1 正是"断言全绿而按钮被裁出窗口"），而 `wrapped` 布局自带"不超出可用宽度"，与字体度量无关。

**验证**：`commands_for_mode` 的命令集合与顺序（`MinInnerSize(MIN_COMPACT_SIZE)` 与 `Resizable(true)` 都必须排在 `InnerSize` 之前，用 `.position()` 断言，沿用 P4 `switching_to_compact_queues_a_borderless_small_window` 的手法；那条测试里断言的 `Resizable(false)` 要跟着改成 `true`）；两个形态各自的下限不串台（切到简洁后命令里不得出现 `MIN_FULL_SIZE`，反之亦然）；`clamp_compact_rect` 三例（正常/过窄/过矮）；`sanitize` 往返一例（100×40 → 320×130）；D-6 一例接线测试（切到简洁后喂一帧"仍是全量大小"的实时几何，断言 `compact_rect` 的宽高不动，grace 过期后再喂才允许写回）；真机 W-2.1 + 新增 W-2.17（拉边缘缩放）。

## REQ-002 实现与验证

把手三处，全部走 `ViewportCommand::StartDrag`（原生拖拽，不自绘坐标）：

1. **顶部 10px 把手条**：`Panel::top("compact_handle").exact_size(10.0)`，`Sense::drag()` + `response.on_hover_cursor(CursorIcon::Grab)`，`drag_started()` 时发命令。`Frame::side_top_panel` 自带 `Margin::symmetric(8,2)`，10px 的条只剩 6px 可用，所以这一条用 `Frame::NONE.fill(panel_fill)` 铺满。
2. **动作行空白**：左侧按钮组与右侧两个指示按钮之间的空隙单独成一块 `interact(..., Sense::drag())`，左右各留 4px 余量，避免吃掉按钮的命中区。
3. **译文区尾部空白**：译文 `ScrollArea` **下方**那条 8px 带（不是文本右侧——右侧正是滚动条要留的位置，REQ-002 明写滚动条要能拖）。这块带在滚动容器之外，从 `CentralPanel` 的矩形里切出来，所以它既不撑高内容也不造出假滚动条。

三处把手是实现成**三个互不重叠的显式矩形**、各自 `ui.interact`，不是"整块窗覆盖一层透明把手"：egui 的命中测试取最后画上去的那个 rect，把手若叠在控件上会把控件吃掉（egui 自己的 `Window` 标题栏避让关闭按钮用的就是这一招）。三个矩形与 `drag_handle` 的实际位置用 `ctx.data` 记下来，供单测断言"都在窗口内且两两不相交"。

`Sense::drag()` 而不是原设计的 `Sense::click_and_drag()`：后者要等指针走过 `max_click_dist`（6.0px）才把 response 标成 dragged，与 REQ-002 的"按下任一处 SHALL 立即发 `StartDrag`"矛盾；纯 `drag()` 在按下那一帧就置位。代价是把手区不能再被点击——三块都是空白区，本来没有点击语义。

未聚焦时第一次按下会被吞：`StartDrag` 的实现被 `window.has_focus()` 挡着（`egui-winit-0.37.0/src/lib.rs:1961-1968`，注释说 x11 上不判焦点会永久吃掉输入）。处置：把手 `drag_started()` 时若 `ctx.input(|i| i.viewport().focused) != Some(true)`，先发 `ViewportCommand::Focus`（同帧命令按序执行，`Focus` 在 `StartDrag` 之前入队），真机验证"一次按下就能拖"。若实测仍要两下，走停止条件 4。

不触发拖动的控件：输入框（`TextEdit` 自己吃掉事件）、四个按钮、引擎分段、译文文本、滚动条——实现上把手只覆盖"空白区域"，绝不与控件 rect 重叠；单测用无头 `run_ui` 在不同指针位置断言 `Effects::start_drag` 有无被置位（沿用 P4 的 Effects 断言手法）。

松手触发吸附：原生拖拽期间 egui 收不到 pointer 事件（AppKit 进了嵌套事件循环），所以用**静止判定**——`DragSettler::feed(outer_rect, now) -> Option<Rect>`：`start_drag` 发出后置 `armed`；此后每帧比较 rect，变化就刷新 `last_move`，连续 `DRAG_SETTLE = 150ms` 不变则返回最终 rect（交 REQ-003 吸附 + REQ-009 落盘），返回后解除 `armed`。纯函数，单测喂一串带时间戳的 rect 序列（移动中 → 停 150ms → 返回一次且只返回一次）。

实现时要加三条原设计没写的规则（都是写单测时暴露出来的）：
- **读不到几何的那一帧按"还在动"处理并刷新计时**。原先这里是 `let rect = outer?;`，直接把计时停在旧值上，于是拖动中途就能判出静止、把用户还按着的窗口吸到边上——子像素抖动同理，比较时用 0.5px 容差而不是 `==`。
- **armed 期间每帧续 `request_repaint_after(DRAG_SETTLE)`**。原生拖拽结束后事件循环可能再没有理由画一帧，不续就永远等不到"静止满 150ms"，吸附形同没做。
- **程序驱动的移动一律 `disarm()`**（`open_settings`、形态切换、以及 REQ-003 的唤出与收回——v0.4 这里列的是 `cycle_dock`，该控件已随 Q-010 下线），否则自己发的 `OuterPosition` 会被当成"用户松手了"再吸一次。吸附回写前还要把吸附结果与松手 rect 比一次（≤0.5px 且边相同就跳过），防止吸完又触发下一轮。

**验证**：把手命中/不命中（4 组指针位置）、`StartDrag` 入队、未聚焦时先入队 `Focus`、`DragSettler` 五例（未 armed 不返回、移动中不返回、静止不足不返回、恰好静止返回一次、再次移动重新计时）；真机 W-2.13（三处把手各拖一次，且拖输入框/按钮不移动窗口）。

## REQ-003 实现与验证

v0.5 把停靠改成"只认左/右两条边 + 悬停唤出"（Q-010），因此不再是两个纯函数，而是**四个纯函数 + 一份运行时状态机**。全部落在 `mode.rs`（纯函数）与 `app.rs`（状态机接线），`views/compact.rs` 只少了停靠指示这一个控件。

**吸附 `snap(window, area, threshold, sliver) -> (Rect, DockEdge)`**：只在左/右两条边里取带符号距离最小的那条

```
d_left  = window.min.x - area.min.x
d_right = area.max.x - window.max.x
d = min(d_left, d_right)
d < threshold ──► dock(window, area, 胜出的那条边, sliver)
否则            ──► (window, DockEdge::None)
```

上/下两条边**整个从判定里删掉**（v0.4 的四边对称作废）：藏进菜单栏或 Dock 之后那 3px 不在用户会去悬停的位置，"鼠标放上去自动出来"在那里不成立。`DockEdge::Top`/`Bottom` 留在枚举里只为反序列化兼容，运行时不再产生，启动读到就按 `None`。

**停靠位 `dock(window, area, edge, sliver) -> Rect`**：Right → `x = area.max.x - sliver`；Left → `x = area.min.x - w + sliver`。窗口藏到边外，可视区内只剩这条 `sliver` 宽的、贴在屏幕最外沿的窄边——光标推到屏幕边必然落进它，这是悬停唤出的几何前提。夹取仍是两轴分开：**交叉轴**（y）全量夹进可用区，**停靠轴**（x）只在窗口宽过可用区时把起点夹到 `area.min`（此时 `area.max - w` 是负数，照抄会把窗口甩到屏幕另一侧之外）。

**唤出位 `reveal_rect(window, area, edge) -> Rect`**：Right → `x = area.max.x - w`；Left → `x = area.min.x`；y 沿用现值并夹进可用区。唤出**不改** `docked_edge`，只改运行时 `dock_shown`。

**拖离 `released(edge, anchor, settled, threshold) -> bool`**：与 v0.4 同一条（相对停靠锚点沿停靠轴向屏幕内的位移 ≥ 阈值；交叉轴位移不计）。锚点在唤出时 SHALL 搬到唤出位、吸附时搬到收起位，否则"从唤出态向内拖"量不出位移。

**运行时状态（`IceApp`，一律不持久化）**：

```
dock_anchor : Option<Pos2>     // v0.4 已有；吸附/唤出/收回三处都要搬它
dock_shown  : bool             // 停靠态当前是唤出（true）还是收起（false）
dock_left_at: Option<Instant>  // 指针离开窗口的时刻，DOCK_AUTO_HIDE = 500ms 的计时起点
dock_latched: bool             // 闸门：true 表示"指针还欠一次离开创口带"，不许唤出
```

闸门在两处置起：**吸附落位时**（用户把窗口拖到边上、松手即收起，此时指针正压在屏幕边上，不置闸门就会立刻再弹回去，停靠看起来"没生效"）与**每次自动收回时**（否则 500ms 后指针仍在 3px 上 → 唤出 → 指针离开 → 收回，来回抖）。解除条件只有一个：指针离开创口带（热区沿停靠轴向屏幕内扩 `dock_sliver_px` 的那一条）后再次进入。

**每帧看护 `watch_dock(ctx)`**（挂在 `ui` 里、`settle_drag` 之后）：

```
停靠边 ∉ {Left, Right}                    → 层级恢复 window_level(always_on_top)，四个状态清空，返回
drag_settler.is_armed()（原生拖拽途中）    → 什么都不做（那时 egui 收不到 pointer 事件，判了也是错的）
docked_edge ∈ {Left, Right} 且层级 ≠ Floating → 发 WindowLevel::Floating（Q-012：不论 always_on_top，否则那 3px 收不到事件）
!dock_shown 且 !dock_latched 且 指针在窗口内   → place(reveal_rect) ；dock_shown = true；anchor 搬到唤出位；request_repaint()
dock_shown 且 指针不在窗口内：
    正在窗口内按住（拖动/选字）或 focus_in_source → 不动，清空 left_at
    否则 left_at 起算 ≥ DOCK_AUTO_HIDE          → place(tucked_rect)；dock_shown = false；dock_latched = true；anchor 搬到收起位；request_repaint_after(DOCK_AUTO_HIDE)
dock_latched 且 指针在创口带之外                → dock_latched = false
```

指针在不在窗口内取 `ctx.input(|i| i.raw.pointer_in_window)`（winit 的鼠标进入/离开事件会自己唤醒事件循环，因此收起态**不需要**常驻重绘心跳；只有 500ms 收回计时需要一次性 `request_repaint_after`）。落位走 `place_compact` 那条既有路径（发 `OuterPosition` + 写 `compact_rect` + 立即落盘）。

**启动落位**：`docked_edge ∈ {Left, Right}` 时按 `dock(compact_rect, area, edge, sliver)` 重算一次再下发（幂等），初始 `dock_shown = false`、`dock_latched = true`。这一条顺带解决"在唤出态被强杀"——盘上存的是唤出位 x，直接照搬会让窗口以"停靠"之名整条可见地挂着。REQ-009 的启动越界校验配套改口径：停靠态按"那 `sliver` 是否还在可用区内"判，不按整窗判（否则左停靠的负 x 每次启动都会被当成越界拉回屏幕中心，等于每次启动都替用户解了停靠）。

`fit_into_area` 仍然实现并被 REQ-009 的启动/插拔恢复用（`snap` 不用它）。它在本包还有一个消费者：松手时窗口整条落在可用区之外（`!area.intersect(settled).is_positive()`），此时不判吸附，先 `fit_into_area` 把窗口拉回可视区。

`area` 来自 `system::screen::work_area(ctx, outer_rect)`（见"Spec 事实更正"）；探针失败时回退 `monitor_size`，并把这次回退记进日志（`warn_once` 语义，避免每帧刷屏）。

**随 v0.5 下线的东西**：`mode::cycle_edge`、`IceApp::cycle_dock`/`move_to_edge`、`views::compact::dock_label`、以及动作行里那个 `btn_dock` 控件与它的布局断言 salt（K-07 作废，Q-010 第④条）。`snap` 里 Top/Bottom 两个分支同样删除。

**验证**：
- 纯函数：`snap` 左右各吸一次 + "上下够近也不吸"一例（锁住 v0.5 砍掉两条边）+ 四边都远不吸；`dock`/`reveal_rect` 的公式与两轴夹取各一例（含"窗口宽过可用区"）；`released` 左右各一例 + 交叉轴滑动不释放 + `None` 恒 false。
- 状态机（`app.rs` 接线，喂 `PointerMoved`/`Gone` 事件与几何）：进入热区→唤出并置 `WindowLevel::Floating`；离开 500ms→收回且置闸门；收回后指针仍在 3px 上→不反弹（断言一个命令都不发）；指针离开再回来→闸门解除并可再次唤出；按住不放或输入框有焦点→不收回；吸附落位后指针在边上→不立即弹回；拖离→层级恢复 `window_level(always_on_top)`。
- 布局：`views/compact.rs` 的 `btn_dock` 相关断言随控件一起删。
- 真机走查：左/右各吸一次、悬停唤出与自动收回各一次、"正在打字时不收回"一次、向内拖离一次（W-2.5、W-2.6，W-2.18 改为"停靠态鼠标推到屏幕边上会不会弹出来"——它同时是那条 HYPOTHESIS 的定论，不成立触发停止条件 10）。

## REQ-004 实现与验证

机制是**瞬时切换**（2026-10-22 需求变更定案）。逐帧 `InnerSize` 补间与"双向窗内补间"都已被真机实测否掉，单发几何命令实测 **20ms 落定**（见"三条 HYPOTHESIS 的最小实测"与 `evidence/T-001-probe-v3.log`）。因此：**没有 `Morph`、没有插值、没有逐帧重绘**，切换就是在同一帧里把目标形态的命令一次性排进 `Effects`。

落点是 `mode.rs` 已有的 `commands_for_mode`（P4 就在用，本包改它的内容而不是新增状态机）：

```rust
pub fn commands_for_mode(cfg: &UiConfig, mode: UiMode) -> Vec<ViewportCommand> {
    let rect = rect_for(cfg, mode);
    let (decorations, min_size) = match mode {
        UiMode::Full => (true, MIN_FULL_SIZE),
        UiMode::Compact => (false, MIN_COMPACT_SIZE),   // REQ-001：简洁窗也要能缩放，下限 320×130
    };
    vec![
        ViewportCommand::Decorations(decorations),   // 原生属性，必须排在改尺寸之前
        ViewportCommand::Resizable(true),            // 两种形态都允许边缘缩放（REQ-001）
        ViewportCommand::MinInnerSize(min_size),
        ViewportCommand::InnerSize(Vec2::new(rect.w, rect.h)),
        ViewportCommand::OuterPosition(Pos2::new(rect.x, rect.y)),
    ]
}
```

与 P4 版本的两处差别，都是有意的：

1. **`Resizable(true)` 与 `MinInnerSize` 对简洁形态也下发**（P4 给的是 `Resizable(false)`），这是 REQ-001；`MinInnerSize` 从"清成 0"改成按形态取值，否则切回全量后简洁窗的 320×130 下限会跟着带过去。
2. **全量形态也下发 `OuterPosition`**（P4 只给简洁形态下发）。不下发时，从贴边停靠的悬浮条切回全量，窗口会留在屏幕边缘那条位置上，与 FR-3.6"记住位置尺寸"矛盾。`full_rect` 存的是外框矩形（P4 已修掉"把标题栏高度算进位置"那个缺陷），所以这里直接用它的 `x/y` 就是对的原点。

顺序有意义：`Decorations` 先于 `InnerSize`。反过来会有一帧按错误的可用高度绘制（装饰回来后内容区矮一截），观感就是"内容闪一下"，正是 REQ-004 禁止的。

驱动在 `app.rs`：切换时 `Effects` 里排这 5 条命令 + `config.mode = 目标形态` + `config_dirty = true`，**不调用 `ctx.request_repaint()`**（切换本身就在一次重绘里发生，之后回到"只有提示条到期与结果到达才重绘"的空闲策略，NFR-2）。下一帧 `mode::capture` 回读真实几何写回 `full_rect`/`compact_rect`，再 `flush_config(true)` 立即落盘——因为命令是**异步请求**（`InnerSize` 走 winit `request_inner_size`），不能把目标值当已生效值写盘；也因为 SIGTERM/强杀不走 `on_exit`。

隐藏态（`state.hidden`，eframe 只跑 `logic` 不跑 `ui`）：切换只改配置里的目标形态，不下发命令、不把窗口叫回来；`reveal()` 时按目标形态一次性设好几何（沿用 P4 的路径）。

不再需要的东西（v0.2 设计里写过，随需求取消一并删掉）：`Morph`/`MorphFrame`/`GeometryCmd`/`union()`/`reverse()`/`MORPH_DURATION`、`state.morphing` 标志、内容矩形插值与 `Ui::new_child(max_rect = …)` 的透明余量画法、`ui.multiply_opacity` 的淡入淡出、objc2 `setHasShadow(false)` 的备用手段（那是为了压外接矩形的阴影），以及 Q-008 记的两条代价（透明区吞点击、阴影跟随大矩形）——瞬时切换下窗口矩形恒等于可见矩形，两条都不存在。

**验证**（全部无头断言，`ctx.viewport(|vp| vp.commands.clone())` 取命令队列）：一次切换里 `InnerSize` 与 `OuterPosition` **各恰好 1 条**、`Decorations` 1 条且排在 `InnerSize` 之前（防止实现悄悄退回逐帧补间）；切换后 `Effects::job` 为空且 `state.in_flight` 未置位（不重发请求）；内容与源/目标语言、引擎选择原样保留（P4 那两条测试必须仍绿）；隐藏态触发切换时命令队列为空、只有配置变化；`capture` 回读的 rect 与目标 rect 一致（±2px）；切换到全量时 `full_rect` 的位置不被标题栏高度污染。真机 W-2.14：连续切 10 次，盯"一步到位、Dock 图标不闪、无黑屏白屏、切完立刻能点"。

## REQ-005 实现与验证

`views/compact.rs` 的版式（460×130 默认，最小 320×130）：

```
┌ 把手条 10px（Grab 光标，拖动）─────────────────────────────┐
│ 输入行 34px：TextEdit::singleline，Enter 提交              │
│ 译文区 剩余高度：ScrollArea::vertical().auto_shrink([false,false])│
│   文本可选中复制；尾部空白是第三处把手                      │
│ 动作行 自然高度（约 24px 一行，窄则两行）：              │
│   全量 · 复制 · 设置 · 机翻|AI · 置顶   （v0.5：无停靠指示）│
└────────────────────────────────────────────────────────────┘
```

- **引擎分段**：直接复用 `views::mod::engine_picker`（提为 `pub(crate)`），它就是 `Button::selectable` 的分段样式，AI 未配置时禁用并带说明——简洁窗不新造一套。
- **译文滚动**：定高 `ScrollArea`（`id_salt("compact_translation")` + `auto_shrink([false,false])`），沿用 P4 缺陷 D-4 的手法；`Ui::new_child` 不继承裁剪，所以正文一律进滚动容器，不手工推进光标。
- **Esc**：`hotkeys::classify` 加 `Key::Escape → Action::ClearInput`；`app.rs::run` 里仅在 `state.mode() == Compact` 时执行 `state.clear_input()`（清源文 + 译文 + failure，不发请求、不退出、不收窗）。
- **设置按钮**：`Effects::open_settings = true` → `app.rs` 执行"切回全量（走 REQ-004 的瞬时切换）+ `state.tab = Tab::Settings`"。
- **动作行用正常尺寸、放不下就换行**（v0.5，Q-011 推翻 v0.4 的三层收缩）：停靠指示下线后这一行只剩 6 个控件（`全量`/`复制`/`设置`/`机翻`/`AI`/`置顶`），实现改成 `ui.horizontal_wrapped(...)` 且**不再动** `override_text_style`、`button_padding`、`item_spacing`——用户 2026-10-23 的否定原话是"悬浮窗也可以正常使用操作栏，没必要搞一个缩减版的操作栏，还不容易点到"。随之删除的是 `NARROW_WIDTH` 常量、`dock_label(edge, narrow)`（整个函数下线）、`topmost_label(narrow)` 的窄标签退化（文案固定 `置顶`，状态由 `selectable_label` 的高亮表达）。选"换行"而不是"继续压缩"的技术理由：无头布局测试跑的是 egui 内置字体（没有中文字形，量出的文字宽度必然偏小），P4 的 D-1 就是这类"断言全绿而按钮被裁出窗口"；`wrapped` 布局自带"永不超出可用宽度"，与字体度量无关，等于把这一整类缺陷从结构上关掉。代价是最窄窗口里动作行会占两行高，所以 REQ-001 的高度下限同步抬到 130px。
- **提示条**：复用 P4 的 `toasts(&ctx, state)`（`egui::Area` 浮层）。简洁窗里的偏移量取 `把手条 10 + 输入行 34 + 4 = 48px`，而不是原写的 12px——12px 正好压在把手条上，而 REQ-005 的硬要求是"位置不遮住输入行文字"。`toasts` 因此多收一个 `offset_y` 参数，全量窗仍给 12px。

**验证**：无头 `run_ui` 断言 6 个控件的矩形都存在、都落在窗口 rect 内且两两不相交（D-1 的回归锁，不相交同时证明把手没有吃掉按钮），外加一条"按钮高度不小于 `interact_size.y`（20px）"——这条量的不是中文宽度，因此不受内置字体没有 CJK 字形的影响，能直接证明字号没被再压回去；320px 窄窗断言动作行**换成了两行**而不是溢出（`actions` 面板高度 > 一行高度）；点"AI"→ `Effects::job` 非空且 `state.engine == Ai`；AI 未配置时该段禁用；长译文 `ScrollArea` 有滚动范围而短译文不凭空造出滚动条（D-4 的手法）；Esc → 源文与译文都清空且 `in_flight` 为 None；`Effects::open_settings` → 模式变 Full 且 tab 变 Settings；复制只置 `Effects::copy`、不碰真剪贴板；真机 W-2.2/2.11/2.15/2.16 + 新增 W-2.19（这一条现在专测"正常尺寸按钮真机点得中"，无头测试证不了文字真实宽度）。

## REQ-006 实现与验证

纯函数（放 `state.rs`，因为它要读 `config.translation`）：

```rust
/// 源语言为自动检测、且检出语言与目标语言相同时，翻转目标语言。
/// 中文（简/繁）→ 英语；其余任何语言 → 简体中文。显式源语言一律不翻（返回 None）。
pub fn auto_reverse_target(detected: Language, target: Language) -> Option<Language>
```

钩子插在 `state.rs::submit()` 里、`request_id += 1` 之前：

```
if config.translation.source.is_none()
   && let Some(d) = ice_core::detect(&self.source).filter(|d| d.confidence >= 0.3)
   && let Some(new_target) = auto_reverse_target(d.language, config.translation.target)
   → config.translation.target = new_target; remember_target(new_target); config_dirty = true;
     toast(Info, format!("检测到{}，已自动改为 {} → {}", ...));
```

置信度门槛 0.3 与机翻引擎内部一致（`machine.rs:62`），避免把"猜的"当成"检出的"。检测失败（`None`）不翻、不拦，按 P4 既有回退走。因为翻转发生在构造 `Job` 之前，`lang_pair()` 拿到的就是新语言对，机翻引擎的同语言拒绝分支（`machine.rs:160-164`）永远不会被触发。

显式源语言 == 目标语言时**不改任何语言**，保持 P4 行为：引擎层就地拦下 → `failure_of` 给可读中文原因、无动作按钮。

**文档变更（用户 2026-10-21 授权）**：`docs/01-requirements.md` FR-1.2/FR-1.4 验收文字、`docs/02-architecture.md` 错误表 `SameLanguage` 行（P4 已埋 "CHG-004 将改这条的上游行为" 的指针，本包兑现）、`docs/06-test-cases.md` TC-1.2-c 状态列、`docs/07-acceptance.md` 变更登记一行。

**验证**：`auto_reverse_target` 五例（ZhHans→En、ZhHant→En、En→ZhHans、Ja→ZhHans、Fr→ZhHans）+ `submit()` 四例（自动检测英译英 → 翻且 Job 用新语言对且 toast 存在；自动检测中译中 → 翻成 En；显式 En→En → 不翻且不发请求走引擎拒绝；纯数字检测失败 → 不翻且正常发请求）；真机 W-2.x 之外单列一行（用户原话复现路径：源文放英文，目标简体中文，自动检测）。

## REQ-007 实现与验证

纯函数 `dim_alpha(mode: UiMode, focused: bool, dim_enabled: bool) -> f32`：只有 `Compact && !focused && dim_enabled` 返回 `0.85`，其余一律 `1.0`。

窗口级实现：`initial_viewport` 加 `.with_transparent(true)`（创建期属性，运行时 `ViewportCommand::Transparent` 在 macOS 上不保证重建 surface）；`CentralPanel` 的 fill 用 `window_fill().gamma_multiply(alpha)` 画，其余内容不额外降透明——观感是"整块窗体连内容一起透 15%"，桌面能看见。

H3 不成立时的降级路径（写在这里以免实现时临时决定）：去掉 `with_transparent`，改为在根 `ui` 上 `set_opacity(0.85)`，观感变成"内容变淡、背景仍不透明"；这属可感知降级，按停止条件 2 记 `SPEC_DEVIATION` 并请用户确认。

设置页的开关 P4 已经有（`dim_when_unfocused`，默认 true）——本包只做接线，若设置页缺这一项则补上（`docs/03-ui-design.md` §1 与 K-08）。

**验证**：`dim_alpha` 四组组合（Compact×失焦×开 → 0.85；Compact×聚焦 → 1.0；Compact×失焦×关 → 1.0；Full×失焦×开 → 1.0）；`initial_viewport` 的 `transparent` 字段断言；真机 W-2.9（点其他应用观察 + 设置里关掉后恒 100%）。

## REQ-008 实现与验证

`system::clipboard::Watcher`（纯判定，impure 的 `get_text` 留在外面）：

```rust
pub struct Watcher { last: Option<String>, ours: Option<String> }
pub const CLIPBOARD_POLL: Duration = Duration::from_millis(700);
/// 返回 Some(text) 表示"该填进输入行并提交"
pub fn observe(&mut self, text: Option<String>, input_focused: bool) -> Option<String>
```

判定顺序（每一条都对应一个真实故障模式）：
1. `text` 为 `None`（读失败或剪贴板不是文本）→ 不填，静默（最多一条 debug 日志，不含内容）；
2. `text.trim().is_empty()` → 不填；
3. `Some(text) == self.last` → 内容没变，不填；
4. `Some(text) == self.ours` → 是应用自己写的（复制译文/复制对照），不填，否则复制译文会立刻拿译文再翻一次；
5. `input_focused` → 用户正在输入行里打字，不填（`last` 仍要更新，否则失焦后会突然灌进来一段旧内容）；
6. 以上都过 → 记 `last`，返回 `Some(text)`。

`note_written(text)` 由 `app.rs` 在执行 `Effects::copy` 后调用，把"我们刚写的"记进 `ours`。

挂点：`App::ui` 末尾，若 `config.ui.clipboard_watch && !state.hidden` → `ctx.request_repaint_after(CLIPBOARD_POLL)` 续期，并在 `App::logic` 里做一次 `Watcher::observe(poll(), state.focus_in_source)`；命中则填 `state.source` + `submit()` + `toast`。窗口隐藏时不续期（Q-005：不轮询、不自动弹窗）。

日志只记长度与哈希前 8 位，与 NFR-3 的密钥口径一致；证据里不出现剪贴板原文。默认 `clipboard_watch = false` 不动。

**验证**：`observe` 七例（读失败 / 空文本 / 未变 / 自己写的 / 焦点占用 / 新内容命中 / 焦点占用后失焦再命中同一段不重复填）；`app.rs` 接线一例（Watcher 命中后 `Effects::job` 非空且源文被填）；空闲 CPU 两次采样（开关各一次，30 秒 `ps -o cputime` 差值，判据 <1%）；真机 W-2.10（在别处复制一段英文，悬浮条自动出译文）。

## REQ-009 实现与验证

落盘触发点：`mode::capture` 已经把"几何变了"转成 `config_dirty`，但 `flush_config(false)` 只在有帧跑的时候被调用，而拖动/缩放结束后没有任何东西再请求重绘 → 脏标记可能一直挂着，强杀就丢。补一条：`capture()` 里几何发生变化时 `ctx.request_repaint_after(CONFIG_SAVE_INTERVAL)`，让 500ms 后必有一帧把盘落了；`DragSettler` 返回最终 rect 时（吸附完成）与形态切换完成后回读到真实几何的那一帧走 `flush_config(true)` 立即落盘。节流不变（拖动过程中不会每帧写文件）。

离屏夹回：`fit_into_area(rect: Rect, area: Rect) -> Rect`（纯函数）——宽高超过可用区时先夹到可用区大小，再把位置夹进 `[area.min, area.max - size]`，保证不产生负偏移、不把窗口推到屏幕外。启动路径（`main.rs` 读配置后）与 `system::screen::work_area` 拿到真实可用区后各跑一次；P1 已有的 `WindowRect::{within, centered_in}` 继续负责"整块都在屏外 → 回中心"。

**停靠态是上一条的例外**（v0.5，REQ-009 新增那条）：`docked_edge ∈ {Left, Right}` 时窗口按设计就有 `w − sliver` 在可用区之外（左停靠必然 x 为负），照上面的整窗判据每次启动都会把停靠好的悬浮条拉回屏幕中心、等于替用户解了停靠。判定改成"那 `dock_sliver_px` 的可见热区还在可用区内吗"：热区在 → 按 REQ-003 的启动落位重发一次 `dock(...)` 的坐标（顺带解决"唤出态被强杀后盘上存的是整窗可见的 x"）；热区也越界（外接屏拔掉）→ 才走 `fit_into_area`/回中心。

FR-5.3：休眠唤醒/插拔后第一帧若 `outer_rect` 已失效（`None` 或落到可用区外），按上一条夹回，并重发一次当前形态的几何命令（复用 P4 `reveal()` 的"逼 winit 同步 surface 尺寸"手法，避免贴图）。

配置写失败：沿用 P4 `flush_config` 的错误路径（保留脏标记 + 可读提示 + 不 panic + 不阻塞翻译）。

**验证**：`fit_into_area` 五例（正常在内 / 越左 / 越下 / 比可用区还宽 / 比可用区还高）；`Runtime` 级一例——跑两帧 `ui`、改变几何、断言 `tempfile` 里的 `config.toml` 在 500ms 后含新几何（P4 的窗口恢复测试同一手法）；`initial_viewport` 的离屏回中心一例（沿用 P1）；真机 W-2.8（停靠右边缘后 `kill -TERM` 重启仍在右边缘）+ 新增 W-2.20（插拔外接屏各一次）。

## REQ-010 实现与验证

走查表：`docs/06-test-cases.md` §3 现有 W-2.1~W-2.14（14 行）保留，W-2.15/2.16 由"未实现"转"本包验"，新增 W-2.17（边缘缩放）、W-2.18（**停靠态悬停唤出/自动收回/打字不收回**，v0.5 改写）、W-2.19（简洁窗 320px 窄宽换行不溢出）、W-2.20（插拔屏幕）、W-2.21（**切换形态后尺寸立刻是 460×130**，锁 D-6）；P4 预报四条的引言块改成"已在 CHG-004 闭环"。

文档回写清单（REQ-010 的 SHALL 逐条落地）：
- `docs/01-requirements.md`：FR-3.2（460×130 起、可缩放、下限 320×130、控件清单含引擎分段且不再压缩字号）、FR-3.5（v0.5：只认左/右边缘 + 3px 热区 + 悬停唤出）、FR-1.2/FR-1.4（自动反向）；FR-3.9（改瞬时切换）**已在 2026-10-22 需求变更时改完**，实现后只需核对文字与代码一致；
- `docs/02-architecture.md`：§2 的 `mode` 行补 `snap`/`dim_alpha`/`DragSettler` 并把状态从"部分"改为完成（**不补 `Morph`**，FR-3.9 已于 2026-10-22 改为瞬时切换，§3.3 的那条已在本轮改完）、`views::compact` 从"—"变实体、`system::clipboard` 补 `Watcher`、新增 `system::screen` 一行；§8 落地表补 P5；错误表 `SameLanguage` 行兑现 CHG-004 说明；
- `docs/03-ui-design.md` §3：ASCII 图重画（把手条 + 可缩放 + 引擎分段 + 停靠/置顶指示）、控件清单加 K-10（引擎分段）与 K-11（缩放手柄/下限）、K-06 补"未聚焦先取焦点"；§4 的形态切换段落**已在 2026-10-22 改成瞬时版**；
- `docs/05-dev-guide.md`：测试计数、目录结构（`views/compact.rs`、`system/screen.rs`）、§8 平台差异补 AppKit 探针三行、§9 常见故障补"egui 没有 work area API"与"改窗口尺寸前必须先改 `MinInnerSize`（`InnerSize` 是异步请求，被旧约束夹住时会静默忽略）"两条——后一条要写清本包按形态取下限、不用 `MinInnerSize(ZERO)` 那个"完全无约束"的逃生口（见 REQ-001）；
- `docs/06-test-cases.md`：TC-3.5-a~e 与 TC-1.2-c 状态列、§3 走查表扩到 20 行（W-2.14 的判据**已在 2026-10-22 改成瞬时版**）；
- `docs/07-acceptance.md`：P5 段落填走查结论与证据指针，变更登记至少四行（FR-3.2 可缩放与控件清单、FR-1.2/1.4 自动反向、**FR-3.9 改瞬时切换——2026-10-22 已登记**、FR-3.7 若降级为内容级 dim），P4 遗留①②两行标记为已闭环；
- `docs/traceability.md`：FR-1.2/1.3/1.4、FR-3.2/3.3/3.5/3.6/3.7/3.8/3.9、FR-5.3、NFR-2/5 的实现列与结果列，清掉所有"待 P5"。

证据规范沿用 P4 的四条硬规则（`RUST_LOG=ice_translator=debug,info`、`LC_ALL=en_US.UTF-8 pbcopy` + 回验、截图裁到只含应用窗口、引用写 `*-V-0NN.txt` 取编号最大者），另加一条本包特有的：**剪贴板即翻的证据只记长度与哈希前 8 位**，不出现剪贴板原文。

**验证**：`tools/check_traceability.py`（需求↔用例双向闭合）、`./sdd scan`、`! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates`、用户走查（V-009）。

## v0.6 实现增量（2026-10-27 走查修正，覆盖上文与 REQ-001~005 相关的旧段落）

上面按 v0.1~v0.5 写下的实现计划有四处已被这一轮真机走查推翻或补强，逐条登记。对应规格见 Q-013/Q-014 与 REQ-001/REQ-002/REQ-003/REQ-005 的 v0.6 文字。

1. **简洁窗带回原生标题栏（Q-013），顶部 10px 把手条整条删除**——REQ-002 那一节"三处把手 + 三个互不重叠显式矩形"的方案作废两处，只剩动作行行尾空白与译文尾部 8px（`TAIL_HEIGHT`）两块；`Panel::top("compact_handle")` 的构造、`rect(ctx,"handle_bar")` 的记录与相应断言一起下线，`views::compact` 的第一带变成输入行（回归锁 `the_input_row_is_the_first_band_now_the_handle_bar_is_gone`）。
   **连带的真问题是吸附的触发**：抓原生标题栏由 AppKit 自己搬窗口，应用侧一帧 `StartDrag` 都收不到，"把手按下即 arm"的那套在最高频的拖动方式下根本不跑。`DragSettler`（静止判定）保留不动，改的是**谁来 arm**：除了窗内两处把手按下，`app::observe_native_move` 在"连续 `STILL_BEFORE_DRAG` 帧几何静止之后出现一次位移"时也 arm，之后照旧由 `settle_drag` 在静止满 `DRAG_SETTLE` 后跑 `apply_snap` + 落盘。四道闸门防误判（误判的代价比漏判大——唤出到贴边位的那次移动若被当成用户拖动，刚弹出来又缩回去）：自己下发过位置用 `commanded` 记住目标点、到达或连续静止后吃掉这次跳变；`pending_geometry` 在宽限期内不 arm（形态切换那次跳变不是拖动）；全量窗与隐藏态不 arm。还有一条不写在需求里但必须做的：`DragSettler` 在 armed 而未静止时要续一帧 `request_repaint_after`，否则原生拖完之后事件循环没有别的理由再画，"静止满 150ms"永远等不到。
   半透明那条不受影响：`with_transparent(true)` 仍在创建期，`apply_dim` 只压底色 alpha，原生标题栏由系统画、不跟着变淡（REQ-007 的走查判据据此补一句）。

2. **输入行按内容长高到三行封顶（Q-014）**——取代"输入行定高 34px + 横向滚动"。机制是"上一帧量到的内容自然高度 → 这一帧给的高度"，夹在 `INPUT_HEIGHT`（34px，空态退回这个值而不是退到一行文字）与 `MAX_INPUT_ROWS`（3 行）之间。三条实测坑，缺一条这个钳制就不生效：`TextEdit::multiline` 默认 `desired_height_rows = 4`（不压回 1 行就永远缩不回去）；外层那条带必须 `auto_shrink([false,false])`（`[_, true]` 会让内层收缩、连带把带子压回 25px）；`ScrollArea` 的 `min_scrolled_size` 默认 64px 比三行还高，不显式 `min_scrolled_height(行高)` 时 `max_height` 直接被顶穿。行高公式用 `text_style_height(Body) + extra_text_line_spacing`，按前者单算会少算行距、第三行裁掉半行。

3. **吸附的边界从单屏可用区改成桌面并集（缺陷 D-8）**——上文 REQ-003 那句"`area` 来自 `screen::work_area(ctx, outer_rect)`"作废，停靠/吸附/唤出三条改用 `screen::desktop()`（所有屏可用区的并集，`union` + `flip`）。双屏并排放时按单屏算会把两块屏相接的那条**接缝**当成可停靠边：窗口藏进接缝后，3px 热区既盖不住另一块屏、悬停也回不来，`released` 沿停靠轴量不出向内位移，于是永远解除不了停靠。`work_area`（单屏、去掉菜单栏与 Dock）只留给 REQ-009 的离屏夹回 `rescue_offscreen`——那条问的是"这块屏上还能不能看见窗口"，语义本来就按屏。接缝这条由 `screen::tests::the_desktop_union_spans_both_displays_and_skips_the_seam` 与 `mode::tests::a_display_seam_is_not_a_dockable_edge` 分两层锁住。

4. **修 D-9：长译文滚到底，最后一行看不见**——两处独立成因、两处独立修法，都归到"框的界限搞错了"这句用户判断上。
   - 简洁窗：共享构造 `full::scroll_area()` 没压 `min_scrolled_size`（egui 默认 64px），矮栏的视口被抬到 64px 并画到框外——实测 130px 高的窗里视口底边 115.9 越过译文框底边 103.0、压在动作行顶边 108.0 之上，而最大偏移按这个虚高视口算，最后十几像素怎么滚都进不来。修法就是那条构造里加 `.min_scrolled_height(0.0)`；"滚不动"的风险不靠这条默认值兜，由调用方的栏高守卫（`MIN_BODY_HEIGHT` 20px）负责。回归锁 `the_translation_viewport_never_grows_past_its_own_pane`（视口底边 ≤ 译文框底边、≤ 动作行顶边，且偏移能走到 `content − viewport`）。
   - 全量窗：浮层动作条不参与布局、直接盖在滚动区之上，正文末尾没给它留位置。修法是在正文末尾 `ui.add_space(bar_cover(inner.width(), actions.len()))`——预留量与画条子用同一条公式（`bar_grid`），所以窄栏换行成两行时留得也更多。回归锁 `the_last_line_stops_above_the_action_bar`（滚到底后最后一行底边 ≤ 条子上沿，且预留量 ≤ 条子高度 +1px，防止改成"少给正文高度"那种会撑出假滚动余量的做法）。
   - 方法论：这两条都是**断言全绿而看不见**的老坑（与 P4 的 D-1 同源），差别只在 D-1 是矩形被挤出面板、这次是视口比矩形高。补进 `docs/05-dev-guide.md` 的布局测试规范：**测滚动容器必须断言"可视区不越出给它的矩形"，只断言"偏移会增长"测不出越界**。

## v0.7 实现增量（2026-10-27 走查第二轮，覆盖上文 REQ-001/REQ-005 与 v0.6 那四段的相关部分）

对应规格见 Q-015~Q-018 与 REQ-001/REQ-004/REQ-005 的 v0.7 文字、新增的 REQ-011~REQ-013。下面第 0 条是缺陷修法，1~3 条是三颗新 REQ。

### 0. D-10：宽限期护栏的兜底方向反了（REQ-001 / REQ-004）

`PendingGeometry`（`mode.rs:420-450`）现在只有一个状态字段 `target: Option<(Vec2, Instant)>`，`size_axis_writable` 的三种放行里"宽限期已过"那条**无条件写 `self.target = None` 并返回 true**——于是切换没落定时，第一个被量到的实测尺寸（属于另一个形态）就被当作"用户选的尺寸"永久写进本槽。更糟的是它不可自愈：两槽一旦相等，下一帧 `measured == target` 恒成立，`landed` 分支每帧命中，护栏形同不存在（盘上 `full_rect` 与 `compact_rect` 逐项一字不差就是这个锁死的形状）。

改法是把护栏从"一个可选目标"变成"三段状态机 + 一个基线"：

```rust
pub struct PendingGeometry {
    stage: Option<Stage>,        // 原 target 挪进来
    baseline: Option<Vec2>,      // 第二段到期时量到的实测尺寸：它成为"不动"的参照
    retry: Option<Vec2>,         // 等调用方取走的重发目标（第一段到期那一帧立起）
}
struct Stage { target: Vec2, since: Instant, resent: bool }
```

- **第一段（不变）**：`|measured − target| ≤ PENDING_SIZE_TOLERANCE` → 落定，清状态，尺寸轴照常写回。
- **第一段到期未命中**：`stage.resent = true`、`since` 重置，并把目标尺寸立进 `retry`；**由调用方重发一组** `InnerSize` + `OuterPosition`（`mode::resend_commands`）。重发不能由 `PendingGeometry` 自己发命令（它拿不到 `Context`，也不该拿），所以 `capture` 那条路径要能回报"这一帧需要重发"：新增 `fn take_retry(&mut self) -> Option<Vec2>`（取走即清空，一轮只给一次），`app::refresh_geometry` 据此发那两条。**只重发尺寸与位置**：`Decorations`/`Resizable`/`MinInnerSize` 在切换那一组里已经生效、也不随尺寸变。REQ-004 那条"各恰好 1 条"的断言因此改成"正常落定各 1 条、模拟未落定至多 2 条且第二组出现在宽限期之后"（`a_first_stage_expiry_resends_the_size_and_position_once` 数的是**当帧新发**的命令——`feed_geometry` 走 `run_ui`，会像原生层那样把上一帧的命令取走）。
- **第二段到期仍未命中**：**不写实测尺寸**。把这一帧的 `measured` 存进 `baseline` 并记一条 WARN（`tracing::warn!` 只打两个槽的宽高与目标宽高，不含路径与任何内容，NFR-3），清掉 `stage`。此后 `size_axis_writable` 的判据变成"与 `baseline` 的差是否超过容差"：没超就返回 false（窗口停在哪儿都不写盘），超了说明是**用户自己在拉窗口**，返回 true 并清 `baseline`。这就是 REQ-001 里"把尺寸轴的所有权交还给用户"那句的机制——不是"到期后放行"，而是"到期后把当前值当成新的静默起点"。
- 新的 `set()`（下一次形态切换）无条件清 `baseline`。

**已污染的那一对槽不改写**（Q-015 ③）：程序没有依据猜 984×632 里哪个是本该属于谁的，自动改值等于偷偷修改用户配置。自愈由用户按设置页那颗「恢复默认窗口尺寸」显式完成（REQ-011 落点②），本包也不写迁移逻辑、不 bump 配置版本。

无头断言三段（都喂假的 `Instant`，不需要真等一秒）：① 第一段到期时 `take_retry` 给出目标尺寸、`size_axis_writable` 对另一形态的尺寸返回 false、槽的宽高未动；② 第二段到期后连续再喂若干帧同样的实测尺寸，断言槽的宽高**保持切换前那个旧值**、位置轴照常更新、`baseline` 生效期间没有任何写盘；③ 喂一个与 `baseline` 相差 > 容差的尺寸，断言它被写回（用户手动缩放不被吞）。加上回归锁 `the_two_slots_never_end_up_equal_after_a_failed_switch`：模拟一次"始终没落定"的切换，断言结束后 `full_rect` 与 `compact_rect` 仍然不等。

### REQ-011 实现与验证

两颗按钮 + 一条说明 + 一颗恢复默认，都接现成的路径，不动 FR-5.2 的任何行为。视图只置意图，落盘与关窗全在 `app.rs`。

- **简洁窗动作行**：`views/compact.rs::action_row` 的控件列表尾部加 `退出`（与 `全量`/`复制`/`设置`/`机翻|AI`/`置顶` 同字号同内边距，Q-011 那条"不许压缩控件"继续适用；窄宽下靠 `horizontal_wrapped` 换行，REQ-005 的换行断言自动覆盖它）。点击 → `effects.quit = true`，`app::run_effects` 见到这个标志就调 `self.run(Action::Quit, ..)`，与快捷键共用同一条路径，不新开第二种退出语义。`退出` 的矩形与其余按钮一样 `record` 进临时数据（`btn_quit`），无头点击才点得中。
- **全量窗设置页**：`views/settings.rs::window_group` 新加一个「窗口」分组（放在 AI 服务组之后），里面两行：`恢复默认窗口尺寸` + `退出程序`，各自旁边一行 `theme::MUTED` 说明——前者"两种形态各回到默认尺寸并居中（窗口被历史版本写坏过时用）"，后者"红灯 / Cmd+W 是收进后台，不是退出"。两颗都只在这一页出现，不进其它页签。
- **「恢复默认窗口尺寸」的落点**：`effects.reset_geometry = true` → `app::reset_window_sizes`。它**不走 `mode::apply`**：`apply` 开头先 `capture`，会把正要被覆盖的坏尺寸再记回槽里一次。改成直接下发 `mode::commands_for_mode` + 按 `pending` 的规矩登记目标尺寸（`pending_geometry.set`），落定后由每帧的 `capture` 自然写回。位置由纯函数 `app::restored_rect(default, keep, monitor)` 决定：拿得到可用区就 `WindowRect::centered_in`，拿不到（无头、探针失败）就**只换尺寸、位置留在原处**——宁可不挪，也不能把窗口甩到屏外。停靠一并清掉（`docked_edge = None` + `dock_anchor = None`），因为收位是按旧尺寸算的，留着旧锚点下一帧会把窗口顶出屏幕。之后 `mark_config_dirty` + `flush_config(true)` 立即落盘、`apply_topmost`、一条提示、`request_repaint`。两颗按钮分工要说清：前者管"退干净"，后者管"尺寸被历史版本写坏了"。
- 路径复用：`app.rs` 里 `Action::Quit` 已经做对三件事（`capture` → 强制落盘 → `mode::quit` 置 `quitting` 再发 `Close`），按钮只是多一个入口，SHALL NOT 出现第二条退出实现。
- 验证：`the_quit_button_is_in_the_action_row`（点简洁窗那颗 → `effects.quit`）、`the_window_group_buttons_set_their_intent`（点设置页两颗 → 各自那个标志，且互不串味）、`the_window_group_buttons_survive_the_narrowest_full_window`（640×420 下两颗都在窗内——REQ-011 的动机就是"看不见等于没有"）、`the_reset_button_restores_both_slots_and_flushes_them`（两个槽回默认、停靠清掉、当帧落盘、目标尺寸登记进护栏）、`the_restored_size_is_centered_when_the_area_is_known`（居中与"拿不到可用区就留在原处"两条分支）。"按了到底有没有退出"必须真机（W-2.26，配 `pgrep` 与 Dock 图标核对，与 W-2.12 那条"收起不退出"成对）。

### REQ-012 实现与验证

简洁窗版式的三条带从"输入行按内容长高 + 译文吃剩余"改成"两带 1:1 分摊 + 带间 ≥5px + 各自成框"，并把下限抬高。

- **先让 egui 扣，再算分摊**：`show` 里 `action_row` 是 `Panel::bottom`，**先建**；两条带画在同一个 `CentralPanel` 里（`inner_margin(0.0)`），于是 `ui.available_height()` 拿到的就是"窗口内高 − 动作行自然高"那块。上一版为此准备的两件东西整个下线：跨帧量动作行高度的 `measured_bar_height` 与首帧兜底常量 `FIRST_FRAME_BAR`——动作行高度不再需要被"量"，也就没有"首帧猜错"那一帧延迟。
- **纯函数**：`pub fn band_heights(available_h: f32) -> (f32, f32)`（一个参数，就是上面那块可用高），`BAND_GAP: f32 = 5.0`。`room = available_h − 2*BAND_GAP − 4*BAND_OUTER_PAD`，两带各取 `room/2`；`room` 小到撑不起"输入带 ≥ `INPUT_HEIGHT`(34) 且译文带 ≥ `MIN_BODY_HEIGHT`(20)"时**优先保译文带**、输入带退回 `INPUT_HEIGHT`（REQ-012 明写的退让）。`BAND_OUTER_PAD = BAND_V_PAD + 1.0 = 5.0` 是实测出来的：一条带的外框比内容高 10（内边距 4×2 + 描边 1×2），按 8 扣就会两带一起压到动作行上（320×160 下译文带底边 142 vs 动作行顶边 135）。两个 `BAND_GAP` 一个是画在两带之间的实间隔，一个留在译文带与动作行之间当余气。
- **`MAX_INPUT_ROWS` 整条下线**（Q-017 推翻 Q-014 的高度上限）：`source_height` 那套"上一帧的自然高度 → 这一帧的高度"钳制删除，输入框高度 = 分到的带高；`desired_rows(1)`、外层 `auto_shrink([false,false])`、`min_scrolled_height(当帧行高)` 三条实现前提保留（它们解决的是"默认 4 行"与"64px 视口"两个坑，与上限无关）。`the_input_height_is_clamped_between_the_empty_row_and_three_rows` 那组断言改写成 `the_two_bands_split_equally_and_fall_back_to_one_input_line`（纯函数逐档）+ `the_input_band_keeps_its_share_when_the_source_gets_long`（320 与 460 两种宽度下两带等高，内容长高不改带高）+ `a_source_beyond_the_cap_scrolls_inside_the_input_box`。
- **两个圆角框**：两带各包一层 `egui::Frame::group`（与全量窗源文栏同一套），带间 `BAND_GAP` 由 `add_space` 保证。断言读的是**外框**矩形（`input_band` / `translation_band`），不是内容矩形——用户看到的间隔是框与框之间的距离。译文尾部那 8px 把手（`TAIL_HEIGHT`）画在框内空白处，`MIN_BODY_HEIGHT` 那条守卫在正文不够高时直接跳过把手并 `advance_cursor_after_rect`。
- **一条 egui 0.37 的坑（与 REQ-013 同一处）**：`Ui::allocate_ui(size, ..)` 分配的是**子级自己的 `min_rect`**，不是请求的那个尺寸；内容全画在 `new_child` 或 `Ui::interact`（把手）里的话，父级一条都没分配，带子会塌成"一个按钮那么高"（实测 26×10）。修法统一是画完 `ui.advance_cursor_after_rect(rect)`。
- **数值联动**：`MIN_COMPACT_H` 130 → **160**、`compact_rect` 默认 → `460×160`，唯一出处仍是 `config.rs` 那四个常量，`initial_viewport` / `commands_for_mode` / `sanitize` / `clamp_compact_rect` 四处自动跟着走；`03-ui-design.md` §3 的 ASCII 图与数值同批改。
- 验证：`band_heights` 逐档单测（够分 1:1、撑不起时输入带退到一行、付不起不给负数、极窄档求和不超过可用高）；`the_bands_are_separated_by_the_gap_and_never_overlap` 断相邻两带间距 ≥5px 且互不重叠、且不越出窗口；下限抬高由命令序列断言 + `an_undersized_configured_rect_is_clamped_before_it_reaches_the_window` 的 130→160 期望值改动 + 真机一次启动观测（简洁窗以 460×160 起）覆盖；观感（是否看得出两个框、间隔够不够）走查 W-2.27。

### REQ-013 实现与验证

全量窗那条动作条从"浮层盖正文"改成"自占一条底栏"，`bar_cover` 整套删除。

- **删除**：`bar_cover`、`floating_actions` 里 `ui.painter().rect_filled` 那类直接画到正文矩形之上的手法，以及"预留量 ≤ 条子高度 +1px"那条断言。`translation_pane` 里那句 `ui.add_space(bar_cover(..))` 一并删掉——正文末尾不再需要假空白。字数也从源文栏脚注搬到底栏左侧，源文栏那条脚注整条收掉。
- **新增**：`views/full.rs::bottom_bar` = `Panel::bottom("full_bottom_bar").frame(Frame::NONE.inner_margin(BAR_MARGIN))`，**在两栏之前创建**，所以两栏拿到的可用高度天生少了它；`pane_body_height` 因此只扣行距与译文栏那条定高徽标行（`PANE_FOOTER_HEIGHT`），不扣底栏。左侧一格 `CHAR_SLOT`(160px) 放 `char_count_text`（`Label::truncate`，可省略号），右侧 `right_to_left` 按 `actions.iter().rev()` 放按钮组（沿用同一套 `BarAction`，数量与语义不变，`Cmd+Shift+C` 那条路径不动），视觉顺序仍是「翻译」在最左。
- **分栏不能用 `Ui::columns`**：它给每列的矩形用的是 `max_rect` 的底边、**不看光标**，刚从光标矩形里扣掉的底栏它看不见（实测正文底边压过条子上沿 5px）。改成 `ui.horizontal_top` + 两个 `allocate_ui_with_layout(top_down)`，列宽按 `available_width` 现算。同一条坑也出现在底栏自己身上：左右两块都画在丢弃的子 `Ui` 里，面板自己不推进光标就会塌回一个按钮的高度，所以结尾同样 `advance_cursor_after_rect(rect)`。
- **自然高与退让顺序**（用户那句"如果不行就把按钮调整小一点"）：`bar_layout(button_area_w, count)` 是"扣掉字数那一格之后"的排布判据，返回 `(按钮尺寸, 行数)`——标准档 `BUTTON_SIZE`(84×28) 一排排得下就用它，排不下先降 `BUTTON_SIZE_NARROW`(64×28)，仍排不下才换行；`bar_height(size, rows)` 与 `bottom_bar` 用**同一个** `bar_layout` 的结果，所以"扣多少"与"画多高"是同一个数（上一版正是这两处各算各的才留出空缝）。`bar_rows` 把可用宽按下限夹一次，保证"至少一颗一排"，永远不会算出 0 行。字数可截断，按钮不行。
- 验证：`the_bottom_bar_occupies_its_own_row_below_both_panes`（底栏在两栏之下、各自在窗内、两栏正文 >0）、`the_bar_shrinks_before_it_wraps`（先降档再换行）、`a_narrow_window_wraps_the_bar_instead_of_overflowing`、`the_last_line_stops_above_the_bottom_bar`（判据从"≤ 浮层上沿"改成"≤ 底栏上沿"，容差里含译文栏那条徽标行 + 行距 + 底栏上内边距，实测 36px）、`both_panes_get_the_same_box`（两栏等高的既有判据按新结构重算，字数不留两份）；观感与"缩小时是不是先缩按钮"走查 W-2.28。
