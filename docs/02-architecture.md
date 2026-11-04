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
| `machine` | `MachineTranslateEngine`、`MACHINE_QUERY_CHARS` | 零配置机翻通道（MyMemory），单次查询 450 字符上限、超时与网络类重试 ≤2；超上限切块后**并行**在飞（`parallel`）并按块序拼回。**响应里 `responseData`、`translatedText`、`responseDetails` 三处按"可以是 `null`"建模**（缺陷 D-20，2026-11-04 实测）：这一家端点对某些语言对给 `responseStatus:200` + `"translatedText":null`、配额用尽给 `"responseData":null`，而 `#[serde(default)]` 只兜"键不存在"、**不兜显式的 `null`**——类型写成 `String` 时整份响应被判成"不是预期 JSON"，用户看到的是 serde 的类型报错，真正的原因（额度用完 / 这一对语言没内容）就在同一份响应的另一格里。`responseStatus` 既可能是数字也可能是字符串；没有可识别状态时按解析失败，不猜成功 | FR-1.3、FR-1.7、FR-1.9、NFR-1 |
| `ai` | `AiEngine` | OpenAI 兼容 chat 通道，提示词构造、分段与上下文续译、拼回；按 `AiProvider::thinking_off()` 塞各家拼法的关闭思考字段；分段**并行**在飞 | FR-1.3、FR-1.9、NFR-1 |
| `parallel` | `map_segments()`、`MAX_IN_FLIGHT` | 分段并行的唯一实现：`tokio::task::JoinSet` + 定长结果槽，在飞窗口 ≤ 4，**按段序**返回、任一段失败即整体失败并 `abort_all()` 取消其余（不私有，只对本 crate 的两个通道开放） | FR-1.9、NFR-1 |
| `protect` | `ProtectedText`、`Restored`、`protect()`/`restore()`、`chunk_chars()` | URL/邮箱/代码块/标识符/带单位数字的占位符保护，以及"取额度内**最后一个**句号或换行断开、不切开占位符"的切块 | FR-1.6、FR-1.9 |
| `cache` | `TranslationCache`、`CacheKey`、`CachedValue` | LRU；AI 通道以术语表版本号加盐 | FR-1.8、FR-4.5 |
| `dispatcher` | `Translator` | 选引擎→查缓存→调用→术语后处理→回写缓存；`availability()` | FR-1.3、FR-1.7、FR-1.8 |
| `error` | `ProviderError`（10 个变体）、`retryable()`、`from_reqwest()`/`from_status()` | 网络/状态/解析错误归一化为可读种类；错误文本剔除 URL 查询串，不回显源文与 Key | FR-1.7、NFR-3 |

`Translator` 是 UI 唯一允许接触的入口。UI 不知道有几个 HTTP 端点、缓存怎么键控。

### ice-vision

> 下表在 P6 实现后（CHG-005）按实际代码校正。设计里 P2 写的 `merge_lines_into_blocks()` 与 `RowKey` 两个名字没落成——分组这件事需要先把归一化坐标换算成像素、把框架对象拆成中间态，最终拆成 `RowObservation`（观察）+ `merge_rows()`（分组）+ `flip_rect()`（坐标换算）三件；`bg`/`font` 两行仍是 P7，本包没动。验证口径随实现一起改了：`layout` 是零 IO、零 `unsafe`、零平台依赖的纯函数，REQ-004 的一条测试都不需要真机（见 docs/04 §2.1）。

| 模块 | 关键类型 | 职责 | 需求 | 状态 |
|---|---|---|---|---|
| `ocr` | `OcrEngine`（trait）、`VisionOcr`、`default_engine()`、`recognize_with_rows()`、`OcrProgress`、`MAX_EDGE_PX`、`LOW_CONFIDENCE_WARN` | 识别入口；平台能力探测。**`recognize_with_rows` 有两条编译面**：macOS 走 `vision_sys`，非 macOS 是同签名返回 `UnsupportedPlatform` 的门控版本，所以调用方跨平台不用写 `cfg`。返回的三元组把 `Vec<RowObservation>`（S1 行级观察）、`OcrDocument`（S3 合并后的块）与 `OcrProgress`（`blocks` + `elapsed_ms`）一起交出，FR-2.3 的行号溯源因此有唯一出处；耗时在本 crate 内测，界面层自己计时测到的是含读盘解码的整段任务 | FR-2.2、NFR-7 | ✅ P6 |
| `vision_sys`（macOS） | `recognize_rows()` | 对 `VNRecognizeTextRequest` 的唯一一处封装：归一化左下坐标 → 像素左上坐标、置信度与候选文本，把框架对象拆成 `RowObservation` 就收手——`unsafe`、ObjC、Core Image 全部关在这一个文件里，模块声明在 `lib.rs` 上带 `#[cfg(target_os = "macos")]`。**请求上的语言设置是两件事不是一件**（2026-11-04 变更登记 M）：`RECOGNITION_LANGUAGES` 那三条（zh-Hans/en/ja）仍写死、不开放给用户，但它从"硬门"降级成"优先提示"，因为同处打开了 `setAutomaticallyDetectsLanguage(true)`，框架据此把范围放开到它支持的全部文字。开关只在 Revision3 起生效，`revision` 本包没显式设过、取框架默认，所以这是运行时能力，换机器以 `examples/self_metric.rs` 的量值为准 | FR-2.2 | ✅ P6 |
| `layout` | `merge_rows()`、`RowObservation`、`MergeParams`、`BlockDraft`、`NormalizedRect`、`flip_rect()` | 行级观察合并为语义段。五条判据 + 两条硬上限全在 `MergeParams::DEFAULT`（对齐容差 0.6×平均**字框高**、垂直间隙 1.3×行高、水平重叠 0.4×较窄行宽、"已填满"0.85×参考宽度即断段、单块 ≤12 行 / ≤1200 字）。`BlockDraft.source_rows` 是拆分回原行的唯一依据；输入顺序不影响结果（内部按"自上而下、自左而右"重排，平局用 `row_index` 决胜） | FR-2.2、FR-2.3 | ✅ P6 |
| `import` | `admit()`、`admit_path()`、`admit_rgba()`、`Admitted`、`NOT_FOUND_TEXT`、`DECODE_TEXT` | 三条入口（路径 / 字节 / 剪贴位图）汇入同一个准入判定：边长 ≤ `MAX_EDGE_PX`、格式可解、标签留给界面回显。两条错误文案各是一个常量，界面与测试引用同一个字符串，不各写一份 | FR-2.1 | ✅ P6 |
| `error` | `VisionError`（8 个变体，含 `NoTextFound` 与 `UnsupportedPlatform`）、`VisionResult` | 视觉层错误归一化；`NoTextFound` 单独一个变体，"这张图没字"不当失败处理。P2 定的枚举本包一个没改，只是把前三类真的抛出来了——`Composite`/`Font`/`NotImplemented` 仍归 P7 | FR-2.2、FR-2.9 | ✅ P6（P2 定义） |
| `bg` | `BackgroundEstimate`、`estimate()` | 框边缘采样 → 背景色、前景色、方差（决定回填 or 覆盖层） | FR-2.4、FR-2.5、FR-2.8 | P7 |
| `composite` | `Renderer`、`CompositeOptions`、`CompositeOutput`、`TextLayout` | 擦除、排版、绘制、字体回退 | FR-2.4~2.8 | P7（接口与桩） |
| `font` | `FontSet`、`resolve()` | Core Text 枚举系统字体、CJK 回退、缺字形检测 | FR-2.7、NFR-4、R-6 | P7（未建模块） |

### ice-app

> 二进制 crate（`main.rs` 里 `mod`，非 `lib.rs`）。下表在 P4 实现后按实际代码校正：设计里的 `FullView`/`CompactView`/`ImageView`/`SettingsView` 四个结构体最终落成各视图模块里的自由函数 + `AppState`，因为界面状态只有一份、不需要每页再包一层持有者；`JobScheduler`/`ModeController` 同理落成 `JobRunner` 与 `mode` 模块的函数集。

