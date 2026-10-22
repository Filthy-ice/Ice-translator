Change: CHG-004
Version: v0.3
Spec-Hash: 235deaf136a3d2a0f9d8585b72f05f2b8948470f70d9e26629c5f5872091fa9c

# CHG-004 · Design

- Change: CHG-004 · v0.1 · `spec_hash 0a74661dd24107ebe2a576aacde34be301ddfcaa457fd8a54484689f3bd057e2`
- Repo: `ice-translator`（分支 `main`）
- 前置：CHG-003 已 ACCEPTED（`f123a8b`），代码态 246 passed / 0 failed / 3 ignored

## 实现方案

### 模块划分

| 文件 | 动作 | 内容 |
|---|---|---|
| `crates/ice-app/src/mode.rs` | 扩 | `MIN_COMPACT_SIZE`、`commands_for_mode` 改（按形态给最小尺寸 + 两种形态都允许缩放 + 全量也下发 `OuterPosition`，即 REQ-004 的瞬时切换）、`snap`/`cycle_edge`/`fit_into_area`/`dim_alpha` 四个纯函数、`DragSettler` 拖动/缩放静止判定 |
| `crates/ice-app/src/views/compact.rs` | **新** | 简洁窗版式：把手条、输入行、译文定高 `ScrollArea`、动作行（全量 / 复制 / 设置 / 机翻\|AI / 停靠指示 / 置顶）。P4 暂由 `app.rs::compact` 私有函数代管，本包迁出（`docs/02-architecture.md` §2 的 `views::compact` 从"—"变实体） |
| `crates/ice-app/src/views/mod.rs` | 改 | `engine_picker` 提为 `pub(crate)` 供简洁窗复用；`Effects` 加 `start_drag`、`open_settings` 两个字段 |
| `crates/ice-app/src/system/screen.rs` | **新** | 屏幕可用区探针：macOS 走 `NSScreen.visibleFrame`（`cfg(target_os = "macos")` + unsafe），其余平台回退 `viewport().monitor_size`；坐标翻转是纯函数，可无头单测 |
| `crates/ice-app/src/system/clipboard.rs` | 扩 | `Watcher`：纯判定（新内容 / 自己写的 / 焦点占用 / 空文本 → 该不该填）+ 一个薄 `poll()`（impure，只调 `arboard::get_text`） |
| `crates/ice-app/src/system/hotkeys.rs` | 扩 | `Key::Escape → Action::ClearInput`（只在简洁模式生效，全量模式 Esc 仍不绑定任何动作，与 P4 一致） |
| `crates/ice-app/src/state.rs` | 扩 | `submit()` 里插自动反向钩子；`auto_reverse_target` 纯函数；`clear_input()` |
| `crates/ice-app/src/app.rs` | 改 | 删掉私有 `compact()`、接 `views::compact`；切换后回读几何并立即落盘（REQ-004）；拖动/缩放静止后的吸附与落盘；剪贴板轮询挂点；失焦 alpha；`initial_viewport` 加 `with_transparent(true)` |

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

`commands_for_mode` 的 Compact 分支改为：

```rust
UiMode::Compact => vec![
    ViewportCommand::Decorations(false),
    ViewportCommand::MinInnerSize(Vec2::ZERO),      // == set_min_inner_size(None)，解掉全量留下的 640×420
    ViewportCommand::MaxInnerSize(Vec2::INFINITY),  // == set_max_inner_size(None)
    ViewportCommand::Resizable(true),               // 用户 2026-10-22 决定：简洁窗要能鼠标缩放
    ViewportCommand::InnerSize(clamped.w/h),        // 约束解除之后才设尺寸
    ViewportCommand::OuterPosition(rect.x/y),
]
```

顺序是正确性的一部分：`InnerSize` 走 `window.request_inner_size`，平台不允许时直接返回当前尺寸并忽略请求（`egui-winit-0.37.0/src/lib.rs:1969-1985`），所以"解除约束"必须排在"设尺寸"之前。Full 分支保持 `MinInnerSize(640×420)`，恢复约束。

