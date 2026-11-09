# CHG-011 · Design

## 模块划分

| 落点 | 职责 | 为什么在这儿 |
|---|---|---|
| `crates/ice-app/src/system/bar.rs`（新建） | **两条药丸共用的那一件**：缝 `GAP`、翻上下 `BarSide`/`side()`、药丸矩形 `rect()`、按内容定宽 `width_for(n)`、一颗线稿按钮 `glyph_button()`（`Glyph` = 对勾/复制/保存/翻译）、圆角底 + painter 淡阴影 | REQ-001 要求"缝与翻转判据与遮罩那条同一套"。现在 `GAP`/`bar_side`/`bar_rect`/`icon_button` 全在 `selection.rs`（`:387-413`、`:1066`，后者还是私有），贴图这边再写一份就是两份会分岔的几何 |
| `crates/ice-app/src/system/pill.rs`（新建） | **贴图那一扇药丸窗**：视口号构造、跟随（位置命令只此一处）、收放（尺寸命令只此一处）、esc 转交所属贴图、首用提示那一块的绘制与勾选 | 事实 18：`pin.rs:1024-1028` 的形状核查明令 `pin.rs` 里 `OuterPosition` 出现 0 次。药丸窗要被摆位置，命令只能落在另一个模块 |
| `crates/ice-app/src/system/pin.rs`（改） | 贴图窗**缩回图片那一块**（不再 `+BAND_H`）；`band_buttons` → `pill_buttons`（仍遍历 `BAND_ACTIONS` 那一张表）；动作表加 `glyph()` 映射；回调里读自己的 `outer_rect` 后调 `pill::follow` | 动作清单与文案仍只有一处（`both_the_band_and_the_menu_iterate_the_one_table` 那条形状核查继续成立，`"翻译"` 仍只出现 1 次） |
| `crates/ice-app/src/system/selection.rs`（改） | 遮罩那条改成调用 `bar::` 的共用件；`BAR_W/BAR_H/BAR_ICONS` 与 `Area` 那一套**一字不动** | 抽公共不改行为：CHG-010 V-002 的断言语义必须原样绿（见「影响与回滚」的边界） |
| `crates/ice-app/src/main.rs`（改一格） | `NativeOptions::viewport` 加 `.with_transparent(true)` | 事实 16：wgpu 的 `support_transparent_backbuffer` **全进程一个 `Painter` 只从这一份读一次**，此后每扇窗的 surface 都按它选合成模式；不开它，药丸窗的 surface 退回 `Auto`（不透明），缝里照样是实底 |
| `crates/ice-app/src/app.rs`（改两格） | ① 覆写 `App::clear_color` 返回全透明 `[0.0; 4]`；② `draw_pins` 每帧为每张活着的贴图各调一次 `pill::show`（与 `pin::show` 同一条纪律：不调那一帧起窗就被 egui 删掉）；`pin_band_line` 从"带子里那一行"改成"悬停里那一句" | 事实 19：默认清屏色 `(12,12,12,180)` 带七成不透明且**每扇窗都用它**，不覆写则缝里是一层暗纱。②那一格：`show_viewport_deferred` 必须每 pass 调（`pin.rs:442-446`）。覆写后主窗的实心只能靠铺满底色保证，**这一格由 T-001 第五档量，不许假设**（停止条件 8） |
| `crates/ice-app/src/state.rs`（改） | 首用提示那一格状态（本次启动弹没弹、勾没勾）+ 两条意图（确认提示 / 勾不再提示）；`Pin::band_open` 语义不变 | 提示是状态不是绘制；`band_open` 那两条既有断言（`state.rs:3351-3353`）继续成立 |
| `crates/ice-core/src/config.rs`（改一格） | 新增一个 bool，形状照 `clipboard_watch`（`:159`/`:182`），`sanitized()` 跟着走 | REQ-006 就是 CHG-009 停止条件 5 要的那一次批准请求 |

**不动**：`system/cover.rs`（层级判据仍只一个出处，CHG-010 V-007）、`system/tray.rs`（双气泡位图是他已点头的观感，只借它的比例语言）、取图与选区行为、翻译/识别/回填链、历史与老化、密钥边界。

## 关键决定