| 模块 | 关键类型 | 职责 | 需求 | 状态 |
|---|---|---|---|---|
| `app` | `IceApp`、`Effects`、`Prepared` | 实现 `eframe::App`；持有 `AppState`、`JobRunner`、tokio runtime、历史存储、配置路径、单实例锁、钥匙串探测回收端；帧末统一执行副作用（含 `Effects::{quit, reset_geometry}` 两条来自设置页的意图）与退避落盘；原生标题栏拖动靠 `observe_native_move` 的几何跳变观测补上吸附判定；`settle_drag` 每帧**先问硬件**（`system::mouse::left_button_down()`），左键还按着就 `DragSettler::postpone` 把静止计时重新起算、这一帧一个结论都不下，问不到才按时间判（缺陷 D-16）；每帧按当前形态调 `system::{alpha, background, titlebar}::set/sync`（`apply_native_alpha`、`apply_native_background`、`apply_zoom_hook`，只在结果变了才记日志），并在帧首 `poll_titlebar_click` 把绿键那次点击兑成 `Effects::toggle_mode`；**图钉不再是原生那一条**（2026-10-29 变更 K：`apply_pin` / `poll_pin_click` / `system::pin::sync` 三个调用点连同整个模块一起删除，图钉改由 `views::pin_toggle` 在两种形态的界面里各画一格，直接置 `Effects::toggle_topmost`）；**菜单栏托盘的泵挂在 `logic`（CHG-008 v0.2）**：`pump_tray(scale, outcome, &ctx, &mut effects)` 先定档——`Unknown` 才 `install`（`scale` 来自 `frame.winit_window().scale_factor()`，选 @1x/@2x），装上之后每帧只 `sync` 核对，装不上就停在 `Unavailable` 并给一条会淡出的提示条；再把 `take_outcome()` 取到的那一个意图交回**既有**的 `run()` 分派，四条意图与快捷键、设置页、绿键落在同一批动作上。**这一句挂错钩子就等于只在窗口可见时生效**——`ui()` 在隐藏期一次都不被调（缺陷 D-21，判据与行号见 `05-dev-guide.md` §6）。**帧尾续期收成一句 `renew_repaints(&ctx)`**：`state.hidden` 为真时一次 `request_repaint` 都不发（REQ-007——隐藏期每续一次期就多跑一整个 pass，而这段时间唯一该发生的唤醒是托盘那一下自己发的 `request_repaint`），提示条心跳与淡出续期只在有观众的时段跑；`watch_visibility` 也在 `Available` 那一档整条早退，唤醒只留托盘一条路。**标题栏双击那条只在全量形态挂**（`apply_dblclick_hook` / `poll_dblclick`，任务 #25）：每帧调 `system::dblclick::sync(frame, mode == Full)`，取到一次双击置 `Effects::toggle_fill`，`toggle_fill` 据此在"铺满当帧可用区"与"还回 `full_rect` 记着的那组几何"之间往返，两种去向都只发 `InnerSize` + `OuterPosition` 各一条、并把即将下发的内尺寸记进 `PendingGeometry`；铺满期间 `state.fill` 是 `Some(那块可用区)`，`mode::capture` 因此**一个字都不写槽，但照常测量**（缺陷 D-24，见 `mode` 行）——护栏量得到、铺不到位就原样重发，槽里保住的才是还原的源；挂载结果只在变化时记日志，接不上时一次性 WARN。**图片侧四条入场（CHG-005）**：`ui` 帧里在 `jobs::drain` 之后依次 `poll_panel`（`while let` **一次读空**而不是只取一条：入口被连按两次时通道里可能压着两条，留一条在里头就要等下一帧、下下帧，而窗口收进后台后 `ui` 根本不跑。`Reply::Picked` 派发识别、`Cancelled` 什么都不改也不播报、`Failed` 给一句提示）与 `poll_drops`（`ctx.input(|i| i.raw.dropped_files)` 取第一个带路径的文件；eframe 每帧现填、不跨帧残留，所以这里不需要去重记忆）；两条都汇进 `dispatch_image`——**切到图片页不是可选项**，识别结果只有那一页画，在文本页派发一次识别而画面不动，用户看到的只是状态条上闪过一句"识别中"。`run_effects` 帧末执行三颗意图：`open_panel`（`request_open` 返回 `false` 时提示"弹不出系统文件面板，请在路径那一格里手输"）、`paste_image`（`clipboard::read_image()` → `ImageInput::Clipboard`）、`ignore_image`（`ignore_image_result()` 为真才提示"已忽略这次识别的结果"，为假不凭空说这句话）；面板的两端 `panel_tx`/`panel_rx` 由 `IceApp` 持有，回调在 ObjC 栈上、只往 `mpsc` 里塞一条消息 | 全部 | ✅ P6 |
| `main` | `prepare()`、`main()` | 启动时序：日志 → 单实例锁 → 读配置 → 导入旧术语表 → 装配引擎 → tokio → 建窗（`main()` 里 `run_native` 的创建闭包第一件事是 `fonts::install`，中文落在黑体上必须赶在第一帧排版之前）→ **后台**读钥匙串 | FR-4.1~4.3、REQ-009 | ✅ P4 |
| `state` | `AppState`、`Failure`、`FailureAction`、`Job`、`Outcome`、`Toast`、`ImageInput`、`ImageRecognition`、`RecognitionStage`、`Split`、`Confirm`、`Tone`、`TrayState`、`KeyState`、`Tab` | 源文/译文/语言对/引擎/在飞任务/失败态/图片工作区/历史查询/提示条；`request_id` 竞态丢弃。**提示条（2026-10-28 变更 I）**：`Toast::opacity()` 给线性不透明度、`expired()` 到点自己走；**没有"用户点掉一条"这条路**（`dismiss_toast` 与那颗 `×` 一起删除），`tick_toasts` 是唯一的过期通道；时长常量按代码为准是 `TOAST_LIFETIME = 1s`（2026-10-29 变更 L 从 500ms 抬上来，本文此前漏改）。**`fill: Option<Rect>`（任务 #25，2026-11-04 缺陷 D-24 由布尔改成矩形）**：`Some(area)` 是"正在铺满"且**记着铺的是哪块可用区**，`None` 是没铺；**只在运行时、不进配置也不落盘**——它是 `mode::capture` 的"不写槽"开关（不是"不测量"开关，见 `mode` 行），也是"下一次双击该还原而不是再铺一次"的判据，重发时护栏要的正是这块矩形（存布尔的话第二轮就得从配置里现推一个位置，两处推到不同的数，护栏判的就不是发出去那件事）。形态切换（`apply`）与从托盘/收起唤回（`reveal`）都会把它置回 `None`，因为那两条路径自己就把几何整套重发了一遍。**图片侧（CHG-005 P6）**：三样产物分三个字段——`image: Option<Admitted>`（S0 入场图）、`image_rows: Vec<RowObservation>`（拆回原行唯一的原料）、`ocr: Option<OcrDocument>`（编辑中的那份，也就是要交给 P7 的那一份），加上 `image_stage: RecognitionStage` 五态、`image_elapsed_ms`、`image_path_input`。`image_request_id`/`image_in_flight` 与文本那条 `request_id`/`in_flight` **分槽**：一次翻译不能让晚到的识别结果作废，反过来也一样（REQ-006 第四条）。`submit_image` 派发即把上一张的行、块、耗时一起清掉（换图即作废，界面上不留"新图配旧结果"）并推到 `Recognizing`，识别中不重复派发返回 `None`；`apply_image` 按代丢弃过期结果；`ignore_image_result` 只把在飞那一次记为过期、**刻意不清**文档（忽略的是这一次，上一张的结果还在原位）；`edit_block_text`/`set_block_enabled`/`remove_block`/`merge_block`/`split_block` 五颗立刻落在 `ocr` 上，`split_block` 依 `source_rows` 回溯、按文字有没有改过分三档返回 `Split`；`image_merged_text` 与 `OcrDocument::merged_text` 同口径，是"参与翻译的文本"那一句的唯一出处。**这一层零 IO**：四颗入口、面板、剪贴板、拖拽全在 `app.rs` 与 `system::*` | FR-1、FR-2 | ✅ P6 |
| `mode` | `commands_for_mode()`、`apply()`、`toggle()`、`capture()`、`PendingGeometry`、`resend_commands()`、`fill_commands()`、`fill_size()`、`restore_commands()`、`snap()`、`dock()`、`reveal_rect()`、`dock_band_contains()`、`walked_away(window, cursor)`/`DOCK_HOVER_PAD`（D-22）、`released()`、`fit_into_area()`、`DragSettler`、`RevealGuard`、`hide(ctx, state, tray)`、`reveal(ctx, state, pending, tray)`、`quit()`、`dim_alpha()`、`level_for_dock()`、`MIN_FULL_SIZE`/`MIN_COMPACT_SIZE` | 两种形态的 `ViewportCommand` 编排（**先改 `MinInnerSize` 再发 `InnerSize`**，顺序本身就是正确性）、置顶与停靠层级、几何捕获与即时落盘、左/右边缘吸附-停靠-悬停唤出-拖离的纯函数、拖动静止判定、失焦降透明度、形态切换的几何宽限期（D-6，v0.7 起是**三段**护栏，见 §3.3）。**铺满/还原（任务 #25）三条**：`fill_size(area, inner, outer)` 用**当帧实测**的 `outer − inner` 当装饰厚度从可用区里减掉（本机 32pt 高、0 宽；不写死是因为 Retina/无边框/将来去掉原生标题栏都会让它变，且两个轴各自 `max(0.0)`——负尺寸会被原生静默忽略，比"铺小了一点"更难查），`fill_commands` 发 `InnerSize` + `OuterPosition(area.min)` 各一条，`restore_commands` 由 `resend_commands(cfg, Full, target_size(cfg, Full))` 派生，保证护栏记的数与发出去的数同一个出处。`capture` 在 `state.fill` 为 `Some` 时**只关写槽、不关测量**（2026-11-04 缺陷 D-24 改的就是这一句，此前是整帧早退）：铺满期间实时几何就是整屏，让它进槽等于把 D-6/D-10 换个入口重犯，而且 FR-3.6 记着的"上次的大小"会变成整屏，重启后窗口再也不是用户那个尺寸——**但"不许写"与"不必量"是两件事**，一起跳过后铺满就成了唯一一条没有自愈的几何请求，命令被原生砍掉一截没人发现（用户原话"双击放大界面还不是全屏"）。所以那一支照常把实测尺寸喂进 `size_axis_writable`，没落定就按宽限期续一帧（续帧不可省：铺满时槽一个字都不写，那条由槽变化立起来的 `CONFIG_SAVE_INTERVAL` 钟在这里帮不上忙，而 egui 是"有事才画"）；重发由 `app::refresh_geometry` 用 `state.fill` 里那块**当初的可用区**算，不是从配置现推位置——槽里记的还是铺满之前的几何，现推会把窗口拽回铺满前再叠上没落定的尺寸。FR-3.9 已于 2026-10-22 改为瞬时切换，**无 `Morph`**。**关窗与唤回各两支（CHG-008，分叉点是 `state.tray` 这一个运行态字段，不进 config、不落盘）**：`hide` 在托盘装上时发 `CancelClose` + `Visible(false)` 并把激活策略切成 `.accessory`（Dock 磁贴随之消失），没装上时**一字不改地保留**今天的 `CancelClose` + `Minimized(true)`；两支的命令队列不许混——`Available` 那支 SHALL NOT 出现 `Minimized(true)`（TC-5.4-c 的否定式判据）。`reveal` 先切回 `.regular` 再 `Visible(true)`（降级支发 `Minimized(false)`），随后**沿用同一段**几何重发：`commands_for_mode` + `target_size` 整套、`pending.set(...)` 上护栏、`apply_topmost`，所以唤回与"窗口一直开着"的画面完全一致。策略切换的原生调用点在 `system::tray::set_policy`，本模块只决定何时调，objc 类型 SHALL NOT 出现在这里。`RevealGuard`（靠几何与焦点猜"回来了"）只在降级支继续生效，托盘支退役——两条并行唤醒路径留着出的就是"点一下弹两回" | FR-3.2~3.9、FR-5.3 | ✅ P5 |
| `theme` | `apply()`、`apply_font_scale()`、`PaneText`、`title_color()`、`surface_fill()`、`window_base()`、`pane_shadow()`、`knob_tones()` | 三态主题映射到 `ctx.set_theme`，字号缩放乘在 egui 默认值上并限幅；`apply()` 每帧重赋 `Spacing::extra_text_line_spacing`（赋值不是累加，帧循环会跑几百次）；**变更 H 新增 `PaneText`——把「字号 + 字距 + 行距」三件事一次性收成一处**，四个正文调用点（全量源文/译文、简洁输入带/译文带）共用，避免 egui 0.37 那三个不同入口（`FontId`、`extra_text_line_spacing` 只喂 TextEdit、section 上的**绝对** `line_height` + `extra_letter_spacing` 要手工挂）各写一份漂掉。**`window_base(dark)` 是两级分层的第一级**（2026-11-04 缺陷 D-23）：整窗基底色，原生 `setBackgroundColor:` 与 egui 的 `window_fill`/`panel_fill` 取的是**同一个数**，第二级才是正文框那层的 `extreme_bg_color`——两处各自持色就是"标题栏的颜色和界面不统一，有色差"的来源，任务 #28 的五套预设色将来只喂这一个字段 | NFR-4、NFR-8 | ✅ P5 |
| `fonts` | `definitions()`、`install()`、`HEITI_PATH`、`HEITI_SC_FACE_INDEX` | 赶在第一帧排版之前（`run_native` 的创建闭包里）把 `/System/Library/Fonts/STHeiti Medium.ttc` 装成字体族：`Proportional` 第 **0** 位、`Monospace` 里排在 Hack **之后**（拉丁列要继续保持等宽）。**面索引 1 才是黑体-简**（第 0 面是黑体-繁，选错不崩不缺字，只有肉眼看得出）；读不到文件返回 `false` 并回落系统解析，**不许因缺字体让界面空白** | NFR-8 | ✅ P5 |
| `views::mod` | `Tab`、`chrome()`、`show()`、`Effects`、`theme_switch()`（内部：`theme_slot`、`theme_at`、`knob_center_x`、`draw_sun`、`draw_moon`、`THEME_SWITCH_W`/`_H`、`KNOB_RADIUS`、`KNOB_TRACK_GAP`、`THEME_EDGE_INSET`）、`pin_toggle()`（内部：`pin_tip`、`draw_pin`、`pin_id`、`PIN_TOGGLE_W`/`PIN_TOGGLE_H`、`PIN_TILT_DEG`） | 顶部工具条 + 页签 + 状态条（设计里归在 `views::full`，实现时抽出共用，简洁模式也要用同一条状态条）；**`theme_switch` 是 2026-10-28 变更 J 抽出来的第二处共用件**——那颗左右推动的主题滑块在简洁窗动作行右下角和全量窗**状态条**右端是**同一个函数**（`salt` 只用于各自记录命中矩形；全量窗那一头 2026-11-04 变更 Q 从底栏挪到了状态条，理由见本行末），落在这一层是因为用户那句"全量界面也要有这个开关"要的是两处不许各画一遍。三态映射是纯函数：`theme_slot` 给 Dark/System/Light = 0.0/0.5/1.0，`theme_at` 按 0.25/0.75 反解（因此天然带迟滞，拖过中间不会连翻三档）；写 `config.ui.theme` + 标脏，生效仍只在 `theme::apply`。**`pin_toggle` 是第三处共用件（2026-10-29 变更 K）**——用户那句"全量窗挪动到精简（悬浮窗）两个字左侧，悬浮窗状态下，放在（右下角）颜色切换左边"要的是同一枚图钉出现在两处，而原生标题栏那颗（`system::pin`）已经连根删掉：egui 画不进 `NSTitlebarView`，能画的地方就只能两处各调同一个函数。图钉是**矢量线稿**（圆帽 + 横梁 + 针，字符图标在 P4 缺陷 D-5 落成过豆腐块）：未置顶 = 空心、斜 `PIN_TILT_DEG`、用主题前景色；置顶 = 实心、摆正、`theme::ACCENT`。它只立意图（`Effects::toggle_topmost`），与 `Cmd+Shift+P`、设置页那一格收口在同一条层级下发上，**这一层不碰窗口**（`salt` 只用于两处各自记录命中矩形）。**`Effects` 图片侧三格（CHG-005）**：`open_panel`、`paste_image`、`ignore_image`——记的都是**意图**不是结果（面板还没回来，帧末只负责把面板叫出来；剪贴板要到那一帧才读；忽略只是把序号记成过期），视图因此能无头调用、永远碰不到 AppKit 与 arboard；识别派发复用既有的 `job` 那一格（`Job::ImageOcr`），不另开第二条派发口。`show()` 在 `CentralPanel` 之前按页签分派**两条**底栏（2026-11-03 从两条改成分派，2026-11-04 变更 Q 又改回两条：`history::theme_bar` 整条删除）：`Tab::Text` → `full::bottom_bar`、`Tab::Image` → `image::status_bar`；`Tab::History` 与 `Tab::Settings` 都不画（同一条几何理由，见 `views::image` 行）。**右下角那颗 `theme_switch` 现在只有一个宿主**——`views::status_bar` 那条三页共用的 `Panel::bottom`（用户 2026-11-04："历史没有这一行，如果放在同样的位置就很突兀，所以还是放在最下方吧，和状态栏一行"），设置页仍不画（那一页自己有「主题」单选，两颗并排会打架）。宿主从三条收敛到一条之后，为"滑块贴宿主栏右缘"而设的那层 `bar_right_edge` 子带随之删掉，**不留用不到的抽象**。两处让位各管一件事，都收在 `theme_switch` 自己身上：`THEME_EDGE_INSET` 4px 让开窗口右缘（用户原话"是不是将其向窗口左侧挪动3-5px"），`KNOB_TRACK_GAP` 3px 让开轨道端头——滑块圆心原来贴着端头内缩一个半径，于是它自己那圈 1.5px 描边正好压在轨道轮廓线上，把轨道右端那半个圆帽盖掉，浅色下轨道只剩一圈线可看，看着就是"按钮缺了一角"（用户同一轮"还是没有完全显示，怎么回事"；逐像素量他那张 74×31 截图的结论：轨道右端 x=59、窗口边框 x=73，中间空 14px，被盖掉的不是窗口那一头）。三档圆心与两端图案圆心都由 `knob_center_x` 一处算出，画滑块、画图案、测试量位置共用同一份口径。 | FR-3.1、FR-3.4、NFR-4、FR-4.1、FR-2.1 | ✅ P6（滑块与图钉在 P5） |
| `views::full` | `show()`、`copy()`、`scroll_area()`、`record_scroll()`、`pane_body_height()`、`char_count_text()`、`bar_actions()`、`bar_layout()`、`bar_height()`、`bottom_bar()` | 全量布局文本页：源文编辑器、译文区两栏等高，两栏之下是**自占一行**的动作底栏（`Panel::bottom`，左边字数计数、右边三/四枚按钮，退让顺序 `84×28` → `64×28` → 换行；**2026-10-28 变更 J 曾把 `views::theme_switch` 放进这一栏的最右格**，2026-11-04 变更 Q 又把它挪到状态条那一行——用户原话"历史没有这一行，如果放在同样的位置就很突兀，所以还是放在最下方吧"，这一栏因此拿回 `THEME_SWITCH_W` 加一道间距那一格宽度（`bar_layout` 不再为它预留））；失败动作按钮在同一行 | FR-1.1~1.5、FR-3.1、NFR-4 | ✅ P5 |
| `views::history` | `show()` | 历史页：搜索、点选回填、时间戳本地化、坏行容错 | FR-4.4 | ✅ P4 |
| `views::settings` | `show()`、`select_default_engine()` | 供应商预设、密钥写钥匙串、主题三态、字号、额外语言、术语分区；外观分区（变更 H）在整体字号之下再给四条滑杆——原文字号 / 译文字号（各 `0.8..=2.0`）、字间距（`0..=4px`）、行间距（`0..=16px`），每根只写 `config.ui` + 标脏；「窗口」分区（F-17）给出「恢复默认窗口尺寸」与「退出程序」两颗看得见的入口，只立意图（`Effects::{reset_geometry, quit}`），落地在 `app`；「退出程序」旁边那句说明由 `system::tray::hint_text(state.tray)` 给（CHG-008：托盘装上写"收进托盘"、没装上写"收进后台"，视图只调用、不各写各的） | FR-4.1~4.2、FR-4.5、FR-5.2、NFR-4、NFR-8 | ✅ P5 |
| `views::image` | `show()`、`status_bar()`（`pub(crate)`；内部：`import_bar`、`preview_row`、`source_preview`、`preview_pixels`、`preview_id`、`block_list`、`count_after`、`region`、`caption`、`record`、`record_checkbox`、`ValueAction`、`ShapeAction`；常量 `PREVIEW_MAX_EDGE`/`PREVIEW_MAX_HEIGHT`/`LIST_MIN_HEIGHT`/`UNSUPPORTED_HINT`） | 图片页四区（CHG-005）：**E 导入条**三颗按钮 + 手输路径那一格，**F 原图预览**（等比缩到长边 ≤320px，`preview_id` 把 `Admitted.label` 与本代 `request_id` 一起编进纹理 ID，换图必然重画、同一张不重复上采样），**H 识别结果列表**（表头五列 选 · `#` · 置信度 · 原文 · 译文，每行复选框 + 就地编辑的单行 `TextEdit` + `合并↓` `拆分` `删除` 三颗；置信度 < `LOW_CONFIDENCE_WARN` 那格加 ⚠ 并用 `theme::WARN`；译文列本包恒为 `—`，留这一格是为了让 P7 有地方长出来；行尾一句"参与翻译的文本：N 字（已勾掉 M 块）"把复选框的效果当场显示出来），**I 状态条**（`RecognitionStage::label()` + 耗时 + 那颗只在识别中出现的「忽略这次识别」）。**动作分两趟落地**：`ValueAction`（勾/改字）只改值，`ShapeAction`（删/合/拆）会重排块号，先值后结构——同一帧里两头的动作都可能发生，反过来的话第二条动作会作用到换过号的块上。拆分按 `Split` 三档各给一句提示（拆回 N 块 / 改过文字所以原文整段留在第 1 块 / 来源行不足两行拆不回）。**每一颗都立刻改到 `AppState.ocr` 那份文档上**——那份文档正是 P7 要拿去翻译与合成的那一份，所以 REQ-005 的判据（改过的文字不丢）就落在这页；列表一变就按 `count_after` 重写 I 区那句"完成 N 块"，两处不同源就是当面自相矛盾。G 结果预览仍是一格空的，画的是回填之后的图，那是 P7 的产物。`status_bar` 必须是 `pub(crate)` 且由 `views::show` 在建 `CentralPanel` **之前**调用：本页正文是一条 `auto_shrink([false, false])` 铺满剩余高度的 `ScrollArea`，状态条排在它后面只剩 0px 一格，那颗「忽略这次识别」根本画不出来（与文本页那条自占底栏同一个坑）。视图不碰系统：面板、剪贴板读图、拖拽落地、派发识别统统记成 `Effects`（`{open_panel, paste_image, ignore_image, job}`） | FR-2.1、FR-2.2、FR-2.3、FR-2.9 | ✅ P6 |
| `views::compact` | `show()`、`band_heights()`（内部：`input_row`、`translation_pane`、`action_row`、`drag_handle`、`band_frame`、`apply_dim`） | 简洁窗版式：带原生标题栏（Q-013），三带堆叠——动作行先建成 `Panel::bottom`，剩余高度由 `band_heights()` 按 **1:1 分给输入带与译文带**（REQ-012，v0.7 推翻了 v0.6 Q-014 的"输入行随内容长到三行封顶"），带间 `BAND_GAP` ≥5px，两带各自是独立圆角框；输入超一屏在框内滚，译文带定高 `ScrollArea` + 尾部 8px 把手；动作行按左→右排**三颗**（复制/设置/机翻\|AI），**右下角钉一格 `views::theme_switch`**（变更 J：它取代了原先行首那颗主题切换框，也取代了行右边缘那颗 `退出`；**「全量」那颗已于 2026-10-28 变更 G 删除**，切回全量改由标题栏绿键与 `Cmd+Shift+M` 承担，见 `system::titlebar`；**「置顶」那颗文字开关已于同日变更 I 删除**；**变更 I 把它换成的原生标题栏图钉已于 2026-10-29 变更 K 撤掉，图钉改由 `views::pin_toggle` 画在这一行的右下角、滑块左边**（`system::pin` 整个模块随之删除，那套 accessory 原生机制不再留存）**），与全量窗同字号同内边距，左排放不下换行、滑块与图钉由一条 `right_to_left` 子带单独钉右（滑块先占走右边缘、图钉后加就落在它左边；子带竖向范围取左排实测那一格，行高仍由左排决定，两者都不参与换行）；两处窗内把手走 `StartDrag`（左排与图钉之间那块空白、译文带尾部 8px——变更 K 之后把手右端从"滑块之前"改成"图钉之前"，图钉那一格自己验过不触发拖拽）；失焦时窗体底色与两带圆角框一起乘 alpha | FR-3.2、FR-3.7 | ✅ P5 |
| `jobs` | `JobRunner`、`submit()`、`drain()`、`apply()`、`OutcomeReceiver`、`admit_input()`、`CHANNEL_CAPACITY` | 唯一的派发口：`Job` 两个变体在这里分岔——`Job::Translation` 走 `runtime.spawn`（网络），`Job::ImageOcr` 走 `runtime.spawn_blocking`（**入场读盘 + 解码 + S1 Vision + S2/S3 合并整段在同一条工作线程**，几百 MB 的截图也不占渲染帧）。两条共用同一个 `mpsc`（容量 8）与同一句 `try_send` + `request_repaint`：**用 `try_send` 而不是 `send().await`**，通道满了就说明界面已经不再回收结果（退出中），await 会永远等不到重绘。`drain` 取回结果交 `apply`，`apply` 按 `Outcome::{Translation, Image}` 各自那一侧的序号判过期。`admit_input` 把 `ImageInput` 变成 `Admitted`：`Path` 连读盘都在这条线程上，`Clipboard` 的像素已由主线程 arboard 取出、这里只做 `RgbaImage::from_raw` 的长度校验（`from_raw` 对长度不符返回 `None`，那句"尺寸与像素数不匹配"因此和"文件解码失败"分写成两句） | FR-2.9、NFR-1、REQ-002 | ✅ P6 |
| `system::clipboard` | `CopyKind`、`compose()`、`write()`、`read()`、`read_image()`、`validate_rgba()`（内部）、`Watcher` | 复制译文 / 复制对照与空译文保护；`Watcher` 是剪贴板即翻的轮询判定（纯函数：开关、可见性、在飞、自写回环、内容变化五道闸），读取本身由调用方注入闭包，因此无头可测。**`read_image()` 只在用户按「粘贴」那一颗时被调一次**，不参与 `Watcher` 的轮询（那是文本即翻，轮询到图片只会白读）；arboard 给的是 `Cow<[u8]>` 的 RGBA 裸像素加宽高，`validate_rgba()` 用 `checked_mul` 校验 `宽×高×4` 对得上才放行——**长度不符返回 `None`，绝不把一张残缺的位图交给工作线程**。取不到剪贴板、不是图片、长度不符都收敛成同一个 `None`，由调用方给那一句"剪贴板里没有图片" | FR-1.5、FR-3.8、FR-2.1 | ✅ P6 |
| `system::panel` | `Reply`、`ReplySender`、`ReplyReceiver`、`channel()`、`classify()`、`request_open()` | 「打开图片」那颗按钮背后的原生面板（CHG-005 Q-002 裁定自接 `NSOpenPanel`，不引 `rfd`）。只做一件事：把面板交出去、等用户选完、把**路径**交回帧循环——像素、解码、缩放一概不在这里，所以本模块不认识图片格式。**`beginWithCompletionHandler:` 而不是 `runModal`**：后者把线程关进 AppKit 的嵌套事件循环，eframe 的帧循环在那期间一帧都不跑（REQ-001 明写 SHALL NOT）。回调只 `try_send` 一条 `Reply` 就返回，因此**回调体里不索引、不解包**——它跑在 ObjC 的调用栈上，那里 panic 会跨过 FFI 边界；`ctx` 在这里唯一的作用是回调末尾请求一次重绘（面板是 AppKit 自己的窗口，用户点"打开"那一下不会给我们的窗口产生输入事件，帧循环不会自己醒来读回信）。`classify(response, path)` 是这条链上唯一能被无头断言的一段，所以单独拎成纯函数：OK 且有路径 = `Picked`，OK 但没路径 = `Failed`，CANCEL = `Cancelled`，其余一切取值（含 `-1001` Abort）= `Failed`——**判据按数值写**，因为 `NSModalResponse` 那三个常量锁在 `feature = "NSApplication"` 后面，而这段判定要在非 mac 编译面也能跑。`channel()` 容量 4（同一时刻只可能有一个面板在用户手上，余量留给回调连着来而帧循环还没读到的那一两帧）。面板里按 `ALLOWED_TYPES`（png/jpg/jpeg，与 `ice_vision::import` 的解码能力同口径）灰掉别的类型是**提示不是限制**：用户仍可选中 `.webp`，那时由 `admit_path` 给出"这张图解不开"那一档文案。`request_open` 返回 `false` = 这条路走不通（非 macOS / 不在主线程），调用方据此提示降级 | FR-2.1 | ✅ P6 |
| `system::screen` | `RawScreen`、`RawRect`、`flip()`、`select()`、`union()`、`fallback_area()`、`work_area()`、`desktop()`、`probe()` | 监视器几何：objc2 读 `NSScreen`（含 `visibleFrame`，即去掉菜单栏与 Dock 的那块）、Cocoa 原点翻转、按窗口选屏；**两条口径分工**——`desktop()` 是所有屏可见区的**并集**，停靠/吸附/唤出用它（桌面外缘才是边的定义，双屏接缝不是，缺陷 D-8）；`work_area()` 是窗口所在**单屏**可用区，只给离屏夹回（FR-3.6）用。非 macOS 与探针失败时回退 `fallback_area`（warn-once，不刷屏） | FR-3.5、FR-3.6、FR-5.3 | ✅ P5 |
| `system::window` | `ns_window()`（仅 macOS） | 从 `eframe::Frame` 拿到那块 `NSWindow` 的**唯一一条路**：`winit_window()` → `window_handle()` → `RawWindowHandle::AppKit(ns_view)` → `NSView::window()`，入口先 `MainThreadMarker::new()?` 卡住线程。`unsafe` 只在这一层（`ns_view` 由 winit 从 `contentView` 取出，生命周期归窗口）。失焦半透明与标题栏绿键都要问原生窗口，两处各写一份就会漂（缺陷 D-9 的同一课） | FR-3.7、FR-3.2 | ✅ P5 |
| `system::alpha` | `set()` | 整窗不透明度：`system::window::ns_window` 取窗口，`NSWindow.setAlphaValue` 写档位，返回是否成功。全量窗固定 1.0、简洁窗按 `unfocused_alpha`（FR-3.7，2026-10-28 从"只染窗内底色"改成整窗原生） | FR-3.7 | ✅ P5 |
| `system::background` | `set()` | **整窗底色**（2026-11-04 缺陷 D-23）：同一个 `system::window::ns_window` 取窗口，`NSWindow.setBackgroundColor:` 写底色。为什么必须走原生这一句——标题栏那 32pt 连同红绿灯由 AppKit 的 `NSThemeFrame` 画，**压根不在 egui 的绘制清单里**，`Visuals` 改多少次都还是两块色（`system::alpha` 当年撞的是同一堵墙，公开 API 里能让整扇窗户用同一个底的只有这一条）。分量走 **sRGB** 那条构造（`colorWithSRGBRed_green_blue_alpha`）：`Color32` 的分量本就是 sRGB，`colorWithRed:` 用的是 `deviceRGB`，在 P3 屏上会偏一档——那正是要修的"色差"的另一种形态。返回 `false` = 非 macOS / 拿不到句柄 / 不在主线程，调用方**不缓存**、下一帧重试；与 `apply_native_alpha` 共用同一条 `needs_native_update`（值变了才下发，也才记日志）。两级分层（基底 / 正文面）的规矩写在模块头，色值出处是 `theme::window_base` | NFR-4、FR-3.1 | ✅ P5 |
| `system::titlebar` | `sync()`、`Outcome`、`mark_click()`、`take_click()` | 标题栏绿键改道（2026-10-28 变更 G，用户"悬浮窗更换全量的按钮可以不要，将标题栏最大化功能替换为切换全量"）：`standardWindowButton(ZoomButton)` 取那颗按钮，`define_class!` 注册一个只做"记一次点击"的 `NSObject` 子类当新 target，`action` 换成 `iceToggleFullFromTitleBar:`。**每帧从按钮本身读 `action()`/`target()` 决定改道还是还原**（AppKit 会在成为 key 窗口等路径上重新接线，缓存"我接过了"会悄悄失效）；`Retained<ZoomTarget>` 存在线程局部 `WIRED` 里，因为 `NSControl.target` 是 `unsafe_unretained`；改道时一并重写 tooltip（行为变了提示不能还说"缩放"）。点击经 `CLICKED` 原子量交给帧循环，落地仍是 `Effects::toggle_mode`，与 `Cmd+Shift+M` 同一条路径 | FR-3.2、FR-3.3 | ✅ P5 |
| `system::dblclick` | `sync()`、`Outcome`、`note_double_click()`、`take_double_click()`、`title_band()`、`in_band()`、`EDGE_PAD` | 标题栏双击 = 铺满/还原（任务 #25，用户"双击标题栏：铺满整屏 + 再双击还原"）。**为什么不是 `NSTitlebarView` 的命中测试**：那条路径要子类化 `window.contentView().superview()` 并覆写 `mouseDown:`，会掐断窗口原生拖动；`NSWindow.titlebarView` 在 objc2-app-kit 0.3.2 里**根本没绑定**，公开 API 中唯一既能看见又能取消派发的位置是 `+[NSEvent addLocalMonitorForEventsMatchingMask:handler:]`——返回 `nil` 即吞掉这一次事件。**只吞双击**：`clickCount() >= 2` 且落进 `title_band()` 算出的那条带（窗口内坐标，y 从 AppKit 的左下原点起算），单击与内容区一律原样放行。带的左右边界由 `standardWindowButton(...).frame()` 折出红绿灯的最右缘、右端只留一道 `EDGE_PAD`（2026-10-29 图钉从原生标题栏挪进界面工具条以后，栏里右半再没有会收点击的东西）；宽度取不到就返回 `None`，`in_band` 同时拒绝退化带（`left < right && bottom < top` 不成立时一个像素都不吞）——**判据是 fail-closed**：拿不准就不吞，宁可由系统去做它默认的最小化。`WATCHED` 存的是被监听那块 `NSWindow` 的指针，换了窗口（重建/换形态时 winit 换窗口）就重新挂；`RcBlock` 由我们自己在线程局部里持有，因为这条 API **不 retain** block，出了作用域就是野指针。只在**全量形态**安装（简洁窗那份几何属于吸附/停靠，双击铺满会把它整屏化、热区就没了），卸载时 `removeMonitor` 并把 `WATCHED` 归零。`Outcome::Unavailable`（拿不到窗口/挂不上 monitor）时什么都不改，系统默认行为原样保留。**已知代价（写进 UI 设计）**：双击之后立刻按住拖动这条路失效——第二次按下的那一下已被吞掉，AppKit 的拖拽循环没有开始。**无头测不到原生那半**，能锁的只有带几何与"一次双击只交接一次" | FR-3.2、FR-3.6 | ✅ P5 |
| `system::mouse` | `left_button_down()`、`cursor()` | **松手的现场状态**（2026-10-28 缺陷 D-16）：`+[NSEvent pressedMouseButtons]` 读窗口服务器维护的硬件按键掩码，取位 0（左键）。原生拖拽走的是 AppKit 的嵌套事件循环，鼠标抬起那一下**不投递给本应用的 view**，所以 egui 的 `pointer.any_down()` 在整段拖动期间是旧值、判不出松手——只能问硬件。返回 `Option`：`None` = 问不出来（非 macOS、或不在主线程被 `MainThreadMarker::new()?` 挡下），调用方 SHALL 把它当"不知道"并退回纯时间判据，**不许因为问不到就永远不吸附**。无需辅助功能/屏幕录制权限。② `cursor()` 是鼠标在**全局坐标系**里的现位置（`+[NSEvent mouseLocation]`，原点在左下角，y 向上），交给 `system::screen::flip_point` 翻成 egui 那套左上原点的逻辑点。它买的是停靠条的收回判定（缺陷 D-22，2026-11-04）：`mode::walked_away` 问的是"指针走出了窗口外扩 24pt 那一圈没有"，而这个问题的**唯一**答案在窗口服务器那里——条贴边缩起来之后窗口自己只剩 3pt 宽，指针"还在条上"这件事窗口矩形根本装不下，用 egui 的 `pointer::pos()` 更不行（隐藏/贴边期间没有输入事件流）。问不到时调用方先退回 `ctx.input pointer::hover_pos()` 那份代理量（窗口内局部坐标 + 外框原点搬到同一套），两份都问不到才算走开——与 ① 同一条纪律：宁可早收，不许永远挂着 | FR-3.5、REQ-003 | ✅ P5 |
| `system::tray` | `SUPPORTED`、`Tier::{One, Two}`、`Outcome::{Reveal, ToggleMode, Settings, Quit}`、`install(mode, ctx, scale)`、`sync(mode)`、`take_outcome()`、`action_of(outcome)`、`menu_titles(mode)`、`hint_text(state)`、`set_policy(regular)`、`glyph(tier)`/`glyph_bytes(tier)` | 菜单栏托盘（CHG-008，FR-5.4）：`NSStatusBar::systemStatusBar()` 上挂一枚长度自适应的 `NSStatusItem`。图是**运行时画的单色线稿 + `setTemplate(true)`**（不是 Dock 那枚彩色方砖缩小——18pt 下读不出形状，彩色也不跟随菜单栏深浅；也不是字符——D-5 那条纪律对所有装饰图案生效，托盘这枚也不例外），`glyph()`/`glyph_bytes()` 完全不碰 AppKit，@1x/@2x 按窗口 `scale_factor` 选档。**与 `system::titlebar`/`system::dblclick` 同一个形状**：ObjC 回调不在帧循环里，方法体只做三件事——读 `currentEvent().type()` 分叉、往一个 `AtomicU8` 槽立**一个**意图、`request_repaint()`；兑现全在 **`App::logic` 里的 `pump_tray`**（v0.2 的更正：泵原来挂在 `App::ui`，而窗口 `orderOut` 之后 eframe 走 `!show_ui` 那一支、`ui()` 一次都不被调（`glow_integration.rs:692-720`），收起之后那一点永远没人取——缺陷 D-21）；`logic()` 可见与隐藏两态都跑、一帧一次，`install` 要的 `scale_factor` 也从这里那个 `Frame` 读。四条意图映射到 `Action::{RevealWindow, ToggleMode, OpenSettings, Quit}`，其中后两条转接**既有**实现（`Cmd+Q` 那条 `Quit`、`app::open_settings`），托盘既不是第二条退出路也不是第二条开设置路。左键直接唤回；右键**不常驻挂菜单**（菜单一挂上去左键就不走 action），只在右键成立那一下**在回调里同步** `popUpStatusItemMenu:` 就地弹一次（四项：`显示窗口` / 另一侧形态名 / `设置` / `退出程序`），追踪循环返回后 `setMenu(None)` 并核对按钮接线还在不在；弹出这件事**不登记旗子等下一帧**——它不需要 `AppState`，而隐藏期可能根本没有下一帧在等它。四个 `Retained` 先克隆出来、释放 `thread_local` 的借用再进那个阻塞的循环，否则重入 `TRAY.with(..)` 是双重借用 panic。`install()` 幂等——重复造会在菜单栏里长出第二枚图标；`sync()` 每帧从按钮本身核对 `target`/`action` 与第二项文案（AppKit 会在别的路径上重新接线，缓存"我接过了"会悄悄失效；文案跟着当前形态在全量↔简洁之间改口，且 SHALL NOT 写"悬浮窗"）；`Retained` 的 target、菜单项、位图由线程局部持有，因为 `NSControl.target` 是 weak。`set_policy(regular)` 是"Dock 磁贴消失/回来"**唯一**的原生调用点（`setActivationPolicy:` + `activateIgnoringOtherApps(true)`，`#[allow(deprecated)]`：非 deprecated 的 `activate()` 是协作式的，不保证真的把应用带到最前），`mode::hide`/`mode::reveal` 只决定何时调、本模块不碰 viewport 命令；菜单栏上没有那枚图标时**拒绝切换**——唤醒入口都没装上就把磁贴收走，等于把用户关在门外（REQ-006 那条老缺陷）。`hint_text()` 让设置页那句说明跟着 `TrayState` 一起改口。非 macOS 那一支只导出同名函数恒返回 `Unavailable`，不引用任何 objc 类型。依赖增量：app-kit 的 `NSStatusBar`/`NSStatusItem`/`NSStatusBarButton`/`NSResponder`/`NSMenu`/`NSMenuItem`/`NSImage`/`NSRunningApplication` 八个 + foundation 的 `NSData`，每个在 `Cargo.toml` 那段注释里写明它买到的符号（变更 K 立的纪律：不开 default 换最小面，新增必须给证据） | FR-5.2、FR-5.4、REQ-001~007 | ✅ CHG-008（真机 W-2.40 待走查） |
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

