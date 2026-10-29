# 架构设计 — Ice Translator v1.0

前置阅读：`01-requirements.md`（需求编号）、`adr/`（关键取舍理由）。

## 1. 分层与依赖

```
                     ┌──────────────────────────────┐
                     │  ice-app  (eframe / egui)    │  事件循环、双模式布局、任务编排
                     └───────┬──────────────┬───────┘
                             │              │
              ┌──────────────▼──┐   ┌───────▼───────────────┐
              │ ice-providers   │   │ ice-vision            │  引擎调度/缓存/HTTP   OCR/回写/几何
              └──────────┬──────┘   └───────────┬───────────┘
                         │                      │
                     ┌───▼──────────────────────▼───┐
                     │  ice-core                    │  类型、配置、历史、术语、纯计算
                     └──────────────────────────────┘
```

规则：箭头单向向下；`ice-core` 无副作用；只有 `ice-app` 依赖 UI。（理由见 ADR-006）

## 2. 模块与关键类型

### ice-core

| 模块 | 关键类型 | 职责 | 需求 |
|---|---|---|---|
| `language` | `Language`、`LangPair`、`EngineKind` | 语言码/显示名/别名解析、宽度系数、按字断行标记 | FR-1.2~1.4、FR-2.7 |
| `document` | `PixelRect`、`PixelPoint`、`TextBlock`、`OcrDocument` | 图片文档模型；框几何运算（union、vertical_gap、字号估算） | FR-2.2~2.8 |
| `config` | `Config`、`UiConfig`、`TranslationConfig`、`AiConfig`、`WindowRect`、`UiMode`、`DockEdge`、`Theme` | 配置读写（原子替换 + 损坏回退 + sanitize 夹取）。**主题三态在 2026-10-28 变更 J 之后这里只剩数据**：那颗滑块按**落点位置**取档（映射在 `ice-app::views::theme_slot` / `theme_at`），不再有点三下的循环，所以 `Theme::next()` 与它的测试一起删除——领域层不该留一个没人调的换档式子 | FR-3.6、FR-4.1~4.3 |
| `history` | `HistoryRecord`、`HistoryStore` | JSONL 追加、容量裁剪、搜索、坏行跳过 | FR-4.4 |
| `glossary` | `Term`、`Glossary` | 术语读写、`prompt_fragment()`、`post_substitute()` | FR-4.5 |
| `detect` | `guess_language()` | 按字符集构成做语言粗判，用于自动检测回显。**计数按翻译单元不按字符**（2026-10-28 复核）：汉字一字一单元、拉丁一段词一单元，中英相等判中文——按字符比会让借词按字母投票，中文句子被判成英语、连 REQ-006 的方向一起带错。复核实例与判据在 `examples/detect_probe.rs` | FR-1.2 |

### ice-providers

| 模块 | 关键类型 | 职责 | 需求 |
|---|---|---|---|
| `engine` | `TranslationEngine`（trait）、`unavailable_error()` | 通道抽象：`kind/translate/is_available` | FR-1.3 |
| `request` | `TranslationRequest`、`TranslationResult` | 通道间唯一的入参/出参结构：语言对、术语提示、分段续译的上下文尾巴、耗时与命中来源 | FR-1.1、FR-1.3、FR-1.9 |
| `machine` | `MachineTranslateEngine`、`MACHINE_QUERY_CHARS` | 零配置机翻通道（MyMemory），单次查询 450 字符上限、超时与网络类重试 ≤2；超上限切块后**并行**在飞（`parallel`）并按块序拼回 | FR-1.3、FR-1.7、FR-1.9、NFR-1 |
| `ai` | `AiEngine` | OpenAI 兼容 chat 通道，提示词构造、分段与上下文续译、拼回；按 `AiProvider::thinking_off()` 塞各家拼法的关闭思考字段；分段**并行**在飞 | FR-1.3、FR-1.9、NFR-1 |
| `parallel` | `map_segments()`、`MAX_IN_FLIGHT` | 分段并行的唯一实现：`tokio::task::JoinSet` + 定长结果槽，在飞窗口 ≤ 4，**按段序**返回、任一段失败即整体失败并 `abort_all()` 取消其余（不私有，只对本 crate 的两个通道开放） | FR-1.9、NFR-1 |
| `protect` | `ProtectedText`、`Restored`、`protect()`/`restore()`、`chunk_chars()` | URL/邮箱/代码块/标识符/带单位数字的占位符保护，以及"取额度内**最后一个**句号或换行断开、不切开占位符"的切块 | FR-1.6、FR-1.9 |
| `cache` | `TranslationCache`、`CacheKey`、`CachedValue` | LRU；AI 通道以术语表版本号加盐 | FR-1.8、FR-4.5 |
| `dispatcher` | `Translator` | 选引擎→查缓存→调用→术语后处理→回写缓存；`availability()` | FR-1.3、FR-1.7、FR-1.8 |
| `error` | `ProviderError`（10 个变体）、`retryable()`、`from_reqwest()`/`from_status()` | 网络/状态/解析错误归一化为可读种类；错误文本剔除 URL 查询串，不回显源文与 Key | FR-1.7、NFR-3 |

`Translator` 是 UI 唯一允许接触的入口。UI 不知道有几个 HTTP 端点、缓存怎么键控。

### ice-vision

| 模块 | 关键类型 | 职责 | 需求 |
|---|---|---|---|
| `ocr` | `OcrEngine`（trait）、`VisionOcr`、`default_engine()` | 识别入口；平台能力探测 | FR-2.2、NFR-7 |
| `vision_sys`（macOS） | 对 `VNRecognizeTextRequest` 的封装、坐标转换 | 归一化左下坐标 → 像素左上坐标；置信度与候选文本 | FR-2.2 |
| `layout` | `merge_lines_into_blocks()`、`RowKey` | 行级观察合并为段落块（同左边界/垂直间隙小于 1.2 倍行高/水平重叠） | FR-2.2 |
| `bg` | `BackgroundEstimate`、`estimate()` | 框边缘采样 → 背景色、前景色、方差（决定回填 or 覆盖层） | FR-2.4、FR-2.5、FR-2.8 |
| `composite` | `Renderer`、`CompositeOptions`、`CompositeOutput`、`TextLayout` | 擦除、排版、绘制、字体回退 | FR-2.4~2.8 |
| `font` | `FontSet`、`resolve()` | Core Text 枚举系统字体、CJK 回退、缺字形检测 | FR-2.7、NFR-4、R-6 |

### ice-app

> 二进制 crate（`main.rs` 里 `mod`，非 `lib.rs`）。下表在 P4 实现后按实际代码校正：设计里的 `FullView`/`CompactView`/`ImageView`/`SettingsView` 四个结构体最终落成各视图模块里的自由函数 + `AppState`，因为界面状态只有一份、不需要每页再包一层持有者；`JobScheduler`/`ModeController` 同理落成 `JobRunner` 与 `mode` 模块的函数集。