1. **走 (b) 另起一扇药丸窗，不走 (a) 同一扇窗改透明。** 事实 6：egui 0.37 没有逐像素命中，`mouse_passthrough` 是整窗开关。(a) 会在药丸两侧留下 `(图宽−药丸宽)/2 × 药丸高` 两块"看得见背后应用却点不着"的隐形陷阱（512 宽的图配 110 宽的药丸 = 每侧 201×34），那是事实 3 已经实测过的陷阱的放大版。(b) 把死区压到**药丸外接框之外那一圈阴影留白 + 四个圆角**，量级见 REQ-001。(c) 画成桌面色直接排除（REQ-007 明令不许假装浮空）。
2. **跟随不绕主窗。** `Context::send_viewport_cmd_to(id, cmd)`（`egui-0.37.0/src/context.rs:4569`，文档明写"This lets you affect another viewport"）让**贴图窗自己那一帧**读到 `outer_rect` 后直接把位置推给药丸窗。若走"子窗算 → 主窗存 → 下一帧传回去"那条环，就是 CHG-010 那一轮"左屏拖右屏跳框 + 一闪一闪"的同一个病（[[macos-carbon-hotkey-and-eframe-wake]]）。而且 `show_viewport_deferred` 的回调"可能在父窗睡着时被子窗自己的动画叫醒多次"（`context.rs:4620-4623`），所以从子窗直推反而是最跟手的一条。
3. **位置命令只在真变了那一刻发。** `send_viewport_cmd_to` 内部会 `request_repaint_of(id)`（`context.rs:4570`），每帧发就是每帧把药丸窗叫醒——D-31~D-35 那一族（一次性原生命令改成每帧做之前必须重量代价）的 egui 版。照 `inner_size_command`（`pin.rs:192-198`）同一个形状：上一份命令值相等或差小于 0.5 点就不发。**这份缓存是"上次发出去的值"，不是第二份位置权威**——位置仍然只从 `outer_rect` 现读，与 `size_key` 已有的写法同构。
4. **药丸窗建出来不抢焦点**：`with_active(false)`（事实 17，`viewport.rs:445-457`）。抢焦点会把用户正在打字的那扇应用挤掉，且每钉一张抢一次。点了药丸上的按钮之后焦点自然落到药丸窗，所以 esc 必须能从那扇窗转交回所属贴图（REQ-004）。
5. **阴影用 painter 画，不开 `with_has_shadow`**：`viewport.rs:427-428` 的 macOS 注记明写透明窗要配 `has_shadow(false)` 否则有 ghosting 残影；且整窗窗影会把图片本体也套一圈（Q-002）。
6. **首用提示画在药丸窗里，不另起第三扇窗**（Q-004）：药丸窗本来就是透明可长高的那一扇，提示是它下方多出来的一块；它随药丸一起跟随、一起关闭，主窗收在托盘里也照样看得见。
7. **翻译那颗是双气泡线稿**，比例语言照 `system/tray.rs:216-222` 那两只气泡（前一只左上、后一只右下、重叠处让位），用 painter 描边而不是栅格化位图。不用「文/A」字符字形——D-5 那一族（字符依赖字体、出过方块）。
8. **透明是三格开关，不是两格**（事实 16/19）：① 进程级 `NativeOptions::viewport.transparent`（一个 `Painter` 只读一次，管所有窗的合成模式）、② 那一扇窗的 `with_transparent(true)`（管 NSWindow 的不透明属性）、③ `App::clear_color` 全透明（管没画到的地方被清成什么）。少任何一格，"浮空"就变成实底或暗纱。**代价是①和③都是进程级的**：一改就改了主窗与贴图窗，所以第五档取证与停止条件 8 是本包的闸门，不是可选项。这一格是本包最容易写出"看着改了其实没改"的地方，所以三格各写各的落点、各给各的读数，不许合并成一句"打开透明"。

## 取证计划（T-001，批准前先跑，仓外探针不进仓）

五档，读数逐条抄回 spec.md 事实表（把 H1/H2/H3 改成 FACT 或写明证伪）。三层开关的说法见事实 16/19：① 进程级 `NativeOptions::viewport.transparent`、② 那一扇窗的 `with_transparent(true)`、③ `App::clear_color` 全透明。