**P6 实现后的校正（2026-11-03，CHG-005）**：上图从"拖拽/粘贴/选文件"到"列表可编辑"那一段已按本包落成，`bg::estimate` 之后的三步仍是 P7。三处与封存设计的偏差登记如下，均不改 REQ：

1. **入口是四条不是三条**：面板、拖拽、粘贴之外，图片页还留了一格手输路径（`AppState::submit_image_path`）。它不是备胎——面板弹不出来时（`system::panel::request_open` 返回 `false`）界面上唯一还能走通的路就是这一格，测试机以外的环境无法保证 `NSOpenPanel` 一定接得上。
2. **没有 `CancellationToken`**：设计里那句"取消在块边界检查"前提是识别是一串可中断的块。实际的一次 Vision 调用是**一个不可撤的整段阻塞调用**，Q-008 据此裁定：那颗按钮写"忽略这次识别"，行为是把这一代的序号记成过期（`ignore_image_result`），结果回来时按既有竞态通道丢掉，**不说"已取消"**。`tokio-util` 因此没进依赖表。
3. **五态不是七态**：`RecognitionStage` 只有 `Idle / Recognizing / Done{blocks} / NoText / Failed(reason)`。`Decoding` 与 `Recognizing` 在同一条 `spawn_blocking` 里连着跑，中间没人能观察到，把它拆成两态只会给界面一个永远闪过去的档位；`Translating`/`Compositing` 归 P7 各自再加。`NoText` 单列是因为"这张图没字"报成识别失败是误导（`VisionError::NoTextFound` 在错误枚举里也就是为这一条存在）。

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
- **标题栏双击 = 铺满可用区 ↔ 还原**（任务 #25，用户"双击标题栏：铺满整屏 + 再双击还原"，2026-10-28）：链路是 `system::dblclick::sync(frame, mode == Full)`（每帧挂/卸，`app::apply_dblclick_hook`）→ `app::poll_dblclick` 取走那一次双击 → `Effects::toggle_fill` → `app::toggle_fill` 按 `state.fill` 决定方向。**五件事是这条改动的全部风险面**：① **铺的尺寸不是可用区本身**——命令下发的是**内尺寸**，而可用区是**外框**目标，所以 `mode::fill_size` 把当帧实测的 `outer − inner`（本机 32pt 高、0 宽）减掉再发；差值取实测而非写死 32，因为装饰厚度会随 Retina 缩放因子与去原生标题栏的变化漂，写死的那次数值一旦漂就必然溢出屏幕外。两个轴各自 `max(0.0)`：负尺寸会被 `request_inner_size` 静默忽略，"命令发了窗口一动不动"比"铺小了一点"难查得多。② **还原要有源**——`mode::capture` 在 `state.fill` 为 `Some` 时不写槽，`full_rect` 因此一直保持铺满之前那组几何，它同时是 FR-3.6"记住位置尺寸"的槽和还原的目标；让整屏尺寸进槽就是把缺陷 D-6/D-10 换个入口重犯，还多赔上"重启后窗口再也不是用户上次那个大小"。形态切换（`mode::apply`）与唤回（`mode::reveal`）都清 `fill`，且 `apply` 里**必须在内部那次 `capture` 之后清**，否则整屏尺寸会先被记进目标槽。③ **铺到一个本来就等于槽里尺寸的尺寸要拒绝**（`app::fill_is_no_op`）：那种情况下 `fill` 立起、capture 停写，下一次双击"还原"回的是一整屏，用户看到的是第一下没反应。④ **两种去向都只发 `InnerSize` + `OuterPosition` 各一条**，并把即将下发的那个内尺寸一字不差记进 `PendingGeometry`（沿用 REQ-004 的三段护栏，不为铺满另开一套判定）。⑤ **②那条"不写槽"不许顺手把"不测量"也带进来**（2026-11-04 缺陷 D-24）：第一版就是在 `capture` 开头整个 `return`，于是铺满成为**唯一一条没有自愈的几何请求**——命令被原生砍掉一截既没人量、也没人重发，界面就停在"放大但没到全屏"那一格。改法是那一支照常喂 `size_axis_writable`，没落定就按宽限期续一帧（续帧不可省：槽一个字不写，`CONFIG_SAVE_INTERVAL` 那口钟立不起来，而 egui 是"有事才画"），重发时用的是 `state.fill` 里那块**当初的可用区**而不是从配置现推的位置。日志里那句 `area=?` 就是为这一条加的：只有尺寸看不出大在哪块屏、哪块可用区上。**只在简洁窗之外的那一侧安装**：简洁窗的位置属于吸附/停靠几何，双击把它整屏化之后那 3px 热区就没了。**已知代价**（写在 `03-ui-design.md`，走查 W-2.37 要盯）：双击之后立刻按住拖动这条路失效——第二次按下的那一下已被监视器吞掉，AppKit 的窗口拖拽循环没有开始；`Outcome::Unavailable`（拿不到 `NSWindow`、monitor 挂不上）时什么都不改，系统的"双击标题栏"设置（通常是最小化）原样生效，界面不会半死不活。
- **置顶的可见入口是那枚 egui 图钉**（FR-3.4；2026-10-28 变更 I 由用户提出"置顶不应该是文字，应该和大部分软件一样，可以在标题栏最右侧有一个类似于钉子的图标，点击置顶变成蓝色并且钉子变成垂直状态，则置顶。再点击变回来。全量界面也要有。"，**2026-10-29 变更 K 按"全量窗挪动到精简（悬浮窗）两个字左侧，悬浮窗状态下，放在（右下角）颜色切换左边"把它从原生标题栏搬进界面**）：变更 I 那一版挂在 `NSTitlebarAccessoryViewController` 里，公开 API 只有那一个槽能"既在栏内又收得到点击"，而它**只能落在标题栏右边缘**，够不到用户指定的那两个内容区位置，于是整模块删除（`system::pin`、`app::{apply_pin, poll_pin_click}`、`objc2-app-kit` 九个 feature）。现在是 `views::pin_toggle`：`allocate_space` 之后**恰好一条** `interact(rect, pin_id(salt), Sense::click())`，`draw_pin` 现画圆帽 + 横梁 + 针三段线稿（不用 SF Symbol、不用字符——回到 egui 就重新受 D-5 那条约束），两态只换实心↔空心、转不转 `PIN_TILT_DEG`、`theme::ACCENT`↔主题前景色；位置由两处各自那条 `right_to_left` 带里的**调用顺序**决定（先加的贴右缘、后加的落左边）。三条不变量：① **两处共用同一个函数**，salt 只用来分开记录矩形，"全量界面也要有"仍是这一句实现的；② **外观每帧从 `config.ui.always_on_top` 重算**，不缓存"我上次画成什么"，任何一条路径（图钉本身、`Cmd+Shift+P`、设置页那一格）改的状态都在下一次重绘里如实显示；③ **点击只置 `Effects::toggle_topmost`**，落地仍是 `app.rs::toggle_topmost` 那一条（`state.toggle_topmost()` + `mode::apply_topmost` + 提示条），**置顶在代码里仍然只有一套实现**（`the_pin_effect_and_the_hotkey_share_one_path` 顺带锁"一次点击只下发一条 `WindowLevel`"）。换进 egui 的净收益：位置、命中格宽度、两态提示文案、命令条数、"按住它不许拖窗"从无头测不到变成全部锁得住，无头剩下的只有像素观感（W-2.34）。
- **提示条一律居中、1s 淡出、不可关闭**（2026-10-28 变更 I，用户"所有的提示类弹窗都是中心弹窗500ms内淡化消失，不应该有关闭的操作和按钮。"；**2026-10-29 变更 L 只改时长**：用户"所有弹窗改为1s淡化消失吧，500ms太短了"）：`views::toasts` 用 `Area::anchor(CENTER_CENTER)` + `Order::Foreground`，`Frame::popup` 的底色/描边/投影由 `multiply_with_opacity(opacity)` 一次乘好、文字色另走 `gamma_multiply`；`opacity` 是 `TOAST_LIFETIME`（现为 1s）上的线性衰减，**同一个常量既管活多久也管淡多久**，不存在"淡完了还挂着"的中间态——那次改判因此只动了这一个数，`Toast::opacity()` 与 `expired()` 都从它推导。**`Area::interactable(false)` 是这条改动的关键**：egui 把不可交互的 layer 整块排除在命中测试之外，所以居中的浮层压在按钮上时下面那颗照样点得到——改成居中而不改交互，就等于把 P4 那颗 `×` 的拦截面积从一小块扩大到窗口正中，是不可接受的回归。代价是淡出期间必须逐帧重绘：屏幕上有提示条时 `request_repaint_after(TOAST_FADE_STEP)`（16ms，1s 整段约 60 帧——比 500ms 那一版多出一倍的重绘帧，仍只在有提示条期间），一空就回到 NFR-2 的空闲策略，TC-N-1 那条 30s cputime 采样测法不变。`confirm()`（失败态那两颗动作按钮）**不在这一条里**——它要的是用户做一个决定，自动消失等于替用户答"否"。启动时那条配置损坏的黄色提示（`ConfigParse`）因此也在这 1s 内淡掉：它本来就只说一次，界面上不留下第二次读它的地方，登记在 `07-acceptance.md`。
- **形态切换的几何下发**（FR-3.9，2026-10-22 由"形变动画"改为瞬时）：切换时在同一帧内一次性下发 `Decorations(目标形态)` → `InnerSize(目标尺寸)` → `OuterPosition(目标位置)`，没有 `Morph` 状态、没有逐帧补间，只有一组一次性命令。`Decorations` 与 `WindowLevel` 是原生属性，本来就只能整段设一次（逐帧设会闪烁）。下发的是**请求**而非同步生效（`InnerSize` 走 `request_inner_size`，异步），因此切换完的下一帧必须用 `mode::capture` 回读真实几何再落盘，不能拿目标值当已生效值。**但 capture 在"命令已发、窗口还没跟上"的过渡期里读到的仍是旧形态的尺寸**（缺陷 **D-6**，2026-10-23 走查取证）：那条过渡几何会被写进目标形态的配置槽，而尺寸命令只在切换那一刻发一次，于是简洁窗永远保持全量的大小。修法不是不写，而是加一个宽限期：切换时记下 `pending_geometry`（目标尺寸 + 时刻），`MODE_GEOMETRY_GRACE`（1s）内 capture 只写位置轴、不写尺寸轴，直到实测尺寸命中目标（±2px）或宽限期到期为止。
  - **三段护栏**（2026-10-27 由缺陷 **D-10** 升级为三段，v0.7 / REQ-004）：只有"宽限期到期即放行"是不够的——原生窗口没跟上的那次切换，到期后会把大窗尺寸照抄进 `compact_rect`，而尺寸命令此后再无来源，`measured == target` 每帧命中，坏状态**自我维持**（盘上两槽四项全等 984×632 就是它的指纹）。三段是：① **宽限期内**（第一段）尺寸轴静默，位置轴照写；② **第一段到期**不再直接放行，`mode::resend_commands` 只补发一组 `InnerSize` + `OuterPosition`（`take_retry` 保证恰好一次），把护栏交回给第二轮；③ **第二段到期**采纳**什么都不写**——把当时实测尺寸存为 `baseline` 并 WARN（只报两个尺寸，不报路径），此后尺寸轴只在"与 `baseline` 的欧氏距离 > `PENDING_SIZE_TOLERANCE`（2px）"时才写回，也就是"用户真的手动改过尺寸"，一次抖动或同一尺寸反复上报都保持静默。`PendingGeometry::set()`（新一轮切换）无条件清 `baseline`。两段加起来的判据由测试 `the_two_slots_never_end_up_equal_after_a_failed_switch` 锁住。
  - **与 Spec 的两处实现偏离**（2026-10-23 记，均已在 `features/CHG-004/evidence.md` 取证）：① 切换那一帧仍然调用一次 `ctx.request_repaint()`。Spec 写的是"切换后不再 `request_repaint()`"，针对的是**逐帧续期**的补间；但目标内尺寸若恰好等于当前尺寸，winit 不会发 `Resized`，没有这一次显式重绘，当前帧画的还是上一形态的布局（切换后第一眼就是错的）。补间已取消，因此这里只有**一次**重绘，之后回到"提示条到期与结果到达才重绘"的空闲策略，NFR-2 不受影响。② `capture` 在几何变化时立一个 `CONFIG_SAVE_INTERVAL`（500ms）的延时重绘，这是 REQ-009 要求的落盘钟，不属于切换的续期。
  - **窗口收起时触发切换**（`state.hidden`）只把目标形态记进配置，一条几何命令都不发，也不把窗口叫回来；从 Dock 恢复时 `reveal` 会按 `config.ui.mode` 整套重发几何。
  - **「恢复默认窗口尺寸」不走 `mode::apply`**（REQ-011）：那颗按钮的语义是把两个尺寸槽改回默认值并立即落盘，而 `mode::apply` 开头会 `capture` 当前真实几何——坏尺寸正是用户要点按钮清除的东西，用它等于把污染又抄一遍。实现是纯函数 `app::reset_window_sizes`：写默认尺寸 + 清停靠、发**一组**几何命令、给 `pending_geometry` 立护栏。位置轴由 `restored_rect` 决定——已知可用区时在其内居中，取不到（首帧还没走 NSScreen）时保留原 x/y，不为了一次复位把窗口甩到屏幕外。