| 模块 | 关键类型 | 职责 | 需求 | 状态 |
|---|---|---|---|---|
| `app` | `IceApp`、`Effects`、`Prepared` | 实现 `eframe::App`；持有 `AppState`、`JobRunner`、tokio runtime、历史存储、配置路径、单实例锁、钥匙串探测回收端；帧末统一执行副作用（含 `Effects::{quit, reset_geometry}` 两条来自设置页的意图）与退避落盘；原生标题栏拖动靠 `observe_native_move` 的几何跳变观测补上吸附判定；`settle_drag` 每帧**先问硬件**（`system::mouse::left_button_down()`），左键还按着就 `DragSettler::postpone` 把静止计时重新起算、这一帧一个结论都不下，问不到才按时间判（缺陷 D-16）；每帧按当前形态调 `system::{alpha, titlebar}::set/sync`（`apply_native_alpha`、`apply_zoom_hook`，只在结果变了才记日志），并在帧首 `poll_titlebar_click` 把绿键那次点击兑成 `Effects::toggle_mode`；**图钉不再是原生那一条**（2026-10-29 变更 K：`apply_pin` / `poll_pin_click` / `system::pin::sync` 三个调用点连同整个模块一起删除，图钉改由 `views::pin_toggle` 在两种形态的界面里各画一格，直接置 `Effects::toggle_topmost`）；**标题栏双击那条只在全量形态挂**（`apply_dblclick_hook` / `poll_dblclick`，任务 #25）：每帧调 `system::dblclick::sync(frame, mode == Full)`，取到一次双击置 `Effects::toggle_fill`，`toggle_fill` 据此在"铺满当帧可用区"与"还回 `full_rect` 记着的那组几何"之间往返，两种去向都只发 `InnerSize` + `OuterPosition` 各一条、并把即将下发的内尺寸记进 `PendingGeometry`；铺满期间 `state.filled = true`，`mode::capture` 靠它整帧早退（见 `mode` 行），槽里保住的才是还原的源；挂载结果只在变化时记日志，接不上时一次性 WARN | 全部 | ✅ P5 |
| `main` | `prepare()`、`main()` | 启动时序：日志 → 单实例锁 → 读配置 → 导入旧术语表 → 装配引擎 → tokio → 建窗（`main()` 里 `run_native` 的创建闭包第一件事是 `fonts::install`，中文落在黑体上必须赶在第一帧排版之前）→ **后台**读钥匙串 | FR-4.1~4.3、REQ-009 | ✅ P4 |
| `state` | `AppState`、`Failure`、`FailureAction`、`Job`、`Outcome`、`Toast` | 源文/译文/语言对/引擎/在飞任务/失败态/图片工作区/历史查询/提示条；`request_id` 竞态丢弃。**提示条（2026-10-28 变更 I）**：`TOAST_LIFETIME = 500ms` 一个数同时管"活多久"和"淡多久"，`Toast::opacity()` 给线性不透明度、`expired()` 到点自己走；**没有"用户点掉一条"这条路**（`dismiss_toast` 与那颗 `×` 一起删除），`tick_toasts` 是唯一的过期通道。**`filled`（任务 #25）**：窗口当前是不是被双击铺满在整屏可用区上，**只在运行时、不进配置也不落盘**——它是 `mode::capture` 的抑制开关，也是"下一次双击该还原而不是再铺一次"的判据；形态切换（`apply`）与从托盘/收起唤回（`reveal`）都会清掉它，因为那两条路径自己就把几何整套重发了一遍 | FR-1、FR-2 | ✅ P4 |
| `mode` | `commands_for_mode()`、`apply()`、`toggle()`、`capture()`、`PendingGeometry`、`resend_commands()`、`fill_commands()`、`fill_size()`、`restore_commands()`、`snap()`、`dock()`、`reveal_rect()`、`dock_band_contains()`、`released()`、`fit_into_area()`、`DragSettler`、`RevealGuard`、`dim_alpha()`、`level_for_dock()`、`MIN_FULL_SIZE`/`MIN_COMPACT_SIZE` | 两种形态的 `ViewportCommand` 编排（**先改 `MinInnerSize` 再发 `InnerSize`**，顺序本身就是正确性）、置顶与停靠层级、几何捕获与即时落盘、左/右边缘吸附-停靠-悬停唤出-拖离的纯函数、拖动静止判定、失焦降透明度、形态切换的几何宽限期（D-6，v0.7 起是**三段**护栏，见 §3.3）。**铺满/还原（任务 #25）三条**：`fill_size(area, inner, outer)` 用**当帧实测**的 `outer − inner` 当装饰厚度从可用区里减掉（本机 32pt 高、0 宽；不写死是因为 Retina/无边框/将来去掉原生标题栏都会让它变，且两个轴各自 `max(0.0)`——负尺寸会被原生静默忽略，比"铺小了一点"更难查），`fill_commands` 发 `InnerSize` + `OuterPosition(area.min)` 各一条，`restore_commands` 由 `resend_commands(cfg, Full, target_size(cfg, Full))` 派生，保证护栏记的数与发出去的数同一个出处。`capture` 在 `state.filled` 为真时**整帧早退**：铺满期间实时几何就是整屏，让它进槽等于把 D-6/D-10 换个入口重犯，而且 FR-3.6 记着的"上次的大小"会变成整屏，重启后窗口再也不是用户那个尺寸。FR-3.9 已于 2026-10-22 改为瞬时切换，**无 `Morph`** | FR-3.2~3.9、FR-5.3 | ✅ P5 |
| `theme` | `apply()`、`apply_font_scale()`、`PaneText`、`title_color()`、`surface_fill()` | 三态主题映射到 `ctx.set_theme`，字号缩放乘在 egui 默认值上并限幅；`apply()` 每帧重赋 `Spacing::extra_text_line_spacing`（赋值不是累加，帧循环会跑几百次）；**变更 H 新增 `PaneText`——把「字号 + 字距 + 行距」三件事一次性收成一处**，四个正文调用点（全量源文/译文、简洁输入带/译文带）共用，避免 egui 0.37 那三个不同入口（`FontId`、`extra_text_line_spacing` 只喂 TextEdit、section 上的**绝对** `line_height` + `extra_letter_spacing` 要手工挂）各写一份漂掉 | NFR-4、NFR-8 | ✅ P5 |
| `fonts` | `definitions()`、`install()`、`HEITI_PATH`、`HEITI_SC_FACE_INDEX` | 赶在第一帧排版之前（`run_native` 的创建闭包里）把 `/System/Library/Fonts/STHeiti Medium.ttc` 装成字体族：`Proportional` 第 **0** 位、`Monospace` 里排在 Hack **之后**（拉丁列要继续保持等宽）。**面索引 1 才是黑体-简**（第 0 面是黑体-繁，选错不崩不缺字，只有肉眼看得出）；读不到文件返回 `false` 并回落系统解析，**不许因缺字体让界面空白** | NFR-8 | ✅ P5 |
| `views::mod` | `Tab`、`chrome()`、`theme_switch()`（内部：`theme_slot`、`theme_at`、`draw_sun`、`draw_moon`）、`pin_toggle()`（内部：`pin_tip`、`draw_pin`、`pin_id`、`PIN_TOGGLE_W`/`PIN_TOGGLE_H`、`PIN_TILT_DEG`） | 顶部工具条 + 页签 + 状态条（设计里归在 `views::full`，实现时抽出共用，简洁模式也要用同一条状态条）；**`theme_switch` 是 2026-10-28 变更 J 抽出来的第二处共用件**——那颗左右推动的主题滑块在简洁窗动作行右下角和全量窗底栏右端是**同一个函数**（`salt` 只用于各自记录命中矩形），落在这一层是因为用户那句"全量界面也要有这个开关"要的是两处不许各画一遍。三态映射是纯函数：`theme_slot` 给 Dark/System/Light = 0.0/0.5/1.0，`theme_at` 按 0.25/0.75 反解（因此天然带迟滞，拖过中间不会连翻三档）；写 `config.ui.theme` + 标脏，生效仍只在 `theme::apply`。**`pin_toggle` 是第三处共用件（2026-10-29 变更 K）**——用户那句"全量窗挪动到精简（悬浮窗）两个字左侧，悬浮窗状态下，放在（右下角）颜色切换左边"要的是同一枚图钉出现在两处，而原生标题栏那颗（`system::pin`）已经连根删掉：egui 画不进 `NSTitlebarView`，能画的地方就只能两处各调同一个函数。图钉是**矢量线稿**（圆帽 + 横梁 + 针，字符图标在 P4 缺陷 D-5 落成过豆腐块）：未置顶 = 空心、斜 `PIN_TILT_DEG`、用主题前景色；置顶 = 实心、摆正、`theme::ACCENT`。它只立意图（`Effects::toggle_topmost`），与 `Cmd+Shift+P`、设置页那一格收口在同一条层级下发上，**这一层不碰窗口**（`salt` 只用于两处各自记录命中矩形） | FR-3.1、FR-3.4、NFR-4、FR-4.1 | ✅ P4（滑块与图钉在 P5） |
| `views::full` | `show()`、`copy()`、`scroll_area()`、`record_scroll()`、`pane_body_height()`、`char_count_text()`、`bar_actions()`、`bar_layout()`、`bar_height()`、`bottom_bar()` | 全量布局文本页：源文编辑器、译文区两栏等高，两栏之下是**自占一行**的动作底栏（`Panel::bottom`，左边字数计数、右边三/四枚按钮，退让顺序 `84×28` → `64×28` → 换行；**最右那格是 `views::theme_switch`，2026-10-28 变更 J 加**——它排进 `right_to_left` 子带的第一个所以落在整条栏右边缘，而 `bar_layout` 拿到的可用宽要先扣掉它的 40px，否则按钮会排到滑块底下）；失败动作按钮在同一行 | FR-1.1~1.5、FR-3.1、NFR-4 | ✅ P5 |
| `views::history` | `show()` | 历史页：搜索、点选回填、时间戳本地化、坏行容错 | FR-4.4 | ✅ P4 |
| `views::settings` | `show()`、`select_default_engine()` | 供应商预设、密钥写钥匙串、主题三态、字号、额外语言、术语分区；外观分区（变更 H）在整体字号之下再给四条滑杆——原文字号 / 译文字号（各 `0.8..=2.0`）、字间距（`0..=4px`）、行间距（`0..=16px`），每根只写 `config.ui` + 标脏；「窗口」分区（F-17）给出「恢复默认窗口尺寸」与「退出程序」两颗看得见的入口，只立意图（`Effects::{reset_geometry, quit}`），落地在 `app` | FR-4.1~4.2、FR-4.5、FR-5.2、NFR-4、NFR-8 | ✅ P5 |
| `views::image` | `show()` | 图片页占位：五个区域已画好，接 OCR 在 P6 | FR-2.3、FR-2.6 | 占位 |
| `views::compact` | `show()`、`band_heights()`（内部：`input_row`、`translation_pane`、`action_row`、`drag_handle`、`band_frame`、`apply_dim`） | 简洁窗版式：带原生标题栏（Q-013），三带堆叠——动作行先建成 `Panel::bottom`，剩余高度由 `band_heights()` 按 **1:1 分给输入带与译文带**（REQ-012，v0.7 推翻了 v0.6 Q-014 的"输入行随内容长到三行封顶"），带间 `BAND_GAP` ≥5px，两带各自是独立圆角框；输入超一屏在框内滚，译文带定高 `ScrollArea` + 尾部 8px 把手；动作行按左→右排**三颗**（复制/设置/机翻\|AI），**右下角钉一格 `views::theme_switch`**（变更 J：它取代了原先行首那颗主题切换框，也取代了行右边缘那颗 `退出`；**「全量」那颗已于 2026-10-28 变更 G 删除**，切回全量改由标题栏绿键与 `Cmd+Shift+M` 承担，见 `system::titlebar`；**「置顶」那颗文字开关已于同日变更 I 删除**；**变更 I 把它换成的原生标题栏图钉已于 2026-10-29 变更 K 撤掉，图钉改由 `views::pin_toggle` 画在这一行的右下角、滑块左边**（`system::pin` 整个模块随之删除，那套 accessory 原生机制不再留存）**），与全量窗同字号同内边距，左排放不下换行、滑块与图钉由一条 `right_to_left` 子带单独钉右（滑块先占走右边缘、图钉后加就落在它左边；子带竖向范围取左排实测那一格，行高仍由左排决定，两者都不参与换行）；两处窗内把手走 `StartDrag`（左排与图钉之间那块空白、译文带尾部 8px——变更 K 之后把手右端从"滑块之前"改成"图钉之前"，图钉那一格自己验过不触发拖拽）；失焦时窗体底色与两带圆角框一起乘 alpha | FR-3.2、FR-3.7 | ✅ P5 |
| `jobs` | `JobRunner`、`drain()`、`apply()` | `spawn` 网络任务，经 `mpsc` 回投 UI 并请求重绘；`spawn_blocking` 图像任务待 P6 | FR-2.9、NFR-1、REQ-002 | ✅ P4（文本部分） |
| `system::clipboard` | `CopyKind`、`compose()`、`write()`、`read()`、`Watcher` | 复制译文 / 复制对照与空译文保护；`Watcher` 是剪贴板即翻的轮询判定（纯函数：开关、可见性、在飞、自写回环、内容变化五道闸），读取本身由调用方注入闭包，因此无头可测 | FR-1.5、FR-3.8 | ✅ P5 |
| `system::screen` | `RawScreen`、`RawRect`、`flip()`、`select()`、`union()`、`fallback_area()`、`work_area()`、`desktop()`、`probe()` | 监视器几何：objc2 读 `NSScreen`（含 `visibleFrame`，即去掉菜单栏与 Dock 的那块）、Cocoa 原点翻转、按窗口选屏；**两条口径分工**——`desktop()` 是所有屏可见区的**并集**，停靠/吸附/唤出用它（桌面外缘才是边的定义，双屏接缝不是，缺陷 D-8）；`work_area()` 是窗口所在**单屏**可用区，只给离屏夹回（FR-3.6）用。非 macOS 与探针失败时回退 `fallback_area`（warn-once，不刷屏） | FR-3.5、FR-3.6、FR-5.3 | ✅ P5 |
| `system::window` | `ns_window()`（仅 macOS） | 从 `eframe::Frame` 拿到那块 `NSWindow` 的**唯一一条路**：`winit_window()` → `window_handle()` → `RawWindowHandle::AppKit(ns_view)` → `NSView::window()`，入口先 `MainThreadMarker::new()?` 卡住线程。`unsafe` 只在这一层（`ns_view` 由 winit 从 `contentView` 取出，生命周期归窗口）。失焦半透明与标题栏绿键都要问原生窗口，两处各写一份就会漂（缺陷 D-9 的同一课） | FR-3.7、FR-3.2 | ✅ P5 |
| `system::alpha` | `set()` | 整窗不透明度：`system::window::ns_window` 取窗口，`NSWindow.setAlphaValue` 写档位，返回是否成功。全量窗固定 1.0、简洁窗按 `unfocused_alpha`（FR-3.7，2026-10-28 从"只染窗内底色"改成整窗原生） | FR-3.7 | ✅ P5 |
| `system::titlebar` | `sync()`、`Outcome`、`mark_click()`、`take_click()` | 标题栏绿键改道（2026-10-28 变更 G，用户"悬浮窗更换全量的按钮可以不要，将标题栏最大化功能替换为切换全量"）：`standardWindowButton(ZoomButton)` 取那颗按钮，`define_class!` 注册一个只做"记一次点击"的 `NSObject` 子类当新 target，`action` 换成 `iceToggleFullFromTitleBar:`。**每帧从按钮本身读 `action()`/`target()` 决定改道还是还原**（AppKit 会在成为 key 窗口等路径上重新接线，缓存"我接过了"会悄悄失效）；`Retained<ZoomTarget>` 存在线程局部 `WIRED` 里，因为 `NSControl.target` 是 `unsafe_unretained`；改道时一并重写 tooltip（行为变了提示不能还说"缩放"）。点击经 `CLICKED` 原子量交给帧循环，落地仍是 `Effects::toggle_mode`，与 `Cmd+Shift+M` 同一条路径 | FR-3.2、FR-3.3 | ✅ P5 |
| `system::dblclick` | `sync()`、`Outcome`、`note_double_click()`、`take_double_click()`、`title_band()`、`in_band()`、`EDGE_PAD` | 标题栏双击 = 铺满/还原（任务 #25，用户"双击标题栏：铺满整屏 + 再双击还原"）。**为什么不是 `NSTitlebarView` 的命中测试**：那条路径要子类化 `window.contentView().superview()` 并覆写 `mouseDown:`，会掐断窗口原生拖动；`NSWindow.titlebarView` 在 objc2-app-kit 0.3.2 里**根本没绑定**，公开 API 中唯一既能看见又能取消派发的位置是 `+[NSEvent addLocalMonitorForEventsMatchingMask:handler:]`——返回 `nil` 即吞掉这一次事件。**只吞双击**：`clickCount() >= 2` 且落进 `title_band()` 算出的那条带（窗口内坐标，y 从 AppKit 的左下原点起算），单击与内容区一律原样放行。带的左右边界由 `standardWindowButton(...).frame()` 折出红绿灯的最右缘、右端只留一道 `EDGE_PAD`（2026-10-29 图钉从原生标题栏挪进界面工具条以后，栏里右半再没有会收点击的东西）；宽度取不到就返回 `None`，`in_band` 同时拒绝退化带（`left < right && bottom < top` 不成立时一个像素都不吞）——**判据是 fail-closed**：拿不准就不吞，宁可由系统去做它默认的最小化。`WATCHED` 存的是被监听那块 `NSWindow` 的指针，换了窗口（重建/换形态时 winit 换窗口）就重新挂；`RcBlock` 由我们自己在线程局部里持有，因为这条 API **不 retain** block，出了作用域就是野指针。只在**全量形态**安装（简洁窗那份几何属于吸附/停靠，双击铺满会把它整屏化、热区就没了），卸载时 `removeMonitor` 并把 `WATCHED` 归零。`Outcome::Unavailable`（拿不到窗口/挂不上 monitor）时什么都不改，系统默认行为原样保留。**已知代价（写进 UI 设计）**：双击之后立刻按住拖动这条路失效——第二次按下的那一下已被吞掉，AppKit 的拖拽循环没有开始。**无头测不到原生那半**，能锁的只有带几何与"一次双击只交接一次" | FR-3.2、FR-3.6 | ✅ P5 |
| `system::mouse` | `left_button_down()` | **松手的现场状态**（2026-10-28 缺陷 D-16）：`+[NSEvent pressedMouseButtons]` 读窗口服务器维护的硬件按键掩码，取位 0（左键）。原生拖拽走的是 AppKit 的嵌套事件循环，鼠标抬起那一下**不投递给本应用的 view**，所以 egui 的 `pointer.any_down()` 在整段拖动期间是旧值、判不出松手——只能问硬件。返回 `Option`：`None` = 问不出来（非 macOS、或不在主线程被 `MainThreadMarker::new()?` 挡下），调用方 SHALL 把它当"不知道"并退回纯时间判据，**不许因为问不到就永远不吸附**。无需辅助功能/屏幕录制权限 | FR-3.5、REQ-003 | ✅ P5 |
| `system::hotkeys` | `action_of()` | 应用内快捷键分类：全局 / 窗口 / 需焦点；按住不重复触发 | FR-5.1 | ✅ P4 |
| `system::secrets` | `read()`、`write()`、`delete()`、`fingerprint()` | 钥匙串读写与指纹（长度 + 哈希前 8 位）；错误文本不回显密钥字节 | FR-4.2、NFR-3 | ✅ P4 |
| `system::single_instance` | `acquire()`、`InstanceLock`、`AcquireError` | PID 锁：陈旧锁（PID 已死/内容损坏/PID 被回收）接管，释放时只删 PID 与自己相符的那一份 | 需求 §5 第 5 条 | ✅ P4（Q-001 裁定归本阶段） |
| `stub`（仅测试） | `StubEngine`、`StubError` | 覆盖全部 11 个 `ProviderError` 变体的假引擎，让界面层测试不碰网络 | — | ✅ P4 |