| 档 | 量什么 | 判据 |
|---|---|---|
| 1 | **只开②，不开①**（NativeOptions 不动） | 预期落进 `egui-wgpu-0.37.0/src/winit.rs:272-305` 那条退回路径：日志里 `does not support a CompositeAlphaMode with transparency`、合成模式回 `Auto`、缝里是实底。这一档用来证"①②③缺一不可"，不是用来选路 |
| 2 | **①② 开、③ 不动**（默认清屏色 `(12,12,12,180)`） | 预期缝里是一层**七成不透明的暗纱**而不是透明。这一档专量事实 19，读数直接决定 `App::clear_color` 那一格是不是必须改 |
| 3 | **①②③ 全开 + 缝里不画东西** | 缝里与圆角外应当透出背后那扇应用。取证方式：背后放一扇纯色窗，对缝里那一点取像素（只取我们自己探针窗那一块，不整屏抓），比对是不是背后那个色 |
| 4 | **①②③ 全开 + 缝里画一层 alpha=0 的矩形**（H1 的最后一格）＋ **透明区吃不吃点击**（H2）＋ **另起小窗的死区量级**（H3） | 前一件：看 egui 的混合会不会把一个"透明色块"回填成清屏色。中间那件：**不许用 `CGEventPost` 注入点击**（本机注入坐标偏出几百点，判命中是伪证据），改用 AppKit 的 `windowNumberAtPoint:belowWindowWithWindowNumber:` 对缝里、药丸两侧、圆角外各问一次"这个点归哪扇窗"。最后一件：量出 (b) 的残留死区面积并与 (a) 的 `2×(图宽−药丸宽)/2×药丸高` 对比 |
| 5 | **三层全开之后主窗与贴图窗还实不实**（停止条件 8） | 主窗与贴图窗各取一次像素：整片基底是否仍是不透明冷灰白、失焦半透明那一档（`setAlphaValue`）行为有没有变。任一露出桌面就停下来报 |

**探针量不到的两格明写**：① "跟不跟手、闪不闪"归真机走查 W-7.2（注入件的坐标本身就偏，拿它判跟随是伪证据，CHG-010 事实 6 同一课）；② 本机两屏 `backingScaleFactor` 都是 1.0，2× 那一档只能无头断言（[[macos-window-native-limits]]）。

## 验证层次

- **无头（本地可跑，必需）**：几何纯函数、意图矩阵、状态迁移、配置往返、形状核查。`ice-app` 是纯二进制 crate，命令一律 `cargo test -p ice-app <过滤词>`，**不许加 `--lib`**。涉及 `show_viewport_deferred` 的用例必须包在真实 pass 里（`ctx.run_ui(RawInput::default(), |_| …).drop_without_applying_deltas()`），否则 0.37 会 inline 跑回调而字体还没初始化，症状是"No fonts available until first call to Context::run()"。
- **静态（必需）**：平台判定不许散进 `views`（延续 CHG-010 V-004）；层级写入仍只一个出处（延续 V-007）；形状核查三条（拖动一条路 / 位置命令只一处 / 清单只一份）。
- **真机（只有他能定，不计开发工时）**：W-7.1 浮空观感与圆角、W-7.2 五件既有行为 + 跟随四件事、W-7.3 首用提示的位置与观感。
- **本机给不出**：Linux/Windows 的透明那一档（NFR-7 只承诺 S 档，[[cross-platform-ubuntu-gaps]]：Wayland 下 `set_window_level` 是空函数）；HiDPI 2×。两格都登记为"未证"，不许写成"三平台一致"。

## REQ-001 实现与验证

**调用链**：`pin::show` 的子窗回调读 `child.input(|i| i.viewport().outer_rect)` → `pill::follow(child_ctx, pin_id, outer_rect, screen)` → `bar::side(image_rect, pill_size, screen_rect)` 定上下 → `bar::rect(...)` 定药丸矩形 → 加上阴影留白与提示块算出药丸窗的外接框 → `pill::position_command(previous, now)` 只在真变了那一刻 `send_viewport_cmd_to(pill_viewport_id(id), OuterPosition(now))`。

**几何只有一处**：`bar::GAP = 3.0`（从 `selection.rs:399` 搬过来，**值不变**，注释里他那句原话跟着搬）。`bar::rect` 就是现在 `selection.rs:403-413` 那一份（中线对齐、Below = `max.y + GAP`、Above = `min.y - GAP - bar.y`），签名里的 `sel` 改叫 `anchor`，语义一字不改。`bar::side` 把 `field: &Field` 换成 `field: Rect`（它本来只读 `field.bounds.max.y`，`selection.rs:387-394`），这样贴图这边不必为了一个用不上的 `px` 字段去造一份假的 `Field`。