- **主题滑块是两种形态共用的一个函数**（NFR-4、FR-4.1，2026-10-28 变更 J，用户"悬浮时放在右下做成一个类似于开关左右推动的效果……而且全量界面也要有这个开关，不需要提示，看到的人都懂。也在右下角。"）：`views::theme_switch(ui, state, salt)` 一处实现，调两次——简洁窗动作行那条 `right_to_left` 子带（salt `compact`）与全量窗三页共用的 D 状态条（salt `status`，`views::status_bar` 里 `state.tab != Tab::Settings` 才画）。**宿主这一格在 2026-11-04 走过两轮**：变更 O 按"主题颜色切换图标除设置页都应该有"先给它建了三条宿主栏（文本页 E 底栏、图片页 I 状态栏、历史页新建一条只挂滑块的栏），同日变更 Q 按"历史没有这一行，如果放在同样的位置就很突兀，所以还是放在最下方吧，和状态栏一行"收成状态条这一处，`history::theme_bar` 与只剩一个调用者的 `bar_right_edge` 随之删除，文本页 E 底栏拿回 `THEME_SWITCH_W` 加一道间距那一格宽度（`bar_layout` 不再为它预留）。**为什么必须共用**：用户要的是"每处都有这颗开关"，各处各画一遍就会在下一轮走查里漂成两个样子——这与 K-03/K-09 那两条"共用同一个原生窗口"是同一课。**控件里不含文字**，所以它不受字号缩放影响、画出来的那一段恒为 `THEME_SWITCH_W`=40px；它的**槽**是 `THEME_SWITCH_W + THEME_EDGE_INSET`（4px，变更 Q：圆角边框与窗内那圈看不见的空气墙会把贴边控件吃掉一点，让位做在槽上，画与命中只吃左边那 40px，两处宿主一起生效）。轨道内还有一处让位：`KNOB_TRACK_GAP`=3px（缺陷 D-17），滑块圆心由 `knob_center_x` 一处算出，画图案、画滑块、测试量落点共用同一份口径。**图案是线稿不是字符**（D-5 的豆腐块教训）：太阳 = 一圈 `circle_stroke` + 八道光芒，月亮 = 外弧 + 内弧**两条开放折线**（缺陷 D-18，2026-11-04）——合成一条 `closed_line` 会在 152.8° 的发卡尖端触发 epaint 的 miter join，沿角平分线甩出一条亮线。**命中只有 `Sense::click_and_drag()` 一条**，落点横向位置经 `theme_at` 反解成档位；轨道占位用 `allocate_space` 而不是 `allocate_at_least`——后者内部自带一次 `interact`（`allocate_response` = `allocate_space` + `interact`），再自己调一次 `interact` 会让同一枚指针被两个响应者各消费一遍，且那两个响应者的 `Id` 不同。**没有悬停提示**是用户明说的，不是漏配。矩形记录走 `Context::data` 的临时项，测试经 `recorded_rect(ctx, "theme", salt)` 读。**`Theme::next()` 随之删除**：点三下的循环式子在滑块上没有对应动作，领域层不留没人调的换档函数（三态映射改由视图侧的 `theme_slot`/`theme_at` 这一对纯函数承担，测试锁死两端与中间、以及 0.25/0.75 的迟滞边界）。
- **滚动容器与动作栏的边界**（2026-10-27 按缺陷 **D-9** 补，v0.7 按 **REQ-013** 改法，两窗共用）：`views::full::scroll_area()` 是两栏与简洁窗译文的同一个构造，里面必须 `.min_scrolled_height(0.0)`——egui 的 `ScrollArea` 有个默认 64px 的 `min_scrolled_size`（`scroll_area.rs:469`，在 `:856` 以 `inner_size[d].max(…)` 生效），矮栏里它把视口抬到 64px 并画到框外，而最大滚动偏移按这个虚高视口算，结果**最后十几像素怎么滚都滚不进来**（简洁窗 130px 高时实测视口底边 115.9 越出译文框底边 103.0、压在动作行顶边 108.0 之上）。"滚不动"的风险不由这条默认值兜，由调用方的栏高守卫兜（`views::compact::MIN_BODY_HEIGHT`）。全量窗这边 v0.7 起换了结构：右下角那条动作条原本是**浮层**、不参与布局，正文末尾得按 `bar_cover()` 预留一条与条子等高的空白带，而"预留公式"与"实画公式"一旦不同步就又把最后一行盖住——现在它是 `Panel::bottom("full_bottom_bar")`，**自占一行**，两栏的可用高在构造时就已被它扣掉，所以这条不再需要任何预留，`bar_cover()` 与它的断言整套作废。保证从"算得准"变成"结构上盖不到"（实测余量 36px，由 `the_last_line_stops_above_the_bottom_bar` 断言"可视区底边在底栏上沿之上"锁住）。
  - 两条一起画的教训（v0.7 踩过的坑，写进 `05-dev-guide.md`）：① 自占高度的面板里必须 `ui.advance_cursor_after_rect(rect)`，否则 `Panel` 量到 26×10 直接塌成一条；② `Ui::columns` 不看光标、按整行宽度均分，和上面的定高行会重叠 5px，要用 `horizontal_top` + `allocate_ui_with_layout`。**凡是不参与布局的覆盖式浮层，优先考虑改成自占一条带**——预留公式与绘制公式分处两地，就是下一次同类缺陷的来源。