## 3. 主数据流

### 3.1 文本翻译（FR-1.1~1.8）

```
用户输入 ──► UiMode 分支渲染 ──► [触发] Enter/按钮/失焦/剪贴板
                                        │
                                        ▼
                            AppState::submit()
                              │ 生成 42 位 request_id，旧结果标记 stale
                              ▼
                       JobScheduler ──spawn──► Translator::translate(kind, req)
                                                  │
                                     ┌────────────┴────────────┐
                                  命中缓存                    未命中
                                     │                          │
                                     │            protect(text) → 占位符
                                     │                          │
                                     │            engine.translate()  ── HTTP ──► 服务
                                     │                          │
                                     │            restore(占位符) ← 校验未篡改
                                     │                          │
                                     └────────────┬─────────────┘
                                                  ▼
                                    JobMsg::Translated{id, result}
                                                  │  channel → ctx.request_repaint()
                                                  ▼
                                     渲染译文 + 检测语言回显 + 写历史
```

要点：
- `request_id` 单调递增，回投消息 id 小于当前 id 时**丢弃**，防止"慢请求覆盖快请求"。
- 引擎切换（FR-1.3）视为新提交，用同一源文重新 submit，而不是复用旧结果。
- `protect` 失败（模型把占位符改掉或丢掉，`restore` 返回的 `lost` 非空）时报 `ProviderError::Decode` 失败，**不返回残缺译文**；早期设想的"降级为不保护直接返回原文"会静默返回一段被改写的文本，被 CHG-002 spec 的 REQ-003 否掉（依据见 `features/CHG-002/spec.md` 与 `ADR-004` 决策 3）。

