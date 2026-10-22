# CHG-004 · 简洁悬浮窗：可缩放无边框窗体、拖动与吸附、形变动画、窗内引擎切换、自动反向

## 原始意图与来源

- 立项要求（2026-10-19，用户原话）："写个 rust 跨平台的客户端软件。哦，别忘了做两个界面……优先针对 mac。" `docs/00-charter.md` 把 P5 定为"简洁界面 / 悬浮条模式"，验收判据写的是"置顶实测有效、四边吸附可用、切换不丢状态"。
- 用户在 P1 走查后追加并已进需求基线的两条：FR-3.2 的"**无边框不得导致窗口不可移动**"、FR-3.9 的"**形态切换是同一个窗口的形变**"。这两条从 P1 起就明确落在 P5，本包是它们的实现包。
- **P4 第四轮走查用户预报的四条缺陷**（2026-10-21 原话，已在 `docs/07-acceptance.md` P4 遗留与 `docs/06-test-cases.md` §3 引言登记）：
  1. "悬浮窗点击后还是和原框一样大，应该是一个很精致的窗口" → W-2.1，本包 REQ-001；
  2. "悬浮窗无法拖动" → W-2.13，本包 REQ-002；
  3. "悬浮窗内无法更换机翻还是 ai" → W-2.15，本包 REQ-005；
  4. "悬浮窗内的译文不能滚动" → W-2.16，本包 REQ-005。
  CHG-003 规格第 19/25 行已把"简洁窗内部版式、吸附、半透明、剪贴板即翻、形变动画"划给 P5，所以这四条不计入 P4 失败项，是本包的入场清单。
- **自动反向**（2026-10-21 用户原话）："原文中我放中文，可以翻译为英文；但是我放英文就无法智能翻译为中文，提示：源语言与目标语言相同（都是英语），不需要翻译；请换一个目标语言。" 用户 2026-10-21 决定"**自动反向并改目标语言**"、"推到 P5/CHG-004" → 本包 REQ-006，已登记为 TC-1.2-c。
- 前置状态：CHG-003（P4 全量界面）已 **ACCEPTED**（2026-10-22，`spec_hash 84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7`，提交 `555e1aa` + 签名 `f123a8b`）。代码态 246 passed / 0 failed / 3 ignored（ice-app 144 · ice-core 49 · ice-providers 53）。P4 唯一遗留证据缺口是 W-1.8 的"假 Key → 红字 + 去设置按钮 视觉同框"，推 P8，本包不做。
- 用户 2026-10-22 对本包范围的三项决定（本会话答复原文见 Q 表）：简洁窗要能鼠标拖动缩放；引擎切换做成动作行常驻分段小按钮；Must + Should 全做。

## 目标与非目标

**目标**：把简洁模式从 P4 的"能切过去、内容不丢"长成真正可用的悬浮条——窗口尺寸命令真正生效且可用鼠标缩放、无边框下三处把手都能拖动、四边吸附与拖离、形态切换是同一窗口的形变动画、窗内可切机翻/AI 且译文可滚、失焦半透明、剪贴板即翻、几何即时落盘与屏幕变化后夹回可视区；外加一处用户本轮决定的行为改动：源语言"自动检测"检出与目标语言相同时自动反向。

**非目标**（各自有归属阶段，本包不做也不验收）：

| 不做 | 归属 |
|---|---|
| 图片翻译全链路：OCR、识别结果编辑、擦除回填、覆盖层、导出（FR-2.1~2.10） | P6/P7 = CHG-005/006 |
| W-1.8 的"假 Key → 红字 + 去设置"视觉同框取证（要覆盖用户真 Key） | P8 = CHG-007 |
| NFR-1 的机翻 P50<800ms 判定（20 次采样脚本）、NFR-5 的 release 体积、NFR-7 跨平台三目标核查、打包 | P8 = CHG-007 |
| AI 五家预设里除 DeepSeek 外四家的真机联调（本机 TLS 建连被重置） | P8，或用户自行切换后各跑一次 live |
| 简洁窗内的历史页、术语表、图片页入口 | 不做：简洁模式按 FR-3.2 只承载"输入 + 译文 + 少量动作"，要这些就切回全量 |
| 多显示器间拖拽的 DPI 切换处理（`pixels_per_point` 变化时的坐标换算） | 本包只保证 FR-5.3 的"不白屏、不越界"，跨屏 DPI 精确定位留 P8 |

本包**不改**引擎语义、不改领域层的持久化格式、不改密钥边界（ADR-005 仍然禁止环境变量注入 Key，Key 只走钥匙串）。

## 事实与假设