- **几何持久化时机**（P1 实测：SIGTERM/强杀不走 `on_exit`，只靠退出保存会丢位置）：在①拖动结束（吸附判定完成时）②形态切换的命令**落定**的那一帧③关窗/退出时 三处强制写盘，另加④`capture` 每次几何变化都续一帧 500ms 的重绘钟，保证用户最后停下的位置一定会落盘。均为小文件（<1KB）原子替换，代价可忽略。

## 4. 并发模型

| 场景 | 机制 | 原因 |
|---|---|---|
| HTTP 翻译 | tokio `spawn`（多线程 runtime，由 `ice-app` 持有） | 不阻塞渲染 |
| 长文分段（FR-1.9） | `providers::parallel::map_segments`：`JoinSet` + 定长结果槽，在飞窗口 ≤ `MAX_IN_FLIGHT`(4) | 段间无依赖，串行只是把往返时间相加。窗口设上限而不是全量并发，是为了不打爆机翻免费额度、不诱发 AI 侧 429；结果写回**槽位**而不是 push 进 Vec，所以谁先答完都不影响拼接顺序；任一段失败 `abort_all()` 后收摊，不再继续花额度 |
| OCR / 图像合成 / 字体加载 | `spawn_blocking` | Vision 与像素操作是 CPU 阻塞调用。**P6 落地的口径**：整段（读盘 → 解码 → 超上限缩放 → Vision 调用 → 行合并）待在**同一条**工作线程里，一次任务不在池间换手；`ImageInput::Clipboard` 那一支的字节由主线程的 arboard 取出（它拿的是系统剪贴板句柄，挪到别的线程属于另一套线程约束），剩下的"拼缓冲 + 缩放"仍在工作线程 |
| 结果回投 UI | `mpsc::channel(CHANNEL_CAPACITY)` + `ctx.request_repaint()` | eframe 事件驱动，必须主动唤醒。**有界**而非设计里写的 `unbounded`：无界队列在结果积压时会悄悄吃内存，有界则让背压可见。**发送一律用 `try_send`**：通道满了就意味着界面已经不再回收结果（退出中），`send().await` 会永远等不到那次重绘。P6 的识别结果与 P3 的翻译结果**共用这一条通道**，靠 `Outcome` 分支、各自那一侧的序号判过期 |
| 钥匙串读取 | `spawn_blocking` + `mpsc::channel(1)`，建窗**之后**才发起 | macOS 对不由本程序写入的条目会弹授权框，`get_password()` 一直阻塞到用户点完。放在建窗之前窗口根本出不来，表现为"启动卡死"（P4 真机踩到并修掉）。线程内必须用 `blocking_send`：`send` 是 async 的，在 `spawn_blocking` 里只会造出一个没人 await 的 future |
| 「打开图片」面板 | `NSOpenPanel.beginWithCompletionHandler:` + `mpsc::channel(4)`（`system::panel`） | 不用 `runModal`：它把线程关进 AppKit 的嵌套事件循环，帧循环在那期间一帧都不跑。回调跑在 ObjC 栈上，所以只 `try_send` 一条消息，不在回调里索引、不解包、不读盘 |
| 菜单栏托盘的点击 | ObjC `target/action` 往一个 `AtomicU8` 槽立**一个**意图 + `ctx.request_repaint()`（`system::tray`） | 窗口 `orderOut` 之后不再产生 `RedrawRequested`，而 eframe 0.37 会**直接**给不可见窗补跑一轮（`run.rs:243-250`，节流 100ms），但那一轮 `show_ui` 仍按可见性判 false、走 `update_logic_only`，**跑 `logic()` 而不绘制**（v0.2 查清），所以回调里那一次唤醒就是隐藏期唯一的唤醒源：既不装常驻事件监视器，帧尾也不发任何续期（REQ-007）。意图交回帧循环后走 `run()` 既有分派，兑现与发起不在同一个线程时机 |
| 引擎实例共享 | `Arc<dyn TranslationEngine>` | `reqwest::Client` 内部连接池，可复用 |
| 缓存 | `Mutex<LruCache>` | 临界区极小，不值得无锁结构 |
| 取消 | **没有 `CancellationToken`**（P6 依 Q-008 改掉本行原设计） | 原设计写"块边界检查取消"，前提是识别是一串可中断的块；实际的一次 Vision 调用是一个**不可撤的整段阻塞调用**，从外面强杀它正是"留下半文件"那一类风险。落地语义因此是"忽略这次结果"：`ignore_image_result` 把在飞那一次记为过期、界面立刻回空闲态，底层那条线程自己跑完、结果回来时被序号丢弃。`tokio-util` 因此从未进依赖表 |

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
| vision | `UnsupportedPlatform` | **两条路**：图片页那四颗入口在能力缺失时整排置灰 + 悬停给 `UNSUPPORTED_HINT`（"当前平台没有本地文字识别；文本翻译不受影响"），已经派发到工作线程的那次失败走 `image_failure_text` 的原句（`to_string()` 已是人话）。页签本身**不禁用**——手输路径那一格和"这张图我们解不开"的提示都要有地方给 |
| | `Decode(msg)` | "图片格式不支持或已损坏（本版本支持 PNG / JPEG，以及剪贴板里的图片）"——`thiserror` 文本直接上屏，UI 不再猜 |
| | `TooLarge(px, limit)` | "图片太大，超出可处理上限 {limit} 像素"。**这一句不原样上屏**：`#[error]` 那行带的是两个像素数，对用户是噪声，只留上限 |
| | `NoTextFound` | `RecognitionStage::NoText` 一态，I 区那句"未发现文字"，**不算失败**（图没错、机器没坏，是这张图上真没有可读的字） |
| | `Recognition(msg)` | "本机识别失败，请重试或换一张图"。**框架回的那段英文不原样上屏**：它是 Vision 的内部串，用户读不懂、日志里也不该留（NFR-3） |
| | `Font(msg)` | 结果区顶部警示条："缺字形，已回退"（P7） |
| | `Composite(_)` / `Font(_)` / `NotImplemented(_)` | "本机处理失败，请重试"——本包这三条没有触发点（回填与合成属 P7），归到这一句是为了 `image_failure_text` 对 8 个变体**穷举**，将来 P7 新加变体会在编译期被 `match` 挡住而不是走默认分支 |
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
| FR-5.4 菜单栏托盘（CHG-008） | `app::system::tray` | `app::{mode,state}`（两支关窗、意图兑现）、`views::settings`（那句说明） |
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