### 3.2 图片翻译（FR-2.1~2.9）

```
拖拽/粘贴/选文件 ──► 解码为 RgbaImage（尺寸上限校验）
        │
        ▼  spawn_blocking
   OcrEngine::recognize ──► 行观察 ──► layout 合并 ──► OcrDocument{blocks}
        │                                              │
        ▼                                              ▼
  列表可编辑（FR-2.3）──────── 用户修正 ────────► 逐块/整篇送 Translator
        │
        ▼  对每个块：bg::estimate → 方差判定 → CompositeOptions
   Renderer::compose ──► Replaced | Overlay（混合输出，块级决定）
        │
        ▼
   对比视图（原图 | 结果）──► 保存 PNG / 复制图片
```

阶段状态机（FR-2.9）：`Idle → Decoding → Recognizing → Translating → Compositing → Done | Failed(reason) | Cancelled`。取消用 `CancellationToken`（`tokio_util`）在块边界检查，不做中途强杀，避免留下半成品图。

### 3.3 模式与停靠（FR-3.2~3.7）

```
[全量] --Cmd+Shift+M / 按钮--> [简洁]
   │                              │  进入时一次性下发：
   │  ViewportCommand:            │  Decorations(true), Resizable(true),
   │  InnerSize, WindowLevel      │  MinInnerSize=(400,160),
   │  Position, Visible           │  InnerSize=(460,160), OuterPosition(compact_rect)
   │                              │  └─ 第一段宽限期到还没落定：resend_commands 只补发
   │                              │     一组 InnerSize+OuterPosition，第二轮到期什么都不采纳
   ▼                              ▼
 拖动定案 ──► ModeController.snap(rect, desktop)   ← 两条路径都要走到这里
   ▲   ├─ 窗内把手：按下发 StartDrag（未聚焦先 Focus），松手即定案
   │   └─ 原生标题栏：AppKit 自己搬窗口，egui 收不到 StartDrag 也收不到松手
   │        └─ app::observe_native_move 观测几何跳变（静止几帧后出现位移）arm DragSettler，
   │           app::settle_drag 每帧先问 system::mouse::left_button_down()：
   │             还按着 → DragSettler::postpone 重新起算，这一帧一个结论都不下（D-16）
   │             问不到 → 退回纯时间判据；静止满 DRAG_SETTLE 后走同一条 apply_snap + 落盘；
   │             自己下发的移动由 commanded 排除
   │
   └── 边界矩形 = screen::desktop()（所有屏可用区的并集，D-8）；离屏校验才用单屏 work_area()
                 │ 距**桌面**左或右外缘 < snap_threshold_px ?（上/下不吸附，两屏接缝也不吸）
                 ├─ 是 → 停靠：x = 边 - (+/−)dock_sliver_px，藏到边外只留 3px 热区，记录 DockEdge
                 │        └─ 指针进入那 3px → reveal_rect 整窗唤出；离开 ≥500ms 收回（按住/打字不收回，收回后置闸门）
                 └─ 否 → DockEdge::None，自由浮动；已停靠时按「相对停靠锚点沿停靠轴向内的位移 ≥ 阈值」解除
              ──► 写回 Config.ui.{mode,full_rect|compact_rect,docked_edge} ──► 落盘
```