**宽度按内容定**：`bar::width_for(n) = n * 26 + (n - 1) * 6 + 2 * 10`（26 是现在 `icon_button` 的按钮边长，`selection.rs:1067`）。三颗 = 110 点。**遮罩那条的 `BAR_W = 300` 不动**——它还带一句尺寸读数，那是 CHG-010 已批准的样子。编译期断言：`width_for(3) < 300`（防止有人把共用件改成"跟锚点宽走"）。

**死区量级（要写进 docs/05 那条硬规矩）**：药丸窗外接框 = 药丸矩形四周各留 `SHADOW_PAD = 6` 点给 painter 阴影。残留吃命中的面积 = 那一圈留白 + 四个圆角外的小角：`2×(110+34)×6 + 4×6² ≈ 1872` 点²，对比 (a) 的 `2×201×34 = 13668` 点²，小一个量级，且这一圈紧贴着一个看得见的物体（读起来是"药丸的影"，不是"背后那扇应用"）。T-001 第四档量它到底吃不吃点击；若吃到用户能撞见，命中停止条件 2。

**验证**：`cargo test -p ice-app bar`（缝=3、两侧矩形与锚点不相交、宽度不随锚点宽变、放不下翻上方且翻上去也不相交、锚点很小药丸探出两侧时仍以中线对齐）；`cargo test -p ice-app pill`（外接框包含药丸与提示、留白=6、位置命令只在真变时发一次、0.4 点抖动不发）；W-7.1 定观感。

## REQ-002 实现与验证

**三颗是哪三颗**：`pin::BAND_ACTIONS`（`pin.rs:44`，Translate/CopyImage/SaveImage）保持不动，只加一个 `PinAction::glyph()` 映射到 `bar::Glyph`。`pill_buttons`（原 `band_buttons`）遍历的还是这张表，绘制改成 `bar::glyph_button`，返回的 `Response.clicked()` 推 `PinIntent::from_action`——**通道一条没新开**（翻译走既有识别/翻译/回填、复制走 `clipboard::write_image`、保存走 `panel::request_save`）。

**进度挪到悬停**：`app.rs:2085` 的 `pin_band_line` 继续算那一句，但不再占药丸宽度——传给 `pill::show` 后挂在药丸整体的 `on_hover_text` 上；三颗各自的悬停仍由 `bar::glyph_button` 给（现在 `selection.rs:1146-1151` 那三句）。文字层那句手势说明（`TEXT_LAYER_TIP`，`pin.rs:634`）留在图片本体的悬停里，不进药丸。

**关闭只走 esc 与右键**：药丸里没有第四颗；`image_menu`（`pin.rs:289`）仍遍历完整的 `ACTIONS` 四项。项数由既有断言钉住（`exactly_four_actions_shared_by_the_band_and_the_menu`，`pin.rs:744`）。

**线稿共用一处**：`bar::glyph_button` 就是 `selection.rs:1066-1152` 那个函数体搬过来（描边 1.6、悬停变白、`shrink(4.0)`），加 `Glyph::Translate` 一支：两只圆角矩形描边、后一只在前一只右下方、重叠那段不画（比例照 `tray.rs:216-222`）。`selection.rs` 改成调它，`BarIcon` 与 `Glyph` 的对应只在一处。

**验证**：`cargo test -p ice-app pin`（三颗是哪三颗 + 不含关闭 + 右键四项 + 每颗给哪条意图 + `"翻译"` 仍只出现 1 次）；`cargo test -p ice-app bar`（`Glyph` 四颗、两条药丸都从共用件读、形状核查"线稿画法只有一处"）；图标观感归 W-7.1。

## REQ-003 实现与验证

**收起 = 药丸窗那一帧起不再被 show**（egui 以"it was never used this pass"删窗，`pin.rs:442-446`），贴图窗则**本来就一直是图片那一块**——`pin_layout` 去掉 `+BAND_H` 那一支，`window_size` 恒等于图片的点尺寸。事实 3 那条隐形陷阱因此**结构性消失**：贴图窗里再也没有"不画东西却吃命中"的那一条。