`MIN_COMPACT_SIZE = Vec2::new(320.0, 96.0)`；`clamp_compact_rect(WindowRect) -> WindowRect` 把小于下限的宽高抬到下限（配置被手改坏也不拒绝启动）。`ice-core::config::sanitize()` 同步夹这两个下限（REQ-001 边界），单测覆盖"手改配置成 100×40"。

版式：`views/compact.rs` 用 `Panel::top`(输入行 34px) + `Panel::bottom`(动作行 28px) + `CentralPanel`(译文区)，译文区吃剩余高度——窗口被拉大时多出来的高度全给译文区（REQ-001 的"缩放到任意合法尺寸三块都不被裁"）。

**验证**：`commands_for_mode` 的命令集合与顺序（`MinInnerSize(ZERO)` 的位置必须在 `InnerSize` 之前，用 `.position()` 断言，沿用 P4 `switching_to_compact_queues_a_borderless_small_window` 的手法）；`clamp_compact_rect` 三例（正常/过窄/过矮）；`sanitize` 往返一例；真机 W-2.1 + 新增 W-2.17（拉边缘缩放）。

## REQ-002 实现与验证

把手三处，全部走 `ViewportCommand::StartDrag`（原生拖拽，不自绘坐标）：

1. **顶部 10px 把手条**：`Panel::top("compact_handle").exact_size(10.0)`，`ui.set_cursor_icon(CursorIcon::Grab)`；用 `ui.interact(ui.max_rect(), Id, Sense::click_and_drag())`，`response.drag_started()` 时发命令。
2. **动作行空白**：动作行 `ui.horizontal` 结束后对剩余空白 `interact(..., Sense::click_and_drag())`。
3. **译文区尾部空白**：译文 `ScrollArea` 内文本右侧的剩余区域同样处理（文本本身要可选中，所以把手是"文本 rect 之外"的那块，用 `ui.available_rect_before_wrap()` 减去文本 rect 得到）。

未聚焦时第一次按下会被吞：`StartDrag` 的实现被 `window.has_focus()` 挡着（`egui-winit-0.37.0/src/lib.rs:1961-1968`，注释说 x11 上不判焦点会永久吃掉输入）。处置：把手 `drag_started()` 时若 `ctx.input(|i| i.viewport().focused) != Some(true)`，先发 `ViewportCommand::Focus`（同帧命令按序执行，`Focus` 在 `StartDrag` 之前入队），真机验证"一次按下就能拖"。若实测仍要两下，走停止条件 4。

不触发拖动的控件：输入框（`TextEdit` 自己吃掉事件）、四个按钮、引擎分段、译文文本、滚动条——实现上把手只覆盖"空白区域"，绝不与控件 rect 重叠；单测用无头 `run_ui` 在不同指针位置断言 `Effects::start_drag` 有无被置位（沿用 P4 的 Effects 断言手法）。

松手触发吸附：原生拖拽期间 egui 收不到 pointer 事件，所以用**静止判定**——`DragSettler::feed(outer_rect, now) -> Option<Rect>`：`start_drag` 发出后置 `armed`；此后每帧比较 rect，变化就刷新 `last_move`，连续 `DRAG_SETTLE = 150ms` 不变则返回最终 rect（交 REQ-003 吸附 + REQ-009 落盘），返回后解除 `armed`。纯函数，单测喂一串带时间戳的 rect 序列（移动中 → 停 150ms → 返回一次且只返回一次）。

**验证**：把手命中/不命中（4 组指针位置）、`StartDrag` 入队、未聚焦时先入队 `Focus`、`DragSettler` 五例（未 armed 不返回、移动中不返回、静止不足不返回、恰好静止返回一次、再次移动重新计时）；真机 W-2.13（三处把手各拖一次，且拖输入框/按钮不移动窗口）。

## REQ-003 实现与验证

纯函数 `snap(window: Rect, area: Rect, threshold: f32, sliver: f32) -> (Rect, DockEdge)`：

```
d_left   = window.min.x - area.min.x
d_right  = area.max.x - window.max.x
d_top    = window.min.y - area.min.y
d_bottom = area.max.y - window.max.y
取最小的那条边与其距离 d；d < threshold 才吸附：
  Right  → x = area.max.x - w + sliver
  Left   → x = area.min.x - w + sliver
  Top    → y = area.min.y - h + sliver
  Bottom → y = area.max.y - h + sliver
吸附后再跑一次 fit_into_area（见 REQ-009）保证不出负坐标、不出可视区（TC-3.5-e）
否则 (window, DockEdge::None)
```