- 吸附计算是 `ice-app::mode` 的纯函数：输入（窗口矩形、**边界矩形**、阈值、热区，拖离那条再加停靠边与停靠锚点，唤出那条再加指针位置与闸门状态）→ 输出（目标矩形 + 停靠状态）。因此可以被单测覆盖，不需要真的拖动窗口。**边界矩形由调用方注入**，这就是两条口径的分界点：停靠/吸附/唤出传 `system::screen::desktop()`（所有屏可见区的并集），离屏夹回传 `system::screen::work_area()`（窗口所在单屏）——按单屏算会把两块并排放的屏中间那条接缝当成可停靠边，窗口藏进接缝后热区收不到悬停、`released()` 也量不出向内位移，永远解不了停靠（缺陷 **D-8**，2026-10-27 走查）。
- **停靠态的窗口层级**（2026-10-23 v0.5 / Q-012）：不论用户的 `always_on_top` 为何值，停靠（收起或唤出）期间一律升到置顶层级，解除停靠后交回置顶偏好决定。原因是那 3px 热区必须能收到指针事件，否则"鼠标放上去自动出来"在机制上不成立（真机实证：层级为普通窗口时藏出去的窄边被其他应用盖住，用户"再也找不到"）。**事实更正**：egui 0.37 的 `WindowLevel` 只有 `Normal | AlwaysOnBottom | AlwaysOnTop` 三个变体（`egui-0.37.0/src/viewport.rs:969-974`），**没有** `Floating`；本包用 `WindowLevel::AlwaysOnTop` 实现，`egui-winit-0.37.0/src/lib.rs:2086-2090` 把它映射成 winit 的 `WindowLevel::AlwaysOnTop`，AppKit 侧落到的正是 `NSWindow` 的 `.floating` 层级，所以行为与需求说的"浮层"一致，只是 API 名字不同。
- **唤出/收回只是运行时状态**（`dock_shown`、`dock_left_at`、`dock_latched`），不新增配置字段、不持久化；`docked_edge` 不因唤出而改变。指针在不在窗口内取 `ctx.input(|i| i.pointer.hover_pos()).is_some()`。**事实更正**：egui 0.37 的 `RawInput` 没有 `pointer_in_window` 字段，winit 的鼠标进入/离开事件在 egui 侧就是 `Event::PointerMoved` / `Event::PointerGone`，因此"指针离开"的判据是 `hover_pos()` 变成 `None`。
- **启动与屏幕变化的离屏校验**（FR-3.6）：分两支。**停靠态**（`docked_edge ∈ {Left, Right}`）按设计就有 `w − sliver` 在可用区外（左停靠必然 x 为负），不能拿整窗判据判它越界——那等于每次启动都替用户解了停靠；由 `watch_dock` 的第一帧 `land_dock` 按**当前**可用区重算收起到位（屏幕拔掉时自然落到还在的那块屏的边上）。**非停靠态**由 `IceApp::rescue_offscreen` 在拿到真实几何的第一帧判一次：整块在可用区外就 `fit_into_area` 夹回并立即落盘（判一次是因为 `screen::work_area` 要走 NSScreen）。`ice_core::WindowRect::centered_in` 作为兜底纯函数保留（当前生产路径未用到它）。
- **简洁窗的拖动**（FR-3.2，2026-10-27 按 CHG-004 Q-013 重写）：简洁窗与全量窗一样带原生标题栏（`Decorations(true)`），主拖动路径就是抓标题栏，由 AppKit 搬窗口；窗内保留两处把手（**左排控件与钉右的那格之间那块空白**——钉右那格在 2026-10-28 变更 F 是 `退出`、**变更 J 起是主题滑块**；再往前是"动作行行尾空白"、译文区尾部 8px），按下时下发 `ViewportCommand::StartDrag`，同样不自算鼠标增量。v0.5 及以前那套"无边框 + 顶部 10px 把手条"作废，把手条随装饰的引入删除（用户原话："已经有外框了，里面原来用于移动的小横岗组件就可以移除了"）。**代价与补法**：原生标题栏那条路径不产生任何 `StartDrag`，应用侧只看到几何在动，而吸附、停靠、拖离全挂在 `DragSettler` 的静止判定上——"把手按下即 arm"在这条路径上根本不会发生。因此 `app::observe_native_move` 补上 arm：连续 `STILL_BEFORE_DRAG` 帧静止后出现位移即 arm，静止满 `DRAG_SETTLE` 后走与松手同一条 `apply_snap` + 落盘；armed 而未静止时续一帧 `request_repaint_after(DRAG_SETTLE)`，否则原生拖完后事件循环没有再画的理由。**误判的代价比漏判大**（把程序自己的移动当成用户拖动，会把刚唤出到贴边位的窗口又吸回藏起来），所以四道闸门排除自发移动：`commanded`（自己下发过位置，到达目标点或连续静止前不 arm）、`pending_geometry`（形态切换的几何宽限期）、全量模式、隐藏态。原生拖拽期间 egui 收不到 pointer 事件（控制权交给 AppKit，macOS 那边进的是嵌套事件循环，鼠标抬起那一下不投递给本应用的 view），所以"松手了没有"既不能从事件流里读、也不能只看几何——**只看几何静止会把"拖到边上停住但手指没抬"判成松手，窗口当场从指针底下滑走**（缺陷 **D-16**，2026-10-28 用户原话"虽然我将其拖动到边上，但我还没有松手，他就着急往里面收入"）。补法是每帧先问一次硬件按键掩码（`system::mouse::left_button_down()`，`+[NSEvent pressedMouseButtons]`，与事件投递无关、无需权限）：报"还按着"就 `DragSettler::postpone` 把计时重新起算并跳过本帧结论；**问不到（`None`）SHALL 退回纯时间判据**，非 macOS 与无头测试因此行为不变，也不许出现"永远不吸附"。
- **两种形态的切换入口不对称**（2026-10-28 变更 G，用户"悬浮窗更换全量的按钮可以不要，将标题栏最大化功能替换为切换全量。"）：全量窗 → 简洁窗是工具条右端的「悬浮窗」按钮；简洁窗 → 全量窗**没有窗内按钮**，入口是标题栏那颗绿灯（`system::titlebar::sync` 在简洁模式的帧里把 `ZoomButton` 的 target/action 改道成"记一次点击"，帧循环 `poll_titlebar_click` 取走后置 `Effects::toggle_mode`）与 `Cmd+Shift+M`。切回全量窗时同一个函数把改道前读到的那对 target/action **原样交回** AppKit，所以那颗在 full 里仍是最大化。**任务 #25 不靠它**（实现时改的口径）：双击标题栏是另一个手势，由 `system::dblclick` 的本地事件监视器吞掉后自己下发 `InnerSize`/`OuterPosition`，铺的目标是**当屏可用区**（`screen::work_area`，即 `visibleFrame`）而不是整块 `frame`——菜单栏（窗口层 24）与 Dock（层 20）合成在我们之上，铺满整块只会让自己的那颗红绿灯被盖住（标题栏里已经没有别的控件了：图钉那颗 2026-10-29 变更 K 起画在内容区），看着像窗口坏掉而不像最大化。绿键单击的原生缩放因此一个字节都没动，改道只发生在简洁形态那一段。两条路径共用 `Action::ToggleMode` 那一条落地通道，不另开第二套切换实现；删的是按钮，不是能力。
- **标题栏双击 = 铺满可用区 ↔ 还原**（任务 #25，用户"双击标题栏：铺满整屏 + 再双击还原"，2026-10-28）：链路是 `system::dblclick::sync(frame, mode == Full)`（每帧挂/卸，`app::apply_dblclick_hook`）→ `app::poll_dblclick` 取走那一次双击 → `Effects::toggle_fill` → `app::toggle_fill` 按 `state.filled` 决定方向。**四件事是这条改动的全部风险面**：① **铺的尺寸不是可用区本身**——命令下发的是**内尺寸**，而可用区是**外框**目标，所以 `mode::fill_size` 把当帧实测的 `outer − inner`（本机 32pt 高、0 宽）减掉再发；差值取实测而非写死 32，因为装饰厚度会随 Retina 缩放因子与去原生标题栏的变化漂，写死的那次数值一旦漂就必然溢出屏幕外。两个轴各自 `max(0.0)`：负尺寸会被 `request_inner_size` 静默忽略，"命令发了窗口一动不动"比"铺小了一点"难查得多。② **还原要有源**——`mode::capture` 在 `state.filled` 为真时整帧早退，`full_rect` 因此一直保持铺满之前那组几何，它同时是 FR-3.6"记住位置尺寸"的槽和还原的目标；让整屏尺寸进槽就是把缺陷 D-6/D-10 换个入口重犯，还多赔上"重启后窗口再也不是用户上次那个大小"。形态切换（`mode::apply`）与唤回（`mode::reveal`）都清 `filled`，且 `apply` 里**必须在内部那次 `capture` 之后清**，否则整屏尺寸会先被记进目标槽。③ **铺到一个本来就等于槽里尺寸的尺寸要拒绝**（`app::fill_is_no_op`）：那种情况下 `filled` 立起、capture 停写，下一次双击"还原"回的是一整屏，用户看到的是第一下没反应。④ **两种去向都只发 `InnerSize` + `OuterPosition` 各一条**，并把即将下发的那个内尺寸一字不差记进 `PendingGeometry`（沿用 REQ-004 的三段护栏，不为铺满另开一套判定）。**只在简洁窗之外的那一侧安装**：简洁窗的位置属于吸附/停靠几何，双击把它整屏化之后那 3px 热区就没了。**已知代价**（写在 `03-ui-design.md`，走查 W-2.37 要盯）：双击之后立刻按住拖动这条路失效——第二次按下的那一下已被监视器吞掉，AppKit 的窗口拖拽循环没有开始；`Outcome::Unavailable`（拿不到 `NSWindow`、monitor 挂不上）时什么都不改，系统的"双击标题栏"设置（通常是最小化）原样生效，界面不会半死不活。
- **置顶的可见入口是那枚 egui 图钉**（FR-3.4；2026-10-28 变更 I 由用户提出"置顶不应该是文字，应该和大部分软件一样，可以在标题栏最右侧有一个类似于钉子的图标，点击置顶变成蓝色并且钉子变成垂直状态，则置顶。再点击变回来。全量界面也要有。"，**2026-10-29 变更 K 按"全量窗挪动到精简（悬浮窗）两个字左侧，悬浮窗状态下，放在（右下角）颜色切换左边"把它从原生标题栏搬进界面**）：变更 I 那一版挂在 `NSTitlebarAccessoryViewController` 里，公开 API 只有那一个槽能"既在栏内又收得到点击"，而它**只能落在标题栏右边缘**，够不到用户指定的那两个内容区位置，于是整模块删除（`system::pin`、`app::{apply_pin, poll_pin_click}`、`objc2-app-kit` 九个 feature）。现在是 `views::pin_toggle`：`allocate_space` 之后**恰好一条** `interact(rect, pin_id(salt), Sense::click())`，`draw_pin` 现画圆帽 + 横梁 + 针三段线稿（不用 SF Symbol、不用字符——回到 egui 就重新受 D-5 那条约束），两态只换实心↔空心、转不转 `PIN_TILT_DEG`、`theme::ACCENT`↔主题前景色；位置由两处各自那条 `right_to_left` 带里的**调用顺序**决定（先加的贴右缘、后加的落左边）。三条不变量：① **两处共用同一个函数**，salt 只用来分开记录矩形，"全量界面也要有"仍是这一句实现的；② **外观每帧从 `config.ui.always_on_top` 重算**，不缓存"我上次画成什么"，任何一条路径（图钉本身、`Cmd+Shift+P`、设置页那一格）改的状态都在下一次重绘里如实显示；③ **点击只置 `Effects::toggle_topmost`**，落地仍是 `app.rs::toggle_topmost` 那一条（`state.toggle_topmost()` + `mode::apply_topmost` + 提示条），**置顶在代码里仍然只有一套实现**（`the_pin_effect_and_the_hotkey_share_one_path` 顺带锁"一次点击只下发一条 `WindowLevel`"）。换进 egui 的净收益：位置、命中格宽度、两态提示文案、命令条数、"按住它不许拖窗"从无头测不到变成全部锁得住，无头剩下的只有像素观感（W-2.34）。
- **提示条一律居中、1s 淡出、不可关闭**（2026-10-28 变更 I，用户"所有的提示类弹窗都是中心弹窗500ms内淡化消失，不应该有关闭的操作和按钮。"；**2026-10-29 变更 L 只改时长**：用户"所有弹窗改为1s淡化消失吧，500ms太短了"）：`views::toasts` 用 `Area::anchor(CENTER_CENTER)` + `Order::Foreground`，`Frame::popup` 的底色/描边/投影由 `multiply_with_opacity(opacity)` 一次乘好、文字色另走 `gamma_multiply`；`opacity` 是 `TOAST_LIFETIME`（现为 1s）上的线性衰减，**同一个常量既管活多久也管淡多久**，不存在"淡完了还挂着"的中间态——那次改判因此只动了这一个数，`Toast::opacity()` 与 `expired()` 都从它推导。**`Area::interactable(false)` 是这条改动的关键**：egui 把不可交互的 layer 整块排除在命中测试之外，所以居中的浮层压在按钮上时下面那颗照样点得到——改成居中而不改交互，就等于把 P4 那颗 `×` 的拦截面积从一小块扩大到窗口正中，是不可接受的回归。代价是淡出期间必须逐帧重绘：屏幕上有提示条时 `request_repaint_after(TOAST_FADE_STEP)`（16ms，1s 整段约 60 帧——比 500ms 那一版多出一倍的重绘帧，仍只在有提示条期间），一空就回到 NFR-2 的空闲策略，TC-N-1 那条 30s cputime 采样测法不变。`confirm()`（失败态那两颗动作按钮）**不在这一条里**——它要的是用户做一个决定，自动消失等于替用户答"否"。启动时那条配置损坏的黄色提示（`ConfigParse`）因此也在这 1s 内淡掉：它本来就只说一次，界面上不留下第二次读它的地方，登记在 `07-acceptance.md`。
- **形态切换的几何下发**（FR-3.9，2026-10-22 由"形变动画"改为瞬时）：切换时在同一帧内一次性下发 `Decorations(目标形态)` → `InnerSize(目标尺寸)` → `OuterPosition(目标位置)`，没有 `Morph` 状态、没有逐帧补间，只有一组一次性命令。`Decorations` 与 `WindowLevel` 是原生属性，本来就只能整段设一次（逐帧设会闪烁）。下发的是**请求**而非同步生效（`InnerSize` 走 `request_inner_size`，异步），因此切换完的下一帧必须用 `mode::capture` 回读真实几何再落盘，不能拿目标值当已生效值。**但 capture 在"命令已发、窗口还没跟上"的过渡期里读到的仍是旧形态的尺寸**（缺陷 **D-6**，2026-10-23 走查取证）：那条过渡几何会被写进目标形态的配置槽，而尺寸命令只在切换那一刻发一次，于是简洁窗永远保持全量的大小。修法不是不写，而是加一个宽限期：切换时记下 `pending_geometry`（目标尺寸 + 时刻），`MODE_GEOMETRY_GRACE`（1s）内 capture 只写位置轴、不写尺寸轴，直到实测尺寸命中目标（±2px）或宽限期到期为止。
  - **三段护栏**（2026-10-27 由缺陷 **D-10** 升级为三段，v0.7 / REQ-004）：只有"宽限期到期即放行"是不够的——原生窗口没跟上的那次切换，到期后会把大窗尺寸照抄进 `compact_rect`，而尺寸命令此后再无来源，`measured == target` 每帧命中，坏状态**自我维持**（盘上两槽四项全等 984×632 就是它的指纹）。三段是：① **宽限期内**（第一段）尺寸轴静默，位置轴照写；② **第一段到期**不再直接放行，`mode::resend_commands` 只补发一组 `InnerSize` + `OuterPosition`（`take_retry` 保证恰好一次），把护栏交回给第二轮；③ **第二段到期**采纳**什么都不写**——把当时实测尺寸存为 `baseline` 并 WARN（只报两个尺寸，不报路径），此后尺寸轴只在"与 `baseline` 的欧氏距离 > `PENDING_SIZE_TOLERANCE`（2px）"时才写回，也就是"用户真的手动改过尺寸"，一次抖动或同一尺寸反复上报都保持静默。`PendingGeometry::set()`（新一轮切换）无条件清 `baseline`。两段加起来的判据由测试 `the_two_slots_never_end_up_equal_after_a_failed_switch` 锁住。
  - **与 Spec 的两处实现偏离**（2026-10-23 记，均已在 `features/CHG-004/evidence.md` 取证）：① 切换那一帧仍然调用一次 `ctx.request_repaint()`。Spec 写的是"切换后不再 `request_repaint()`"，针对的是**逐帧续期**的补间；但目标内尺寸若恰好等于当前尺寸，winit 不会发 `Resized`，没有这一次显式重绘，当前帧画的还是上一形态的布局（切换后第一眼就是错的）。补间已取消，因此这里只有**一次**重绘，之后回到"提示条到期与结果到达才重绘"的空闲策略，NFR-2 不受影响。② `capture` 在几何变化时立一个 `CONFIG_SAVE_INTERVAL`（500ms）的延时重绘，这是 REQ-009 要求的落盘钟，不属于切换的续期。
  - **窗口收起时触发切换**（`state.hidden`）只把目标形态记进配置，一条几何命令都不发，也不把窗口叫回来；从 Dock 恢复时 `reveal` 会按 `config.ui.mode` 整套重发几何。
  - **「恢复默认窗口尺寸」不走 `mode::apply`**（REQ-011）：那颗按钮的语义是把两个尺寸槽改回默认值并立即落盘，而 `mode::apply` 开头会 `capture` 当前真实几何——坏尺寸正是用户要点按钮清除的东西，用它等于把污染又抄一遍。实现是纯函数 `app::reset_window_sizes`：写默认尺寸 + 清停靠、发**一组**几何命令、给 `pending_geometry` 立护栏。位置轴由 `restored_rect` 决定——已知可用区时在其内居中，取不到（首帧还没走 NSScreen）时保留原 x/y，不为了一次复位把窗口甩到屏幕外。