**`band_rect` 那一格怎么办**：它现在的语义是"窗口内带子那块"。改成"药丸窗那一块在**屏幕坐标**里的矩形"会让 `pin_layout` 多出一个它不该有的输入（屏幕位置）。所以 `PinLayout` 去掉 `band_rect`，药丸几何整体搬进 `bar`/`pill`；`pin_layout` 只剩 `window_size` 与 `image_rect` 两件，**像素到点仍然只除一次**（`pin.rs:107-125` 那条硬规矩不变）。

**收放只发一次命令**：`pin.band_open` 变 false 的那一帧起不再 `pill::show` ⇒ 窗被删，不需要发尺寸命令；变 true 的那一帧重新 show，`ViewportBuilder` 的尺寸在创建那一帧生效（事实 26）。药丸窗**自己**的高度变化（提示弹出来那一格）走 `pill::size_command`，形状照 `inner_size_command`（相等或差 <0.5 点不发）。

**第一次钉上是展开的**：`state.rs:541` 的 `band_open: true` 不动，既有断言 `state.rs:3351` 继续绿。

**验证**：`cargo test -p ice-app pin`（`pin_layout` 三档：展开与收起的 `window_size` 都等于图片点尺寸、像素到点只除一次、ppp 坏了退回 1 不退回 0、不自己造位置）；`cargo test -p ice-app state`（空格只翻焦点那张、刚钉上是开的）；`cargo test -p ice-app pill`（收起那一帧不再 show、提示长出来时尺寸命令只发一次）；空格真不真生效归 W-7.2。

## REQ-004 实现与验证

**五件既有行为**：拖动（图片本体按下左键那一帧 `StartDrag`，`pin.rs:534-552`）、esc 关这张、`Cmd/Ctrl+C` 复制本体、文字层 Shift 拖选、右键四项——**代码路径一条不改**，因为贴图窗里除了少一条带子以外什么都没动。`key_intent`（`pin.rs:172`）那张矩阵不动。

**esc 从药丸窗转交**：药丸窗的子窗回调里同样跑 `key_intent`，但**只取 esc 那一支**（空格与 C 仍归贴图窗，避免两扇窗各收一份重复触发），推进**所属那张贴图**的意图槽（`intents_slot(ctx, pin_id)`，`pin.rs:412`）并 `root.request_repaint()`。槽是按 pin id 键控的，所以"按一次关掉两张"结构上不成立；六张贴图各跟各的 `outer_rect`、各推各的位置，串格的那一格（CHG-010 那一轮）在这里由"键控 + 子窗直推"两道挡住。

**不开 `movable_by_background`**：形状核查继续断言它在 `pin.rs` 出现 0 次，并新增一条断言它在 `pill.rs` 也出现 0 次（药丸窗不需要拖，它跟着贴图走）。

**事实 18 那条断言按原意拆成两条**（停止条件 9，不许悄悄删）：
- `pin.rs`：`OuterPosition` 仍 0 次、`StartDrag` 仍恰好 1 次、`movable_by_background` 仍 0 次——**原断言一字不删**。
- `pill.rs`：`OuterPosition` 恰好 1 次，且那一处的值只来自 `bar::rect` 的输出（形状核查：该函数体里不许出现坐标字段读写，只许出现 `outer_rect` 与纯函数调用）；`movable_by_background` 0 次。

**顺手修一处注释 rot**：`pin.rs:218-225`（`image_area` 的文档）现在还写着"左键按住拖动交给系统（`with_movable_by_background(true)`，事实 29/32 实测能拖）"，与实际代码和形状核查正好相反。改成描述现在那一条 `StartDrag`，解释性注释只留一处（模块硬规矩与 D-29 那段已经在 `:474-477`，这里只引它）。

**验证**：`cargo test -p ice-app pin`（意图矩阵、真实 pass 里 esc → Close、空格 → 收放、形状核查三条）；`cargo test -p ice-app pill`（药丸窗收到 esc 只推所属那一张的槽、两张同时在时各推各的、药丸窗不取空格与 C）；`cargo test -p ice-app app`（`app.rs:4975-4976` 那两条"第二张收了第一张不被牵连"继续绿）；手感归 W-7.2。

## REQ-005 实现与验证