| 标签 | 结论 | 来源 / commit / 环境 / 时间 |
|---|---|---|
| FACT | **W-2.1 的根因已定位**：`mode.rs:29-34` 在切到全量时发了 `MinInnerSize(640×420)`，而 `mode.rs:35-40` 切到简洁时只发 `Decorations(false)`/`Resizable(false)`/`InnerSize(460×96)`/`OuterPosition`，**从没清掉那个最小尺寸**，于是 AppKit 把内尺寸夹在 640×420 上——命令发了、窗口没变小。逃生口在 egui-winit：`MinInnerSize(s)` 当 `s == Vec2::ZERO` 时调 `set_min_inner_size(None)`；`MaxInnerSize(s)` 当 `s == Vec2::INFINITY` 时清上限 | `~/.cargo/registry/.../egui-winit-0.37.0/src/lib.rs:2020-2029`、`crates/ice-app/src/mode.rs:26-42`；查阅 2026-10-22 |
| FACT | `InnerSize` 走 `window.request_inner_size(...)`，注释明写"平台不允许该尺寸时返回当前尺寸、请求被忽略" → 尺寸类命令必须**先解除约束再设尺寸**，顺序本身就是正确性的一部分（`commands_for_mode` 已经在测试里断言过 Decorations 先于 InnerSize，这条要扩到 MinInnerSize） | `egui-winit-0.37.0/src/lib.rs:1969-1985`；`crates/ice-app/src/mode.rs:276-286` |
| FACT | `ViewportCommand::StartDrag` → `window.drag_window()`，且**被 `window.has_focus()` 挡着**（注释说 x11 上不判焦点会永久吃掉输入）→ 无边框窗在未聚焦时的第一次按下可能不触发拖动，实现要在把手按下时先确保窗口拿到焦点，并把这条列为真机走查项 | `egui-winit-0.37.0/src/lib.rs:1961-1968` |
| FACT | winit 在 macOS 上 `set_resizable(true)` 只是把 `NSWindowStyleMask::Resizable` OR 进 styleMask，与 `Titled`（装饰）互不依赖 → **无边框窗可以有原生边缘缩放**，不需要自绘缩放手柄 | `winit-0.30.13/src/platform_impl/macos/window_delegate.rs:1037-1050` |
| FACT | 配置层已经为 P5 备好了字段，本包**不需要改 schema**：`UiConfig::{docked_edge, dock_sliver_px(6.0), snap_threshold_px(40.0), dim_when_unfocused(true), clipboard_watch(false), compact_rect(1400,320,460,96)}`，`DockEdge::{Left,Right,Top,Bottom,None}`（默认 Right），以及 `WindowRect::{within, centered_in}`（P1 已有离屏检测与居中） | `crates/ice-core/src/config.rs:17-126`、`config.rs:599`（`window_off_screen_detection` 单测） |
| FACT | `ice_core::detect(text) -> Option<Detection{language, confidence}>` 是纯本地函数（按 Unicode 脚本 + 权重，混合文本降 0.75 置信度），已被 12 项单测覆盖 → 自动反向所需的"检出语言"在 UI 层可直接调用，零网络成本 | `crates/ice-core/src/detect.rs:40-122` 与其 `mod tests` |
| FACT | 机翻引擎在**发请求之前**就拒绝同语言对（返回 `ProviderError::SameLanguage(display_name)`），AI 通道只检测不拒绝；`state.rs::failure_of` 对 `SameLanguage` 不给动作按钮 → 自动反向必须在 `state.rs::submit()`（第 287 行）里、构造 `Job` 之前完成，任何引擎都不会再看到同语言对 | `crates/ice-providers/src/machine.rs:160-164`、`ai.rs:257-259`、`crates/ice-app/src/state.rs:62-74,287-310` |
| FACT | `Ui::set_opacity` / `multiply_opacity` 存在（painter 级，影响其后绘制的内容）→ 内容级降透明随时可做；真正的**窗口级**半透明需要 `ViewportBuilder::with_transparent(true)`，那是创建期属性，运行时 `ViewportCommand::Transparent` 在 macOS 上不保证重建 surface | `egui-0.37.0/src/ui.rs:586-602`；`eframe-0.37.0` 的 `ViewportBuilder` |
| FACT | eframe 0.37 在窗口隐藏期间走 `update_logic_only`：只调 `App::logic`，`App::ui` 一帧都不跑（P4 缺陷 D-2 的根因，已从源码确证）→ 剪贴板轮询与隐藏期心跳必须挂在 `logic` 上；形变动画只在可见期跑，挂 `ui` 即可 | `eframe-0.37.0/src/wgpu_integration.rs:716-767`、`epi_integration.rs:384`；`docs/07-acceptance.md` D-2 第二轮行 |
| FACT | 配置落盘是"脏标记 + 500ms 节流"，`on_exit` 强制刷一次；**SIGTERM 不触发 `on_exit`**（P1 实测，`docs/07-acceptance.md` 已记为 P5 待办）→ 拖动/缩放结束与形态切换必须能自己触发落盘，否则强杀或崩溃会丢掉悬浮条位置 | `crates/ice-app/src/app.rs:36,306-328`；`docs/07-acceptance.md` 变更登记 2026-10-19 行 |
| FACT | 快捷键表里已有 `Cmd+Shift+M`（切换）、`Cmd+Shift+C`（复制译文）、`Cmd+Shift+P`（置顶）、`Cmd+W`（收起）、`Cmd+Q`（退出）、`Enter`/`Cmd+Enter`（翻译）、`Shift+Enter`（换行）；**Esc 尚未绑定** → `03-ui-design.md` §4 的"Esc（简洁模式）清空输入但不退出"与走查 W-2.11 本包补 | `crates/ice-app/src/system/hotkeys.rs:12-54`；`docs/03-ui-design.md:140` |
| FACT | P4 的界面层测试手法可直接沿用：无头 `egui::Context::default()` + `ctx.run_ui` + 断言 `Effects` 队列 + `StubEngine`，不碰网络/剪贴板/钥匙串；viewport 命令用 `ctx.viewport(|vp| vp.commands.clone())` 取出断言（`mode.rs` 的 `queued()`） | `crates/ice-app/src/{stub.rs,mode.rs}` 测试段；`docs/05-dev-guide.md` §6 |
| FACT | P4 的两条教训对本包直接适用：① `Ui::new_child` **不继承裁剪**，`TextEdit::multiline` 按内容全高分配矩形（D-4 根因）→ 简洁窗译文区必须用定高 `ScrollArea`，不能靠手工推进光标；② dingbats 字形（⌗ ✕ ▣ ⧉）既不在 egui 内置字体也无系统回退，会出豆腐块（D-5）→ 简洁窗图标一律用中文短词或 Latin-1 安全字符 | `docs/07-acceptance.md` D-4/D-5 行；`egui-0.37.0/src/ui.rs:210-235` |
| HYPOTHESIS | **无边框 + 可缩放**在 macOS 上真的能拉到边就出缩放光标、且拖动缩放不与我们的吸附判定抢事件。styleMask 层面已确证可行（见上），但"无标题栏时 AppKit 的缩放热区还在不在"只能真机验。不成立的处置见停止条件 1 | 待真机实测；本仓库尚无无边框缩放的实测记录 |
| HYPOTHESIS | **逐帧 `InnerSize` 补间**（180~220ms、ease-out）在 macOS 上不闪、不撕裂、末尾能校正到 ±2px。`03-ui-design.md` §4 定的时间轴（0-35% 改尺寸、35-60% 淡出、60-100% 淡入，`decorations` 只在 35% 那一帧切一次）是设计意图，尚无实测。不成立的处置见停止条件 3 | 待真机实测（录屏 + 几何日志） |
| HYPOTHESIS | **窗口级半透明**可用：创建期 `with_transparent(true)` + 每帧画带 alpha 的背景填充，失焦时 alpha 0.85。若 macOS 上出现黑底、圆角异常或全量模式下视觉受损，降级为内容级 `Ui::set_opacity`（观感是"内容变淡"而非"能看见桌面"），并按停止条件 2 记偏差 | 待真机实测 |
| HYPOTHESIS | **剪贴板轮询的空闲开销可接受**：arboard 3.7 只暴露 `get_text()`（无 changeCount 接口），每 ~700ms 读一次全文并比对，典型剪贴板下是亚毫秒级。若 NFR-2/NFR-5 的空闲 CPU ≥1%，先加大间隔，仍不达标则按停止条件 5 回来找你 | `arboard-3.7.0/src/lib.rs:81-171`；NFR-2 的既有实测是 30 秒 0.000s CPU（`features/CHG-003/evidence/*-V-010.txt` §3，轮询关闭状态） |