- **主题滑块是两种形态共用的一个函数**（NFR-4、FR-4.1，2026-10-28 变更 J，用户"悬浮时放在右下做成一个类似于开关左右推动的效果……而且全量界面也要有这个开关，不需要提示，看到的人都懂。也在右下角。"）：`views::theme_switch(ui, state, salt)` 一处实现，简洁窗动作行的 `right_to_left` 子带和全量窗底栏的 `right_to_left` 子带各调一次，`salt`（`"compact"` / `"full"`）只用来把命中矩形分开记录（`Context::data` 的临时项，测试经 `recorded_theme_switch` 读）。**为什么必须共用**：用户要的是"两处都有这颗开关"，两处各画一遍就会在下一轮走查里漂成两个样子——这与 K-03/K-09 那两条"共用同一个原生窗口"是同一课。**控件里不含文字**，所以它不受字号缩放影响、占位恒为 `THEME_SWITCH_W`=40px，全量底栏那侧的 `bar_layout` 因此要先从可用宽里扣掉它（滑块不参与"先降一档再换行"，退让全由按钮承担）。**命中只有 `Sense::click_and_drag()` 一条**，落点横向位置经 `theme_at` 反解成档位；轨道占位用 `allocate_space` 而不是 `allocate_at_least`——后者内部自带一次 `interact`（`allocate_response` = `allocate_space` + `interact`），再自己调一次 `interact` 会让同一枚指针被两个响应者各消费一遍，且那两个响应者的 `Id` 不同。**没有悬停提示**是用户明说的，不是漏配。**`Theme::next()` 随之删除**：点三下的循环式子在滑块上没有对应动作，领域层不留没人调的换档函数（三态映射改由视图侧的 `theme_slot`/`theme_at` 这一对纯函数承担，测试锁死两端与中间、以及 0.25/0.75 的迟滞边界）。
- **滚动容器与动作栏的边界**（2026-10-27 按缺陷 **D-9** 补，v0.7 按 **REQ-013** 改法，两窗共用）：`views::full::scroll_area()` 是两栏与简洁窗译文的同一个构造，里面必须 `.min_scrolled_height(0.0)`——egui 的 `ScrollArea` 有个默认 64px 的 `min_scrolled_size`（`scroll_area.rs:469`，在 `:856` 以 `inner_size[d].max(…)` 生效），矮栏里它把视口抬到 64px 并画到框外，而最大滚动偏移按这个虚高视口算，结果**最后十几像素怎么滚都滚不进来**（简洁窗 130px 高时实测视口底边 115.9 越出译文框底边 103.0、压在动作行顶边 108.0 之上）。"滚不动"的风险不由这条默认值兜，由调用方的栏高守卫兜（`views::compact::MIN_BODY_HEIGHT`）。全量窗这边 v0.7 起换了结构：右下角那条动作条原本是**浮层**、不参与布局，正文末尾得按 `bar_cover()` 预留一条与条子等高的空白带，而"预留公式"与"实画公式"一旦不同步就又把最后一行盖住——现在它是 `Panel::bottom("full_bottom_bar")`，**自占一行**，两栏的可用高在构造时就已被它扣掉，所以这条不再需要任何预留，`bar_cover()` 与它的断言整套作废。保证从"算得准"变成"结构上盖不到"（实测余量 36px，由 `the_last_line_stops_above_the_bottom_bar` 断言"可视区底边在底栏上沿之上"锁住）。
  - 两条一起画的教训（v0.7 踩过的坑，写进 `05-dev-guide.md`）：① 自占高度的面板里必须 `ui.advance_cursor_after_rect(rect)`，否则 `Panel` 量到 26×10 直接塌成一条；② `Ui::columns` 不看光标、按整行宽度均分，和上面的定高行会重叠 5px，要用 `horizontal_top` + `allocate_ui_with_layout`。**凡是不参与布局的覆盖式浮层，优先考虑改成自占一条带**——预留公式与绘制公式分处两地，就是下一次同类缺陷的来源。