**P6 实现后的校正（2026-11-03，CHG-005）**：同样保留 P2 结论不动，四处回写正文。① 第 2 节 `ice-vision` 表按实际代码重列并加"状态"列：设计里的 `merge_lines_into_blocks()` / `RowKey` 没落成，分组成为 `RowObservation` + `flip_rect()` + `merge_rows()` 三件，新增 `import` 一行（三条入口汇同一个准入判定），`bg`/`font`/`composite` 三行明写仍是 P7；`ice-app` 表里 `state`、`jobs`、`views::mod`、`views::image`、`system::clipboard` 五行按 P6 事实改写，新增 `system::panel` 一行。② 第 3.2 节末尾登记三条偏差：入口是四条不是三条、没有 `CancellationToken`、识别段是五态不是七态。③ 第 4 节"取消"那一行按 Q-008 改掉原设计，并补出"一次识别整段待在同一条工作线程""回投用 `try_send`""面板走异步回调 + 一条 `mpsc`"三条。④ 第 5 节 vision 那三行补成 8 个变体的完整映射（`image_failure_text` 穷举，自检项 4 继续成立）。以上均不改动任何 REQ。

落地进度与本文的对应关系（本文描述 v1.0 全貌，不要求 P1 就全部存在）：

| 状态 | 模块 |
|---|---|
| 已实现并有单测 | `ice-core` 的 7 个业务模块（45 项，含 `config` 的供应商预设与有效值规则）；`ice-providers` 的 `engine`/`request`/`cache`/`dispatcher`/`error`/`machine`/`ai`/`protect`（51 项，HTTP 行为由本地假服务覆盖；`dispatcher` 用 mock 引擎覆盖选路、缓存与可用性）；`ice-app` 的窗口形态与置顶控制（3 项无头断言） |
| 已实现、靠 `#[ignore]` 实测 | `ice-providers/tests/live.rs`（3 项：机翻短文本、机翻长文本切块、AI 真实通道）——需要真实网络与钥匙串里的 Key，不进默认测试集 |
| 只有接口与桩，尚无单测 | `ice-vision` 的 `composite`，以及从未建模块的 `bg`/`font`（P7 填充并补测）；`ocr`/`error`/`layout`/`import` 已在 P6 落成并有单测（见下） |
| P4/P5 落地（P5 = CHG-004，2026-10-27 收口） | `state`、`mode`（含吸附/停靠/悬停唤出/拖动静止判定/失焦降透明）、`views::{mod,full,history,settings,compact}`、`jobs`、`system::{clipboard（含 `Watcher`）,hotkeys,secrets,single_instance,screen}` |
| P6 落地（CHG-005，2026-11-03） | `ice-vision` 的 `vision_sys`、`layout`、`import`、`ocr`（`recognize_with_rows` 那条链）；`ice-app` 的 `views::image`（由占位填实）、`system::panel`、`system::clipboard::read_image`、`state` 的图片工作区、`jobs` 的 `spawn_blocking` 那一支。**跨平台那一半由源码扫描静态成立**（`ice-vision/src/lib.rs` 的门控核查），真机 Linux/Windows 验证仍排 P8 |
| CHG-008 落地（2026-11-04，v0.2；W-2.40 的托盘那一组已定论） | `ice-app` 的 `system::tray`（菜单栏那枚图标本体）、`mode::{hide, reveal}` 的两支、`state::TrayState` 这一个运行态字段、`views::settings` 的说明句；`ice-providers` 的 `machine` 响应 null 容错（缺陷 D-20，同轮） |
| 2026-11-04 走查轮（缺陷 D-21/D-22 + 视觉变更 V） | `app::logic` 接管托盘泵（`pump_tray`，D-21）、`system::tray` 的 `pop_menu()` 与四项菜单、`Action::OpenSettings`、`glyph()` 改成应用图标那两只气泡；`mode::walked_away` + `DOCK_HOVER_PAD` 与 `system::mouse::cursor()` + `system::screen::flip_point`（D-22）；`theme` 的冷灰白基底（`COOL_*` 六档 + `pane_shadow` + `knob_tones`）、`views::mod::draw_knob` 三层圆盘（变更 V） |
| 2026-11-04 夜间轮（缺陷 **D-23** 整窗底色 + **D-24** 铺满护栏；改动只在本地提交，等用户逐条批准） | **D-23**：`ice-app` 新增 `system::background::set`（`NSWindow.setBackgroundColor:`，与 `system::alpha` 共用 `ns_window` 和"值变了才下发"那条缓存，泛化后叫 `needs_native_update`）、`theme::window_base(dark)`（两级分层的第一级，同一个数喂原生与 `window_fill`/`panel_fill`），`objc2-app-kit` 的 feature 只多开 `NSColor` 一个模块。**D-24**：`state::filled: bool` → `state::fill: Option<Rect>`（存"铺的是哪块可用区"），`mode::capture` 的铺满支从"整帧早退"改成"照量、只是不写槽"并自己续帧，`app::refresh_geometry` 的重发按 `state.fill` 那块原始可用区算而不是从配置现推；铺满那条日志加 `area=?` |