## 行为与验收

### REQ-001: 简洁窗尺寸真正生效，且可用鼠标缩放（FR-3.2 改、FR-3.6）

WHEN 用户从全量切到简洁模式
THEN 系统 SHALL 先解除全量模式设下的最小内尺寸约束（`MinInnerSize(Vec2::ZERO)` 等价于 `set_min_inner_size(None)`），再设 `Resizable(true)`、`InnerSize(460×96)` 与 `OuterPosition(compact_rect.x/y)`；命令顺序 SHALL 是"解除约束 → 改尺寸"，否则尺寸会被夹住（这就是 W-2.1 的根因）。

WHEN 切换完成
THEN 简洁窗的内容区 SHALL 是 460×96（±2px）、无系统标题栏装饰、无系统标题文字；`config.ui.compact_rect` 里保存过的尺寸位置 SHALL 优先于默认值。

WHEN 用户把鼠标移到简洁窗任一条边并向外/向内拖
THEN 系统 SHALL 让窗口跟随缩放（走 AppKit 原生边缘缩放，不自绘手柄）；内尺寸 SHALL 不小于 320×96（`MIN_COMPACT_SIZE`），不设上限；缩放结束后新尺寸 SHALL 写回 `config.ui.compact_rect` 并落盘。

WHEN 简洁窗被缩放到任意合法尺寸
THEN 系统 SHALL 保证：输入行、译文区、动作行三块都不被裁掉、不互相重叠；译文区吃掉多余高度并可滚动（见 REQ-005）；动作行始终贴底可见。

WHEN 用户从简洁切回全量
THEN 系统 SHALL 恢复 `Decorations(true)`、`Resizable(true)`、`MinInnerSize(640×420)` 与 `InnerSize(full_rect)`；全量窗的最小尺寸约束 SHALL 重新生效（缩不到 640×420 以下）。

边界：`compact_rect` 若因用户手改配置文件而小于 `MIN_COMPACT_SIZE`，SHALL 被 `sanitize()` 夹回下限而不是拒绝启动；本包不引入"按内容自动长高"（用户 2026-10-22 选的是"能鼠标拖动缩放"，见 Q-004）。
并发：尺寸/装饰命令只在形态切换与缩放结束时发，不逐帧重发（`decorations` 逐帧设会闪，`03-ui-design.md` §4 已明写）。
验证层次：命令序列与顺序用无头断言（`queued(&ctx)`）；"窗口真的变小了""边缘能拉"必须真机走查（W-2.1、新增 W-2.17）。

### REQ-002: 无边框下三处把手都能拖动（FR-3.2、K-06）

WHEN 简洁窗没有系统标题栏
THEN 系统 SHALL 由内容区承担拖动，把手为三处：① 顶部 10px 把手条；② 动作行的空白处；③ 译文区尾部空白。按下任一处 SHALL 立即发 `ViewportCommand::StartDrag` 走原生窗口拖拽，不自绘坐标、不自己算位移（避免与吸附判定抢事件）。

WHEN 用户按在输入框、按钮、引擎分段、译文文本或滚动条上
THEN 系统 SHALL NOT 触发窗口拖动：输入框要能选字、按钮要能点、译文要能选中复制、滚动条要能拖。

WHEN 窗口当前未聚焦而用户按下把手
THEN 系统 SHALL 先让窗口取得焦点再发 `StartDrag`（egui-winit 的 `StartDrag` 被 `has_focus()` 挡着，未聚焦时第一次按下会被吞掉），保证"一次按下就能拖"。