- **几何持久化时机**（P1 实测：SIGTERM/强杀不走 `on_exit`，只靠退出保存会丢位置）：在①拖动结束（吸附判定完成时）②形态切换的命令**落定**的那一帧③关窗/退出时 三处强制写盘，另加④`capture` 每次几何变化都续一帧 500ms 的重绘钟，保证用户最后停下的位置一定会落盘。均为小文件（<1KB）原子替换，代价可忽略。

## 4. 并发模型

| 场景 | 机制 | 原因 |
|---|---|---|
| HTTP 翻译 | tokio `spawn`（多线程 runtime，由 `ice-app` 持有） | 不阻塞渲染 |
| 长文分段（FR-1.9） | `providers::parallel::map_segments`：`JoinSet` + 定长结果槽，在飞窗口 ≤ `MAX_IN_FLIGHT`(4) | 段间无依赖，串行只是把往返时间相加。窗口设上限而不是全量并发，是为了不打爆机翻免费额度、不诱发 AI 侧 429；结果写回**槽位**而不是 push 进 Vec，所以谁先答完都不影响拼接顺序；任一段失败 `abort_all()` 后收摊，不再继续花额度 |
| OCR / 图像合成 / 字体加载 | `spawn_blocking` | Vision 与像素操作是 CPU 阻塞调用 |
| 结果回投 UI | `mpsc::channel(CHANNEL_CAPACITY)` + `ctx.request_repaint()` | eframe 事件驱动，必须主动唤醒。**有界**而非设计里写的 `unbounded`：无界队列在结果积压时会悄悄吃内存，有界则让背压可见 |
| 钥匙串读取 | `spawn_blocking` + `mpsc::channel(1)`，建窗**之后**才发起 | macOS 对不由本程序写入的条目会弹授权框，`get_password()` 一直阻塞到用户点完。放在建窗之前窗口根本出不来，表现为"启动卡死"（P4 真机踩到并修掉）。线程内必须用 `blocking_send`：`send` 是 async 的，在 `spawn_blocking` 里只会造出一个没人 await 的 future |
| 引擎实例共享 | `Arc<dyn TranslationEngine>` | `reqwest::Client` 内部连接池，可复用 |
| 缓存 | `Mutex<LruCache>` | 临界区极小，不值得无锁结构 |
| 取消 | `CancellationToken`（块边界） | 强杀阻塞任务会留下半文件 |

约束：**UI 线程绝不 await 网络**，也绝不做像素运算。

## 5. 错误传播与提示映射