与 `03-ui-design.md` §3 的伪代码逐行对应，`sliver`/`threshold` 从 `config.ui` 取（默认 6/40）。

`cycle_edge(DockEdge) -> DockEdge`：`Right → Bottom → Left → Top → Right`，`None → Right`。停靠指示是个 `selectable_label`，文案"停靠：右"（宽度不足时退化成"右"，REQ-005 的窄宽规则），点击后 `cycle_edge` + 立刻按新边重算 rect 并发 `OuterPosition` + 落盘。

`area` 来自 `system::screen::work_area(ctx, outer_rect)`（见"Spec 事实更正"）；探针失败时回退 `monitor_size`，并把这次回退记进日志（`warn_once` 语义，避免每帧刷屏）。

**验证**：TC-3.5-a~e 五例纯函数单测（右 30px / 左上右下各 30px / 四边都 >40px / 停靠后拖离 200px / 窗口高于可用区）+ `cycle_edge` 五例 + 坐标翻转纯函数四例（主屏、副屏在主屏右侧、副屏在主屏下方、窗口不在任何屏内）；真机 W-2.5/2.6 + 新增 W-2.18（指示循环一圈）。

## REQ-004 实现与验证

机制是**瞬时切换**（2026-10-22 需求变更定案）。逐帧 `InnerSize` 补间与"双向窗内补间"都已被真机实测否掉，单发几何命令实测 **20ms 落定**（见"三条 HYPOTHESIS 的最小实测"与 `evidence/T-001-probe-v3.log`）。因此：**没有 `Morph`、没有插值、没有逐帧重绘**，切换就是在同一帧里把目标形态的命令一次性排进 `Effects`。

落点是 `mode.rs` 已有的 `commands_for_mode`（P4 就在用，本包改它的内容而不是新增状态机）：