WHEN 拖动松手
THEN 系统 SHALL 触发一次吸附判定（REQ-003），并把最终几何写回 `compact_rect` + 落盘（REQ-009）。

WHEN 置顶开启（`always_on_top = true`）
THEN 拖动 SHALL 同样有效，窗口 SHALL 能被拖到屏幕任意位置且始终在其他应用之上（W-2.3、W-2.13）。

边界：全量模式有系统标题栏，本 REQ 不适用；简洁窗的把手区 SHALL 让光标变成抓手（`CursorIcon::Grab`）以给出可拖暗示。
验证层次：把手命中与"输入框/按钮不触发拖动"用无头断言（模拟指针位置 + 断言 `StartDrag` 有无入队）；真机走查三处把手各拖一次（W-2.13）。

### REQ-003: 四边吸附、拖离与停靠指示（FR-3.5、K-07）

WHEN 拖动松手且窗口到某条屏幕可用区边缘的距离 < `snap_threshold_px`（默认 40）
THEN 系统 SHALL 把窗口贴到那条边并保留 `dock_sliver_px`（默认 6）的可见热区：右 `x = area.right - w + sliver`、左 `x = area.x - w + sliver`、上 `y = area.y - h + sliver`、下 `y = area.bottom - h + sliver`，并把 `config.ui.docked_edge` 设为对应边。

WHEN 松手位置距四边都 ≥ 阈值
THEN 系统 SHALL 保持自由浮动（`docked_edge = None`），位置不变。

WHEN 窗口已停靠而用户从热区向外拖出超过阈值
THEN 系统 SHALL 恢复自由浮动并清除 `docked_edge`；此后松手不再自动吸回原边（除非又进入阈值）。

WHEN 停靠会导致坐标越出可视区（例如窗口比屏幕可用区还高）
THEN 系统 SHALL 把结果夹在可视区内，SHALL NOT 产生负坐标或把窗口推到屏幕外（TC-3.5-e）。

WHEN 简洁窗处于停靠态
THEN 系统 SHALL 在动作行显示当前停靠边（"停靠：右"），点击该指示 SHALL 在四边之间循环（右→下→左→上→右）并立即把窗口移到新边；同时 SHALL 显示置顶状态并允许就地切换（K-09）。

WHEN 吸附几何被计算
THEN 判定 SHALL 是纯函数（输入：窗口 rect + 监视器可用区 rect + 阈值 + sliver；输出：新 rect + `DockEdge`），可无头单测；屏幕可用区 SHALL 取"去掉菜单栏与 Dock"的那一个（egui 的 `ctx.available_rect` 不够，用 `MonitorWorkArea`/viewport 的 monitor 信息）。

边界：全量模式不吸附（FR-3.5 只约束简洁模式）；贴边那一侧的原生缩放手柄与 6px 热区重叠时，**停靠边不允许从该侧缩放**，其余三边照常（实现选择，见 Q-006）。
验证层次：TC-3.5-a~e 全部纯函数单测；真机走查四边各吸一次 + 拖离一次 + 指示循环一次（W-2.5、W-2.6、新增 W-2.18）。

### REQ-004: 形态切换是同一个窗口的形变（FR-3.9）

WHEN 用户触发全量↔简洁切换（按钮或 `Cmd+Shift+M`）
THEN 系统 SHALL 用 180~220ms 的补间让**同一个窗口**在两端 rect 之间形变，不得出现"关一个窗再开一个窗"的跳变、不得黑屏、不得内容闪烁。时间轴按 `03-ui-design.md` §4：ease-out（`t' = 1-(1-t)²`），0%~35% 只改尺寸（每帧一次 `InnerSize`，内容仍按旧形态绘制），35%~60% 淡出旧形态内容，60%~100% 淡入新形态内容，末帧校正到精确 rect（±2px）。

WHEN 动画进行到 35% 那一帧
THEN 系统 SHALL 只在这一帧切换一次 `Decorations`（原生属性，逐帧设会闪）；`WindowLevel`（置顶）在整个动画期间 SHALL 保持恒定。

WHEN 动画进行中用户再次触发切换
THEN 系统 SHALL 立即反向插值（从当前插值位置往回），不排队、不叠加、不重启动画时钟。

WHEN 动画进行中用户点击按钮或在输入框打字
THEN 输入 SHALL NOT 被阻塞：动画只读 UI 状态、不发起请求、不吞指针事件；按钮点击 SHALL 立即响应（W-2.14）。

WHEN 动画结束
THEN 系统 SHALL 停掉逐帧重绘请求（回到"只有提示条到期与结果到达才重绘"的空闲策略，NFR-2 的空闲 CPU 必须仍是 0 量级），并把最终几何按当前形态写回配置。

WHEN 切换发生时有源文/译文/语言对/引擎选择
THEN 内容 SHALL 一律保留，且 SHALL NOT 因为在飞请求或重绘而重发请求（FR-3.3 的内容保留部分已在 P4 闭环，本包不得回退：`toggling_mode_keeps_content_and_marks_config_dirty`、`mode_toggle_preserves_content_and_does_not_request` 必须仍绿）。

边界：动画状态放在 `ice-app::mode::Morph`，只依赖 `ctx.request_repaint()` 与逐帧 `InnerSize`/`OuterPosition`，**不引入动画库**；窗口隐藏期间（`state.hidden`）不启动动画，从 Dock 叫回来后按目标形态一次性设好几何（沿用 P4 的 `reveal`）。
并发：`Morph` 是纯 UI 状态，不跨线程；每帧的插值函数是纯函数（输入 t、起止 rect，输出当前 rect + 内容透明度），可无头单测。
验证层次：插值曲线、端点精度、反向插值、末帧校正、"动画期间不重发请求"用无头断言；"看得出是同一窗口在伸缩"必须真机录屏走查（W-2.14，连续切 10 次）。