**状态（`state.rs`）**：`AppState` 加一格 `pin_hint: PinHint`，三态 `NotShown / Showing { dont_show_again: bool } / DoneThisRun`；两条意图 `AckPinHint { dont_show_again: bool }`。迁移：
- 第一次钉上贴图（`pins` 从 0 变 1）且配置里那个 bool 为 false 且本进程还没弹过 ⇒ `Showing`。
- 确认且勾了 ⇒ 写配置 bool = true、`DoneThisRun`，此后不再弹（跨启动也不弹）。
- 确认没勾 ⇒ `DoneThisRun`，**不写配置**，下一次启动应用再弹一次（不是每钉一张都弹）。
- 关掉了那张贴图 ⇒ 提示随之消失（它画在药丸窗里，窗没了提示就没了），本进程仍算 `DoneThisRun`。

**esc 不许被挡**：提示开着时按 esc 走的是同一条 `PinIntent::Close`——图关了、药丸窗没了、提示也没了，**一下做到两件**，不做"先关提示再关图"的两段式。提示自己的「知道了」只关提示不关图。

**绘制（`pill.rs`）**：提示是药丸下方多出来的一块（同一扇透明窗，`SHADOW_PAD` 留白照旧），放不下时与药丸一起翻上方（同一套 `bar::side`，锚点仍是图片）。里面两句话 + 一个勾选 + 一颗「知道了」。**文案与代码同源一处**：`pill.rs` 里一张常量表，docs/03 引它，不另抄一份。

**落点**：它画在药丸窗里 ⇒ 一定在用户正看着的那块屏上（药丸窗的位置就是从那张贴图的 `outer_rect` 现推的），主窗收在托盘里也照样看得见（Q-004）。

**验证**：`cargo test -p ice-app state`（四条迁移：第一次弹 / 勾了不再弹 / 没勾下次启动还弹 / 提示开着 esc 仍关图；再加"同一进程钉第二张不再弹"）；`cargo test -p ice-app pill`（提示块在药丸下方、放不下一起翻上方、提示长出来时窗高变化只发一次尺寸命令）；位置与观感归 W-7.3。

## REQ-006 实现与验证

`config.rs` 照 `clipboard_watch: bool`（`:159`，默认 `false` 在 `:182`）加一格 `pin_hint_dismissed: bool`，默认 `false`（= 还要提示）。`sanitized()` 那一层跟着走（bool 无需夹取，但要在往返里不被丢掉）。**只加这一个**：历史天数、张数上限、药丸尺寸、留白宽度都不进配置（CHG-009 Q-009 那条纪律）。它不承载任何用户内容——只是一个"提示过没有"的标记，不含文本/路径/图像，所以 `docs/01` 的 NFR-3 隐私口径一字不动。

**验证**：`cargo test -p ice-core config`（默认 false、serde 往返、旧配置文件缺这一格时回 false、`sanitized()` 不把它夹坏）；静态核查"配置面只多这一个字段"（与批准前的字段清单逐条比对，写进 evidence）。

## REQ-007 实现与验证

**分档口径**：macOS 这一档由 T-001 与 W-7.1~W-7.3 定；Linux/Windows 登记为"本机给不出"（NFR-7 只承诺 S 档）。平台判定**不许散进 `views`**（延续 CHG-010 V-004 那条静态判据）：透明那一格若要有平台差异，只许落在 `system/` 这一层。

**证伪就如实退回**：T-001 第三/四档若量到三层全开仍透不出（H1 证伪），或第四档量到 (b) 的死区绕不开（停止条件 2），或第五档量到主窗不再实心（停止条件 8）⇒ 退回"带子是窗口一部分"的旧形态，`deviation` 登记原因，**不用桌面色假装浮空**。退回时 `bar.rs` 的共用件仍可留着给遮罩那条用（它本来就是从那儿搬来的），`pill.rs` 不接线；`main.rs` 与 `app.rs` 那两格（进程级 alpha、`clear_color` 覆写）**一起撤回**，不许留一格"透明但没人用"的开关——那会让主窗的清屏色永远是一个没人看得见的值。

**首用提示与那一个 bool 与透明无关**：非 macOS 上照常成立（它画在窗口里，不依赖透明）。若退回旧形态，提示改画在带子下方那一块（仍在贴图窗内），四条状态迁移一字不改。

**验证**：静态核查（`views` 里无 `cfg(target_os`、层级写入仍只 `cover.rs` 一处）；`cargo test --workspace`（退回路径也不许让既有断言变红）；Linux/Windows 与 HiDPI 两格在 evidence 里明写"本机给不出"。

## REQ-008 实现与验证