```rust
pub fn commands_for_mode(cfg: &UiConfig, mode: UiMode) -> Vec<ViewportCommand> {
    let rect = rect_for(cfg, mode);
    let (decorations, min_size) = match mode {
        UiMode::Full => (true, MIN_FULL_SIZE),
        UiMode::Compact => (false, MIN_COMPACT_SIZE),   // REQ-001：简洁窗也要能缩放，下限 320×96
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

1. **`Resizable(true)` 与 `MinInnerSize` 对简洁形态也下发**（P4 给的是 `Resizable(false)`），这是 REQ-001；`MinInnerSize` 从"清成 0"改成按形态取值，否则切回全量后简洁窗的 320×96 下限会跟着带过去。
2. **全量形态也下发 `OuterPosition`**（P4 只给简洁形态下发）。不下发时，从贴边停靠的悬浮条切回全量，窗口会留在屏幕边缘那条位置上，与 FR-3.6"记住位置尺寸"矛盾。`full_rect` 存的是外框矩形（P4 已修掉"把标题栏高度算进位置"那个缺陷），所以这里直接用它的 `x/y` 就是对的原点。

顺序有意义：`Decorations` 先于 `InnerSize`。反过来会有一帧按错误的可用高度绘制（装饰回来后内容区矮一截），观感就是"内容闪一下"，正是 REQ-004 禁止的。

驱动在 `app.rs`：切换时 `Effects` 里排这 5 条命令 + `config.mode = 目标形态` + `config_dirty = true`，**不调用 `ctx.request_repaint()`**（切换本身就在一次重绘里发生，之后回到"只有提示条到期与结果到达才重绘"的空闲策略，NFR-2）。下一帧 `mode::capture` 回读真实几何写回 `full_rect`/`compact_rect`，再 `flush_config(true)` 立即落盘——因为命令是**异步请求**（`InnerSize` 走 winit `request_inner_size`），不能把目标值当已生效值写盘；也因为 SIGTERM/强杀不走 `on_exit`。

隐藏态（`state.hidden`，eframe 只跑 `logic` 不跑 `ui`）：切换只改配置里的目标形态，不下发命令、不把窗口叫回来；`reveal()` 时按目标形态一次性设好几何（沿用 P4 的路径）。

不再需要的东西（v0.2 设计里写过，随需求取消一并删掉）：`Morph`/`MorphFrame`/`GeometryCmd`/`union()`/`reverse()`/`MORPH_DURATION`、`state.morphing` 标志、内容矩形插值与 `Ui::new_child(max_rect = …)` 的透明余量画法、`ui.multiply_opacity` 的淡入淡出、objc2 `setHasShadow(false)` 的备用手段（那是为了压外接矩形的阴影），以及 Q-008 记的两条代价（透明区吞点击、阴影跟随大矩形）——瞬时切换下窗口矩形恒等于可见矩形，两条都不存在。

**验证**（全部无头断言，`ctx.viewport(|vp| vp.commands.clone())` 取命令队列）：一次切换里 `InnerSize` 与 `OuterPosition` **各恰好 1 条**、`Decorations` 1 条且排在 `InnerSize` 之前（防止实现悄悄退回逐帧补间）；切换后 `Effects::job` 为空且 `state.in_flight` 未置位（不重发请求）；内容与源/目标语言、引擎选择原样保留（P4 那两条测试必须仍绿）；隐藏态触发切换时命令队列为空、只有配置变化；`capture` 回读的 rect 与目标 rect 一致（±2px）；切换到全量时 `full_rect` 的位置不被标题栏高度污染。真机 W-2.14：连续切 10 次，盯"一步到位、Dock 图标不闪、无黑屏白屏、切完立刻能点"。

## REQ-005 实现与验证

`views/compact.rs` 的版式（460×96 默认，最小 320×96）：

```
┌ 把手条 10px（Grab 光标，拖动）─────────────────────────────┐
│ 输入行 34px：TextEdit::singleline，Enter 提交              │
│ 译文区 剩余高度：ScrollArea::vertical().auto_shrink([false,false])│
│   文本可选中复制；尾部空白是第三处把手                      │
│ 动作行 28px：全量 · 复制 · 设置 · 机翻|AI · 停靠：右 · 置顶 │
└────────────────────────────────────────────────────────────┘
```

- **引擎分段**：直接复用 `views::mod::engine_picker`（提为 `pub(crate)`），它就是 `Button::selectable` 的分段样式，AI 未配置时禁用并带说明——简洁窗不新造一套。
- **译文滚动**：定高 `ScrollArea`（`id_salt("compact_translation")` + `auto_shrink([false,false])`），沿用 P4 缺陷 D-4 的手法；`Ui::new_child` 不继承裁剪，所以正文一律进滚动容器，不手工推进光标。
- **Esc**：`hotkeys::classify` 加 `Key::Escape → Action::ClearInput`；`app.rs::run` 里仅在 `state.mode() == Compact` 时执行 `state.clear_input()`（清源文 + 译文 + failure，不发请求、不退出、不收窗）。
- **设置按钮**：`Effects::open_settings = true` → `app.rs` 执行"切回全量（走 REQ-004 的瞬时切换）+ `state.tab = Tab::Settings`"。
- **提示条**：复用 P4 的 `toasts(&ctx, state)`（`egui::Area` 浮层），简洁窗里定位到窗口顶部以下 12px，不遮输入行文字。
- **窄宽不溢出**：动作行用 `ui.available_width()` 判断，宽度 < 380px 时停靠指示退化为单字、置顶退化为"顶"；按钮本身不缩（可点性优先）。这是 P4 缺陷 D-1 的同类回归，必须有断言。

**验证**：无头 `run_ui` 断言六个控件都存在且 `min_rect` 不越出 320×96 的窗口 rect（D-1 的回归锁）；点"AI"→ `Effects::job` 非空且 `state.engine == Ai`；滚轮 8 帧后译文 `ScrollArea` 偏移增长（D-4 的手法）；Esc → 源文与译文都清空且 `in_flight` 为 None；`Effects::open_settings` → 模式变 Full 且 tab 变 Settings；真机 W-2.2/2.11/2.15/2.16 + 新增 W-2.19。

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

FR-5.3：休眠唤醒/插拔后第一帧若 `outer_rect` 已失效（`None` 或落到可用区外），按上一条夹回，并重发一次当前形态的几何命令（复用 P4 `reveal()` 的"逼 winit 同步 surface 尺寸"手法，避免贴图）。

配置写失败：沿用 P4 `flush_config` 的错误路径（保留脏标记 + 可读提示 + 不 panic + 不阻塞翻译）。

**验证**：`fit_into_area` 五例（正常在内 / 越左 / 越下 / 比可用区还宽 / 比可用区还高）；`Runtime` 级一例——跑两帧 `ui`、改变几何、断言 `tempfile` 里的 `config.toml` 在 500ms 后含新几何（P4 的窗口恢复测试同一手法）；`initial_viewport` 的离屏回中心一例（沿用 P1）；真机 W-2.8（停靠右边缘后 `kill -TERM` 重启仍在右边缘）+ 新增 W-2.20（插拔外接屏各一次）。

## REQ-010 实现与验证

走查表：`docs/06-test-cases.md` §3 现有 W-2.1~W-2.14（14 行）保留，W-2.15/2.16 由"未实现"转"本包验"，新增 W-2.17（边缘缩放）、W-2.18（停靠指示循环）、W-2.19（简洁窗 320px 窄宽不溢出）、W-2.20（插拔屏幕）；P4 预报四条的引言块改成"已在 CHG-004 闭环"。

文档回写清单（REQ-010 的 SHALL 逐条落地）：
- `docs/01-requirements.md`：FR-3.2（460×96 起、可缩放、下限 320×96、控件清单含引擎分段）、FR-1.2/FR-1.4（自动反向）；FR-3.9（改瞬时切换）**已在 2026-10-22 需求变更时改完**，实现后只需核对文字与代码一致；
- `docs/02-architecture.md`：§2 的 `mode` 行补 `snap`/`dim_alpha`/`DragSettler` 并把状态从"部分"改为完成（**不补 `Morph`**，FR-3.9 已于 2026-10-22 改为瞬时切换，§3.3 的那条已在本轮改完）、`views::compact` 从"—"变实体、`system::clipboard` 补 `Watcher`、新增 `system::screen` 一行；§8 落地表补 P5；错误表 `SameLanguage` 行兑现 CHG-004 说明；
- `docs/03-ui-design.md` §3：ASCII 图重画（把手条 + 可缩放 + 引擎分段 + 停靠/置顶指示）、控件清单加 K-10（引擎分段）与 K-11（缩放手柄/下限）、K-06 补"未聚焦先取焦点"；§4 的形态切换段落**已在 2026-10-22 改成瞬时版**；
- `docs/05-dev-guide.md`：测试计数、目录结构（`views/compact.rs`、`system/screen.rs`）、§8 平台差异补 AppKit 探针三行、§9 常见故障补"egui 没有 work area API"与"`MinInnerSize(ZERO)` 才是清约束"两条；
- `docs/06-test-cases.md`：TC-3.5-a~e 与 TC-1.2-c 状态列、§3 走查表扩到 20 行（W-2.14 的判据**已在 2026-10-22 改成瞬时版**）；
- `docs/07-acceptance.md`：P5 段落填走查结论与证据指针，变更登记至少四行（FR-3.2 可缩放与控件清单、FR-1.2/1.4 自动反向、**FR-3.9 改瞬时切换——2026-10-22 已登记**、FR-3.7 若降级为内容级 dim），P4 遗留①②两行标记为已闭环；
- `docs/traceability.md`：FR-1.2/1.3/1.4、FR-3.2/3.3/3.5/3.6/3.7/3.8/3.9、FR-5.3、NFR-2/5 的实现列与结果列，清掉所有"待 P5"。

证据规范沿用 P4 的四条硬规则（`RUST_LOG=ice_translator=debug,info`、`LC_ALL=en_US.UTF-8 pbcopy` + 回验、截图裁到只含应用窗口、引用写 `*-V-0NN.txt` 取编号最大者），另加一条本包特有的：**剪贴板即翻的证据只记长度与哈希前 8 位**，不出现剪贴板原文。

**验证**：`tools/check_traceability.py`（需求↔用例双向闭合）、`./sdd scan`、`! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates`、用户走查（V-009）。