### REQ-005: 简洁窗内可用：引擎切换、译文滚动、四个动作、Esc（FR-3.2 改、FR-1.3、W-2.15/2.16/2.11）

WHEN 简洁窗渲染
THEN 系统 SHALL 呈现三块：输入行（单行，Enter 提交，失焦不提交）、译文区（可选中复制，尾部显示当前引擎标记）、动作行；动作行 SHALL 常驻这些控件：`全量`（切回，同 `Cmd+Shift+M`）、`复制`（复制译文，成功时短暂显示"已复制"）、`设置`（切到全量模式的设置页）、`机翻|AI` 分段小按钮（当前引擎高亮）、停靠与置顶指示（REQ-003）。图标一律用中文短词或 Latin-1 安全字符，**不用 dingbats**（P4 缺陷 D-5：⌗ ✕ ▣ ⧉ 无字形会出豆腐块）。

WHEN 用户在简洁窗点 `机翻` 或 `AI`
THEN 系统 SHALL 切换引擎并对当前源文自动重译（复用 P4 的 `state.set_engine`，FR-1.3）；切到 AI 而 Key 未配置时 SHALL 复用 P4 的错误路径：红色可读原因 + "去设置"动作（不在简洁窗内新开一套错误 UI）。

WHEN 译文长度超过译文区当前高度
THEN 系统 SHALL 让译文区在一个**定高 `ScrollArea`**（`auto_shrink([false,false])`）里滚动：滚轮/触控板/拖滚动条都能到末尾，文本不被裁到窗口外、不变形、不贴底（P4 缺陷 D-4 的同一手法）；源文超长时输入行 SHALL 保持单行横向滚动，不把窗体撑高。

WHEN 用户在简洁模式按 Esc
THEN 系统 SHALL 清空输入行（并清掉对应译文与错误），SHALL NOT 退出应用、SHALL NOT 收起窗口（W-2.11；`03-ui-design.md` §4）。

WHEN 有提示条（toast）
THEN 简洁窗 SHALL 也能显示（例如"已复制"、"检测到英语，已自动改为 英语 → 简体中文"），位置不遮住输入行文字，到期自动消失且不常驻重绘。

WHEN 简洁窗宽度被缩到最小（320px）
THEN 动作行 SHALL 不溢出、不重叠：放不下时优先压缩指示文字（"停靠：右"→"右"），按钮保持可点；SHALL NOT 出现控件被裁出窗口（P4 缺陷 D-1 的同类回归）。

边界：简洁窗不提供页签、历史、术语、图片入口（要这些就点 `全量`）；`设置` 按钮的行为是"切回全量 + 打开设置页"，不是"在悬浮条里弹设置"。
验证层次：控件存在性、引擎切换触发重译、Esc 清空、窄宽不溢出、译文区可滚（滚轮 N 帧后偏移增长）全部无头断言（沿用 D-4 的两条测试手法）；观感与真机可点性走查 W-2.2/2.11/2.15/2.16/2.19。

### REQ-006: 自动检测撞到目标语言时自动反向（FR-1.2/FR-1.4 改，TC-1.2-c）

WHEN 源语言为"自动检测"（`config.translation.source == None`）且本地检测出的语言与当前目标语言相同
THEN 系统 SHALL 在**发请求之前**把目标语言翻转为对应语言：`ZhHans`/`ZhHant` → `En`，其余任何语言 → `ZhHans`；随后用新语言对正常提交本次翻译（用户不需要再按一次）。

WHEN 发生自动反向
THEN 系统 SHALL 把新目标语言写回 `config.translation.target`、走 `remember_target`（进最近使用）、标记配置脏以便落盘，并给一条提示条说明："检测到英语，已自动改为 英语 → 简体中文"（措辞按实际语言对生成）。

WHEN 用户**显式**选了源语言且与目标语言相同
THEN 系统 SHALL NOT 自动改任何语言，保持 P4 的行为：就地拦下、不发请求、显示可读中文原因（`ProviderError::SameLanguage` 的文案），因为"改目标语言"只能由用户决定。

WHEN 检测不出语言（纯数字、纯符号、空文本）
THEN 系统 SHALL 不做反向、按 P4 的既有回退走（机翻回退英语、不回显检测结果），SHALL NOT 因为检测失败而阻塞提交。

WHEN 自动反向后翻译成功
THEN 结果回显 SHALL 与新语言对一致：检测语言标记显示检出的语言，历史记录写入的语言对是翻转后的那一对。

边界：只翻目标语言，**永不改用户显式设定的源语言**；一次提交最多翻转一次，翻转后即使再次同语言（不可能，因为翻转结果与检出语言必然不同）也不再翻；反向逻辑 SHALL 是纯函数（输入：检出语言 + 当前目标；输出：新目标或"不变"），可无头单测。
文档变更（用户 2026-10-21 授权）：实现阶段同步改 `docs/01-requirements.md` FR-1.2/FR-1.4 的验收文字、`docs/02-architecture.md` 错误表 `SameLanguage` 行（P4 已埋了指向 CHG-004 的说明）、`docs/06-test-cases.md` TC-1.2-c 的状态列，并在 `docs/07-acceptance.md` 变更登记新增一行。
验证层次：纯函数单测（中→英、英→中、日→中、显式同语言不翻、检测失败不翻）+ `StubEngine` 断言"发出的 `Job` 用的是翻转后的语言对"；真机走查一次英译中（用户原话复现路径）。

### REQ-007: 失焦半透明（FR-3.7）