| 层 | 错误类型 | 到界面的映射 |
|---|---|---|
| providers | `ProviderError::Unauthorized` | "API Key 无效，请重新保存" + 跳转设置页按钮 |
| | `NotConfigured` | "AI 服务未配置" + 跳转设置页按钮 |
| | `RateLimited` | "被限流，稍后重试"，不禁用按钮 |
| | `Timeout(ms, retries)` | "超时（已重试 n 次）"，提供"改用机翻"动作 |
| | `Empty` | "服务返回空译文"，保留原文 |
| | `SameLanguage(lang)` | "源语言与目标语言相同（都是简体中文），不需要翻译；请换一个目标语言"，不给动作按钮（换目标语言只能由用户决定）。**P4 新增**：机翻端点对同语言对必拒，实测回 HTTP 200 + `responseStatus: 403` + 英文 `PLEASE SELECT TWO DISTINCT LANGUAGES`，因此在发请求前就地判定。**CHG-004 将改这条的上游行为**（用户 2026-10-21 决定）：源语言为"自动检测"且检测出的语言==目标语言时，不再报错，而是把目标语言自动翻转（英语 ↔ 简体中文）、持久写回、提示条说明并直接出译文；届时本变体只保留给"用户显式选了同一个源与目标"的情形。见 `06-test-cases.md` TC-1.2-c |
| vision | `UnsupportedPlatform` | 图片页签禁用并说明"当前平台不支持" |
| | `NoTextFound` | 提示"未发现文字"，不算失败 |
| | `Font(msg)` | 结果区顶部警示条："缺字形，已回退" |
| core | `ConfigParse` | 启动时黄色提示条（应用仍能起来，FR-4.3）。**2026-10-28 变更 I 之后它也只活一条 `TOAST_LIFETIME`**（500ms，2026-10-29 变更 L 起为 1s）：损坏只可能发生在启动那一次，界面上不存在第二次读到它的地方，这条后果已登记在 `07-acceptance.md` |

规则：错误一律向上带**人话文本**（thiserror `#[error]`），UI 不做 `match` 猜含义；只有需要"给出动作按钮"的分类才在 UI 层 `match` 具体变体。

## 6. 配置生命周期

```
main() ──► Config::load_at() ──► (cfg, Option<reason>)
             │ 不存在 → 默认值           │ 损坏 → 默认值 + reason（界面提示）
             ▼
        cfg.sanitized() ──► 旧 glossary.toml 一次性只读导入为 cfg 的 [[term]]（导入后不删用户文件）
             ▼
        构建 AppState / 装配 Translator（术语同帧推给派发器，不等用户改一次才生效）
             │
             │ 运行中：任何改变即时更新内存副本并标脏（config_dirty）
             │ 钥匙串探测回来后按事实对齐 ai.key_configured（钥匙串是事实来源，config 里只是缓存标记）
             ▼
        脏标记 + 退避 500ms 落盘；Cmd+Q 时强制落盘，落盘成功后才释放单实例锁（原子替换）
```

`recent_targets` 在切换目标语言时更新（`remember_target`），最多 5 条（FR-1.4）。

变更 H 那四个排版字段（`source_font_scale`、`translation_font_scale`、`letter_spacing`、`line_spacing`）全部 `#[serde(default)]`，所以用户已经在用的 `config.toml` 不会因为多字段而读失败；`sanitized()` 只对**有限数**接受越界夹取，NaN/Inf 退回默认档——手改配置文件不许把界面熬成看不见的字。

**与封存设计的两处偏差**：① 设计写"当帧结束就 save"，实现改为退避 500ms——字号滑块这类连续控件每帧都会标脏，逐帧写盘会把原子替换变成每帧一次 rename；② 设计没有"钥匙串探测回来再对齐 `key_configured`"这一步，因为当时假定 Key 在建窗前就读到了（该假定已被 P4 真机推翻，见第 4 节）。两者都不改任何 REQ。

## 7. 需求 → 模块映射（节选，完整矩阵见 `traceability.md`）

| 需求 | 主责模块 | 协作 |
|---|---|---|
| FR-1.3 双引擎 | `providers::dispatcher` | `providers::{machine,ai}` |
| FR-1.6 内容保护 | `providers::protect` | — |
| FR-1.9 长文分段提速 | `providers::parallel`（窗口与按序拼回） | `providers::{ai,machine}`、`protect::chunk_chars`、`core::config::AiProvider::thinking_off` |
| FR-2.2 本地 OCR | `vision::{ocr,vision_sys}` | `core::document` |
| FR-2.4/2.5 回填与覆盖层 | `vision::composite` | `vision::bg` |
| FR-3.4 置顶 | `app::mode` | `core::config::UiConfig` |
| FR-3.5 边缘吸附 | `app::mode::snap`（纯函数） | `core::config::WindowRect` |
| FR-4.2 钥匙串 | `app::system::secrets` | `core::config` 常量、`app::spawn_key_probe` |
| 需求 §5 第 5 条 单实例 | `app::system::single_instance` | `app::IceApp`（退出时释放）、`main`（建窗前获取） |
| FR-5.1 快捷键 | `app::system::hotkeys` | `app::state`（动作落地） |
| NFR-6 分层 | 全 workspace | `cargo tree` 校验 |

## 8. 设计自检（P2 验收）

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 每条 Must 需求有主责模块 | ✅ 见第 7 节与 `traceability.md` |
| 2 | 无"设计了但没人调用"的模块 | ✅ 骨架只落地被调用的接口；回写侧的中间抽象（块级排版计划等）等 P7 有真实文字测量需求时再定义，不预先设计 |
| 3 | 并发与取消语义明确 | ✅ 第 4 节 |
| 4 | 错误到界面的映射穷举 | ✅ 第 5 节，覆盖全部已定义变体 |
| 5 | 关键取舍有 ADR | ✅ ADR-001/002/003/005/006；ADR-004（机翻端点）当时留待 P3 实测，P3 已落档 |
| 6 | 可测试性：纯函数与 IO 分离 | ✅ 吸附计算、几何合并、缓存、术语替换均为纯逻辑 |
| 7 | 模块表与实际代码一致 | ✅ 逐 crate 核对 `pub mod` 清单，发现并补登记 `ice-providers::request`；其余计划模块按阶段落地（见下） |

**验收结论（P2）**：**签发**。骨架编译通过、53 项单测通过、clippy 无警告，依赖方向由 `cargo tree` 核实（`ice-core` 依赖树内无 egui/eframe/reqwest/image），ADR-001 的单窗口双形态已在 macOS 真机按预期改变窗口形态（`evidence/P1-03-window-geometry.txt`）。

**P4 实现后的校正（2026-10-20）**：本节表格是 P2 当时的结论，保留不改。P4 落地后按其校正了三处，均已回写正文：① 第 2 节 `ice-app` 模块表按实际代码重列，补 `system::single_instance`（Q-001 裁定归 P4）、`theme`、`views::{mod,history}`、`stub`，并说明设计里的四个 `*View` 结构体最终落成自由函数 + 单一 `AppState`；② 第 4 节结果回投是有界通道，并新增"钥匙串读取必须在建窗之后、在 `spawn_blocking` 里"一行；③ 第 5 节错误映射补 `SameLanguage` 变体（11 个变体仍穷举，自检项 4 继续成立）。第 6 节补记两处与封存设计的偏差（退避落盘、探测后对齐 `key_configured`）。以上均不改动任何 REQ，完整偏差清单见 `07-acceptance.md` §5。

落地进度与本文的对应关系（本文描述 v1.0 全貌，不要求 P1 就全部存在）：

| 状态 | 模块 |
|---|---|
| 已实现并有单测 | `ice-core` 的 7 个业务模块（45 项，含 `config` 的供应商预设与有效值规则）；`ice-providers` 的 `engine`/`request`/`cache`/`dispatcher`/`error`/`machine`/`ai`/`protect`（51 项，HTTP 行为由本地假服务覆盖；`dispatcher` 用 mock 引擎覆盖选路、缓存与可用性）；`ice-app` 的窗口形态与置顶控制（3 项无头断言） |
| 已实现、靠 `#[ignore]` 实测 | `ice-providers/tests/live.rs`（3 项：机翻短文本、机翻长文本切块、AI 真实通道）——需要真实网络与钥匙串里的 Key，不进默认测试集 |
| 只有接口与桩，尚无单测 | `ice-vision` 的 `ocr`/`composite`/`error`（P6/P7 填充并补测） |
| P4/P5 落地（P5 = CHG-004，2026-10-27 收口） | `state`、`mode`（含吸附/停靠/悬停唤出/拖动静止判定/失焦降透明）、`views::{mod,full,history,settings,compact}`、`jobs`、`system::{clipboard（含 `Watcher`）,hotkeys,secrets,single_instance,screen}`；`views::image` 仍是占位，接 OCR 在 P6 |
| P6/P7 落地 | `vision_sys`、`layout`、`bg`、`font` |