**旧契约就地改口**：CHG-009 `spec.md` 的 `REQ-003` 那句「SHALL NOT 依赖"窗口透明 + 按钮浮在透明区"」与 `design.md` 开放点 1 **不改写当年那行**，在各自行下加一条带日期的补正（`2026-11-09 补正：CHG-011 REQ-001 起改为浮空药丸，依据是用户当日原话与事实 16/17/19 的依赖源码读数；当年搁置的理由（"egui 的 viewport 透明本仓没用过"，且那份旧文把后端写成了 glow，本仓实际跑 wgpu）已由 T-001 闭合/或已如实退回`）。CHG-009 已批准快照不动——补正写在 spec 正文的沿革段，若 `./sdd validate` 因批准哈希漂移报错，则走 `deviation` 登记而不是改快照。

**回写清单**：`docs/02`（模块表加 `bar.rs`/`pill.rs` 两格、`pin.rs` 那一格改口）、`docs/03`（贴图窗版式：3pt 缝、药丸宽度按内容定、首用提示的样子与文案）、`docs/05`（§6 硬规矩加一条可核查的："透明窗里没画东西的地方仍会吃命中，所以浮空物件的外接框只许比它本体大一个阴影留白，量级要写出来"）、`docs/06`（新 TC 与 W-7.1~W-7.3 走查）、`docs/07`（变更登记）、`docs/traceability.md`。`docs/01` 的 NFR-3 不动。

**留档次序（这一格会咬人）**：`docs/` 在树指纹内（`tools/sdd.py:164-188` 的 `fingerprint`，`LEDGER` 只排除 `features/`、`quick/`、`incidents/`、`project/STATE.md`、`.sdd/`），所以**改文档会让已录证据过期**。次序必须是：文档定稿 → 走查登记 → `./sdd check`/`record` 一次性做完 → 之后不再动任何树内文件 → commit（commit 与录证据都不改 `tree_hash`，安全）。

**验证**：`./sdd validate --offline`；追溯矩阵四边覆盖（`plan_valid`，`sdd.py:195-238`：Requirement ↔ Design ↔ Task ↔ 必需 Verification）；`./sdd check` 逐条录。

## 影响与回滚

**涉及仓库**：`ice-translator` 一个，目标分支见 `codebase/repositories.json`（批准时快照进 `approval.branches`）。

**兼容性**：配置只加一个默认 false 的 bool，旧配置文件缺这一格回默认（REQ-006），不需要迁移。贴图的历史文件、老化口径、翻译回填全不动。

**允许改的既有断言（边界写死，越界即命中停止条件 6/9）**：只有**编码了被本包改口的那条契约**的那几条可以改，且改的是"契约换了"不是"迁就实现"：
- `pin.rs:686` `the_open_band_sits_below_the_image_and_grows_the_window`、`:700` `a_closed_band_shrinks_the_window_itself_and_leaves_no_dead_strip`、`:715`/`:733`/`:778` 里用到 `BAND_H` 的那几处断言 ⇒ 改成编码新契约（窗口恒等于图片那块、药丸在另一扇窗、缝=3、两侧不相交）。
- `pin.rs:1034` `both_the_band_and_the_menu_iterate_the_one_table` 里的函数名 `band_buttons` ⇒ 跟着改名成 `pill_buttons`，**"只遍历一张表"与"`翻译` 只出现 1 次"两条语义不动**。
- `selection.rs` 里 `bar_side`/`bar_rect` 的调用点 ⇒ 只改参数形式（`&Field` → `Rect`），**断言语义一字不动**，CHG-010 V-002 必须原样绿。
- 其余 **740 项里凡与本包无关的断言一律不许动**（CHG-010 V-001 那条注记的口径）。实施前先录一次全量数与失败数，实施后逐条比对，差集只许落在上面这三类里。

**回滚条件**：停止条件 1/2/3/7/8 任一命中，或真机走查报 D-29 那一族回退。

**回滚方法**：`pin.rs` 恢复 `+BAND_H` 那一支与带子绘制、`app.rs` 不再调 `pill::show`、`main.rs` 那一格透明开关撤回 false；`bar.rs`/`pill.rs` 留在仓里不接线（用户 2026-11-09 定的纪律：新码留仓、入口退回旧路径）。配置那个 bool 留着无害（默认 false，旧路径不读它）。**不用 `git checkout --`/`restore` 动任何带未提交工作的文件**（[[feedback-no-vcs-restore-on-dirty-files]]），要看旧写法用只读的 `git show HEAD:<path>`。