WHEN 简洁模式且 `config.ui.dim_when_unfocused = true` 且窗口失去焦点
THEN 系统 SHALL 把窗口不透明度降到 85%；重新聚焦 SHALL 立即回到 100%。

WHEN `dim_when_unfocused = false`（设置里关掉）
THEN 系统 SHALL 恒为 100%，不论焦点状态；该开关在设置页可见、改动即时生效并持久化。

WHEN 处于全量模式
THEN 系统 SHALL NOT 降透明度（FR-3.7 只约束简洁模式），失焦只影响光标与提示条，不影响窗口透明度。

WHEN 半透明生效
THEN 提示条、错误红字、译文文本 SHALL 仍然可读（对比度不因降透明而不可辨认）；SHALL NOT 出现黑底、花屏或圆角异常。

边界：优先做**窗口级**半透明（创建期 `with_transparent(true)` + 每帧画带 alpha 的背景）；若 macOS 上不可用或损伤全量模式观感，降级为**内容级** `Ui::set_opacity(0.85)`，并按停止条件 2 记 `SPEC_DEVIATION`（观感差别是"能看见桌面"与"内容变淡"）。
验证层次：透明度取值的决策逻辑（模式 × 焦点 × 开关 → alpha）是纯函数，无头单测四组组合；真机走查 W-2.9（点其他应用观察 + 关闭开关后恒 100%）。

### REQ-008: 剪贴板即翻（FR-3.8）

WHEN `config.ui.clipboard_watch = true` 且窗口可见
THEN 系统 SHALL 每 ~700ms 读一次系统剪贴板文本；出现**新的**文本时 SHALL 自动填入输入行并提交翻译（走与手动提交完全相同的 `submit()` 路径，因此 REQ-006 的自动反向同样生效）。

WHEN 剪贴板内容的变化是应用自己写的（`复制` / `复制译文` / `复制对照`）
THEN 系统 SHALL NOT 把它当成用户输入（否则复制译文会立刻拿译文再翻一次）；判定依据是"上次由本应用写入的文本"，比对内容而非时间。

WHEN 剪贴板内容未变、为空或只有空白、不是文本（例如图片）、或读失败
THEN 系统 SHALL 什么都不做：不填输入、不发请求、不弹错误提示（读失败静默，最多记一条 debug 日志）。

WHEN 用户正在输入框里打字而剪贴板此时变化
THEN 系统 SHALL NOT 覆盖用户未提交的输入；SHALL 等到输入行不再持有键盘焦点时才自动填入（避免抢用户正在编辑的内容）。

WHEN `clipboard_watch = false`（默认）
THEN 系统 SHALL 完全不轮询剪贴板，空闲时不产生任何计时器与重绘请求（NFR-2/NFR-5 的空闲 CPU 必须与 P4 的实测同量级：30 秒 0.000s）。

WHEN 窗口处于收起（隐藏）状态
THEN 系统 SHALL NOT 轮询、SHALL NOT 自动把窗口弹回来（见 Q-005：默认按"只在可见时即翻"实现；若你在评审时改成"收起也要即翻并自动叫回"，实现改挂 `App::logic` 并在本 REQ 补一条行为）。

边界：只读文本，SHALL NOT 读图片（图片入口属 FR-2.1，P6）；SHALL NOT 把剪贴板内容写进日志明文——日志只记长度与哈希前 8 位（NFR-3 的同一口径）。
隐私：这是"应用在后台读用户剪贴板"，默认关闭，开关文案 SHALL 明写"开启后会读取你复制的文本并自动翻译"。
验证层次：轮询的判定逻辑（上次内容 / 自己写的 / 焦点占用 / 空文本 → 该不该填）是纯函数，无头单测五种组合；真机走查 W-2.10（在别处复制一段英文，悬浮条自动出译文）+ 空闲 CPU 复测（开关各测一次）。

### REQ-009: 几何即时落盘与屏幕变化后的合法性（FR-3.6、FR-5.3、P4 转来的待办）

WHEN 拖动结束、缩放结束、吸附完成、停靠边改变、形态切换完成或置顶切换
THEN 系统 SHALL 把当前几何与形态**立刻**标记为脏并触发一次落盘（不等 `on_exit`，因为 SIGTERM/强杀不触发它），落盘仍走 500ms 节流以免拖动过程中每帧写文件。

WHEN 应用被强杀或崩溃后重启
THEN 简洁窗 SHALL 回到用户最后一次停留的位置与尺寸、停靠边与置顶状态（W-2.8）。

WHEN 启动时记录的几何已落在当前屏幕可用区之外（外接屏拔掉、分辨率变化）
THEN 系统 SHALL 把窗口夹回可用区或回到屏幕中心，SHALL NOT 让窗口消失或不可找回（FR-3.6 的验收、FR-5.3 的"不越界"）。

WHEN 系统休眠后唤醒，或显示器插拔
THEN 界面 SHALL 不白屏、不贴图、不越界；若几何失效按上一条处理（FR-5.3）。

WHEN 配置写失败（磁盘满、权限）
THEN 系统 SHALL 保留脏标记以便下次重试、给一条可读提示，SHALL NOT panic、SHALL NOT 阻塞翻译（沿用 P4 `flush_config` 的错误路径）。

边界：`history.jsonl` 的追加写不在本 REQ（P4 已闭环）；本包只处理 `config.toml` 的几何/形态字段。
验证层次：脏标记触发条件与"节流下仍会落盘"用无头断言（`tempfile` 造配置路径，断言 mtime/内容）；离屏夹回是纯函数单测；强杀重启与插拔走查 W-2.8、新增 W-2.20。

### REQ-010: 验收方式与证据规范

WHEN 本包验收
THEN 系统 SHALL 按 `docs/06-test-cases.md` §3 的简洁模式走查表逐条执行（W-2.1~W-2.14 的既有 14 行 + 本包新增的 W-2.15~W-2.20），把"方法 / 预期 / 实测结论"写进 `docs/07-acceptance.md` 的 P5 段；每行 SHALL 记清是"无头断言"还是"用户真机走查"，不得混用。

WHEN 需要真机取证
THEN 启动实例 SHALL 用 `RUST_LOG=ice_translator=debug,info`（crate 名是 `ice_translator`，不是目录名 `ice-app`——P4 第四轮就是因为用错目标丢了整轮日志），交付前 SHALL 先确认日志文件里有"取得单实例锁"那一行再交给用户。

WHEN 需要灌中文样本到剪贴板
THEN SHALL 用 `LC_ALL=en_US.UTF-8 pbcopy < file` 并以 `pbpaste | wc -m` 回验字符数（本机 `pbcopy` 在非 UTF-8 locale 下会静默丢弃非 ASCII）。

WHEN 截图要入库
THEN 每张 SHALL 裁剪到只含本应用窗口所在矩形，提交前逐张确认画面里没有用户其他应用的内容；裁不干净就不提交，改记日志/几何文本证据（P1 立的硬规则）。

WHEN 证据里涉及剪贴板内容或翻译内容
THEN SHALL 只出现长度与哈希前 8 位，SHALL NOT 出现源文/译文全文、Key 明文或任何可还原片段（NFR-3）。

WHEN 证据文件被引用
THEN 文档里 SHALL 写 `features/CHG-004/evidence/*-V-0NN.txt`（取编号最大者）而不是写死编号——证据件编号只增，重录会让写死的编号失效，而改动已记录的证据件会让 `artifact_hash` 失配（`tools/sdd.py:181-188`）。

WHEN 本包完成
THEN 系统 SHALL 回写 `docs/traceability.md`（FR-1.2/1.3/1.4、FR-3.2/3.3/3.5/3.6/3.7/3.8/3.9、FR-5.3、NFR-2/5 的实现列与结果列，清掉"待 P5"）、`docs/02-architecture.md` §2 的 `ice-app` 模块表（`views::compact` 从"—"变成实体，`mode` 补 `Morph`/`snap`，`system::clipboard` 补 `Watcher`）与 §8 落地表、`docs/03-ui-design.md` §3（控件清单加引擎分段与缩放手柄、ASCII 图与"默认 460×96"改成"460×96 起、可缩放、下限 320×96"）、`docs/05-dev-guide.md` 的测试计数与目录结构、`docs/06-test-cases.md` 的 §3 走查表与 TC-3.5-*/TC-1.2-c 状态列；并在 `docs/07-acceptance.md` 变更登记新增相应行（至少三条：FR-3.2 的可缩放与控件清单、FR-1.2/1.4 的自动反向、FR-3.7 若降级为内容级 dim）。文档里不留"待 P5"与"已完成"两种并存说法。

WHEN 阶段验收通过
THEN 系统 SHALL 由用户签 `./sdd accept`（Agent 不得自签），随后 commit + push；提交信息写 `CHANGE-ID@version` + Spec SHA-256（`docs/workflow.md` §敏感信息与提交）。

## 未决问题

| id | 问题 | 阻塞 | 结论 |
|---|---|---|---|
| Q-001 | 简洁窗要不要可缩放？`03-ui-design.md` §3 定的是固定 460×96、`mode.rs` 发的是 `Resizable(false)` | 否 | **已决（用户 2026-10-22 答复"简洁窗口也需要能鼠标拖动缩放啊"）**：改成可缩放，默认仍 460×96、下限 320×96、无上限，尺寸写回 `compact_rect`。需求 FR-3.2 与设计 §3 的文字在实现阶段同步改并登记变更行（REQ-001、REQ-010） |
| Q-002 | 简洁窗内切机翻/AI 的控件形态（动作行只有 28px，原来只有三个动作） | 否 | **已决（用户 2026-10-22 选"动作行常驻「机翻|AI」分段小按钮"）**：常驻、当前引擎高亮、点一下即切并自动重译；动作行按钮改为中文短词以腾出宽度（REQ-005） |
| Q-003 | 本包范围：P5 还挂着三条 Should（FR-3.7 半透明、FR-3.8 剪贴板即翻、FR-5.3 屏幕变化）与一条 P4 转来的待办（几何即时落盘） | 否 | **已决（用户 2026-10-22 选"Must + Should 全做"）**：REQ-001~REQ-009 全包，P6 直接进图片翻译，不再为简洁模式单开一包 |
| Q-004 | 默认 96px 高下长译文仍看不全——要不要让窗口按内容自动长高？ | 否 | **按用户答复推断为"不做自动长高"**：用户对这一问的回答是"简洁窗口也需要能鼠标拖动缩放啊"，即由用户自己拉大窗口 + 译文区可滚（REQ-001/REQ-005）。若你在评审时想要"随内容长高到最多 4 行"，说一声，我把它加进 REQ-001 再重算哈希 |
| Q-005 | 剪贴板即翻在窗口收起（隐藏）时要不要也生效，并自动把窗口叫回来？ | 否 | **默认按"只在窗口可见时即翻"实现**（REQ-008）：收起时不轮询、不弹窗，空闲 CPU 与 P4 同量级。理由是不希望"复制一段文字就凭空弹出一个悬浮条"。若要"收起也即翻并自动叫回"，实现改挂 `App::logic`（隐藏期只有它在跑），我按你的选择在实现阶段调整并登记 |
| Q-006 | 停靠时贴边那 6px 热区与 AppKit 的原生缩放手柄重叠 | 否 | **实现选择，不改需求**：停靠边不允许从该侧缩放，其余三边照常；拖离恢复自由浮动后四边都可缩放。理由是热区优先——用户拉回悬浮条的动作比"从贴边侧拉宽"更常见（REQ-003 边界） |
| Q-007 | 三条 HYPOTHESIS（无边框缩放热区、逐帧 InnerSize 补间不闪、窗口级半透明）都要真机才能定论，而我无法自己驱动 GUI（osascript 被辅助功能权限拒、`screencapture` 出黑图） | 否 | **保持未决，不阻塞批准**：实现第一天先各做一次最小实测（拉一下边、切一次形态、失焦一次），把几何日志与录屏交给你复核。任一条不成立按停止条件 1/2/3 处理——改实现方案的我自己定，改需求或降级 Must 的回来找你 |

## 允许动作与停止条件

**批准即允许**：本地实现与验证；跑真实端点的只读翻译请求（机翻免费端点 + 用户自己的 DeepSeek Key，额度消耗极小）；读写用户配置目录下的 `config.toml` / `history.jsonl` / 单实例锁文件；**在用户开启 `clipboard_watch` 后读取系统剪贴板文本**；在用户于设置页点"保存"时写钥匙串条目（本包不改密钥路径，只可能因 REQ-005 的"设置"按钮跳转而复用 P4 的实现）；按 Q-001/Q-002/Q-004 与 REQ-006 的授权改 `docs/01-requirements.md`（FR-3.2、FR-1.2/1.4）、`docs/03-ui-design.md` §3、`docs/06-test-cases.md` 中与这些决定直接冲突的文字并登记变更行；`docs/` 与 `features/CHG-004/` 的其余文档回写；阶段验收后 commit + push。

**不允许**：未经登记就改需求基线；把源文、译文、剪贴板内容或截图上传到任何第三方；在证据、日志、提交里出现 Key、剪贴板原文或源文/译文全文（只记长度与哈希前 8 位）；删除或改写用户已有文件；未经确认执行 `rm`；把剪贴板即翻默认打开（默认必须仍是 `clipboard_watch = false`）；为了过门禁而修改已记录的证据件。

**停止条件**（触发即记 `SPEC_DEVIATION` 退回评审，不自行改需求或降级 Must）：

1. macOS 上无边框窗拿不到原生边缘缩放热区（HYPOTHESIS 1 不成立），需要加回系统装饰或自绘缩放手柄才能满足 REQ-001；
2. `with_transparent(true)` 导致全量模式或简洁模式出现黑底/花屏/圆角异常，只能降级为内容级 `Ui::set_opacity`（REQ-007 的观感从"能看见桌面"变成"内容变淡"，属可感知降级，需你确认）；
3. 逐帧 `InnerSize` 补间在 macOS 上抖动、撕裂或末尾无法校正到 ±2px，且改实现手段（降低补间频率、只补间尺寸不补间位置、改用 `OuterPosition` 单帧校正）后仍不达标——FR-3.9 是 Must，不能悄悄退回硬切；
4. `StartDrag` 在未聚焦窗口上第一次按下无效，且"先取焦点再发命令"也解决不了（REQ-002 的"一次按下就能拖"是 FR-3.2 的 Must 部分）；
5. 剪贴板轮询把空闲 CPU 顶到 ≥1%（NFR-2/NFR-5），加大间隔后仍不达标，需要你决定是降低轮询频率、改探测方式还是把该功能标记为"仅全量模式可用"；
6. 走查发现控件位置与改后的 `03-ui-design.md` §3 不符，而我判断改设计比改实现更合理（设计变更需要你确认）；
7. 自动反向在真实语言对上产生反直觉结果（例如繁体中文源被翻成简体而不是英文），需要重新定义翻转规则。

## 时间估算

- 开发 9~12 小时：9 个行为 REQ + 1 个验收 REQ。新增 `views/compact.rs`（简洁窗版式，P4 暂由 `app.rs` 私有函数代管）、`mode.rs` 的 `Morph` 补间与 `snap` 纯函数、`system::clipboard::Watcher`、`state.rs` 的自动反向钩子与 Esc 动作；预计新增 900~1,300 行 + 40~60 项测试（插值曲线、端点精度、反向插值、吸附五例、把手命中、透明度四组、剪贴板五种组合、自动反向五例、窄宽不溢出、译文可滚）。
  - 其中不确定性最高的三块：形变动画约 2~3 小时（逐帧命令 + 反向插值 + 末帧校正 + 真机调参）、无边框拖动与吸附约 2 小时（把手命中不能吃掉输入框和按钮，吸附要拿真实监视器可用区）、剪贴板即翻约 1.5 小时（轮询挂点 + 自写回环防护 + 空闲 CPU 复测）。
- 验证 2~3 小时：无头断言为主；真机走查一轮（需要你配合约 30~40 分钟，W-2 全表 20 行，含连续切换 10 次录屏、四边吸附各一次、强杀重启、插拔外接屏）；两条真实端点各跑一次；空闲 CPU 两次采样（剪贴板即翻开/关）。
- 外部等待：你的 spec 批准一轮、走查一到两轮（P4 走了四轮，本包按两轮估）、验收签名一轮——不计入开发工时。
- 依据：P4 的 9 个 REQ 用了约两个工作日（含四轮走查与五个缺陷修复）；本包 REQ 数相近但代码量更小（UI 只有一个视图），风险集中在三条 HYPOTHESIS 与"真机才能定论"的比例上，所以走查轮次是主要变量。
