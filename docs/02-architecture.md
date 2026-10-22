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
| `config` | `Config`、`UiConfig`、`TranslationConfig`、`AiConfig`、`WindowRect`、`UiMode`、`DockEdge`、`Theme` | 配置读写（原子替换 + 损坏回退 + sanitize 夹取） | FR-3.6、FR-4.1~4.3 |
| `history` | `HistoryRecord`、`HistoryStore` | JSONL 追加、容量裁剪、搜索、坏行跳过 | FR-4.4 |
| `glossary` | `Term`、`Glossary` | 术语读写、`prompt_fragment()`、`post_substitute()` | FR-4.5 |
| `detect` | `guess_language()` | 按字符集构成做语言粗判，用于自动检测回显 | FR-1.2 |

### ice-providers

| 模块 | 关键类型 | 职责 | 需求 |
|---|---|---|---|
| `engine` | `TranslationEngine`（trait）、`unavailable_error()` | 通道抽象：`kind/translate/is_available` | FR-1.3 |
| `request` | `TranslationRequest`、`TranslationResult` | 通道间唯一的入参/出参结构：语言对、术语提示、分段续译的上下文尾巴、耗时与命中来源 | FR-1.1、FR-1.3、FR-1.9 |
| `machine` | `MachineTranslateEngine`、`MACHINE_QUERY_CHARS` | 零配置机翻通道（MyMemory），单次查询 450 字符上限、超时与网络类重试 ≤2 | FR-1.3、FR-1.7、NFR-1 |
| `ai` | `AiEngine` | OpenAI 兼容 chat 通道，提示词构造、分段与上下文续译、拼回 | FR-1.3、FR-1.9 |
| `protect` | `ProtectedText`、`Restored`、`protect()`/`restore()`、`chunk_chars()` | URL/邮箱/代码块/标识符/带单位数字的占位符保护，以及"优先在换行或句末断开、不切开占位符"的切块 | FR-1.6、FR-1.9 |
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
| `app` | `IceApp`、`Effects`、`Prepared` | 实现 `eframe::App`；持有 `AppState`、`JobRunner`、tokio runtime、历史存储、配置路径、单实例锁、钥匙串探测回收端；帧末统一执行副作用与退避落盘 | 全部 | ✅ P4 |
| `main` | `prepare()` | 启动时序：日志 → 单实例锁 → 读配置 → 导入旧术语表 → 装配引擎 → tokio → 建窗 → **后台**读钥匙串 | FR-4.1~4.3、REQ-009 | ✅ P4 |
| `state` | `AppState`、`Failure`、`FailureAction`、`Job`、`Outcome` | 源文/译文/语言对/引擎/在飞任务/失败态/图片工作区/历史查询/提示条；`request_id` 竞态丢弃 | FR-1、FR-2 | ✅ P4 |
| `mode` | `hide()`、`quit()`、`apply_topmost()`、`capture_rect()`、`initial_viewport()` | 模式切换的 `ViewportCommand` 编排、置顶、几何捕获与持久化；边缘吸附待 P5（FR-3.9 已于 2026-10-22 改为瞬时切换，无形变动画） | FR-3.2~3.9 | 部分（P4 只到切换与置顶） |
| `theme` | `apply()` | 三态主题映射到 `ctx.set_theme`，字号缩放乘在 egui 默认值上并限幅 | NFR-4、NFR-8 | ✅ P4 |
| `views::mod` | `Tab`、`chrome()` | 顶部工具条 + 页签 + 状态条（设计里归在 `views::full`，实现时抽出共用，简洁模式也要用同一条状态条） | FR-3.1 | ✅ P4 |
| `views::full` | `show()`、`copy()` | 全量布局文本页：源文编辑器、译文区、翻译与两个复制按钮、失败动作按钮 | FR-1.1~1.5、FR-3.1 | ✅ P4 |
| `views::history` | `show()` | 历史页：搜索、点选回填、时间戳本地化、坏行容错 | FR-4.4 | ✅ P4 |
| `views::settings` | `show()`、`select_default_engine()` | 供应商预设、密钥写钥匙串、主题三态、字号、额外语言、术语分区 | FR-4.1~4.2、FR-4.5、NFR-4 | ✅ P4 |
| `views::image` | `show()` | 图片页占位：五个区域已画好，接 OCR 在 P6 | FR-2.3、FR-2.6 | 占位 |
| `views::compact` | — | 窄条布局：输入行 + 译文行 + 动作图标、无边框拖动（`StartDrag`） | FR-3.2 | 待 P5（P4 暂以 `app.rs` 私有函数代管切换后的窗体编排） |
| `jobs` | `JobRunner`、`drain()`、`apply()` | `spawn` 网络任务，经 `mpsc` 回投 UI 并请求重绘；`spawn_blocking` 图像任务待 P6 | FR-2.9、NFR-1、REQ-002 | ✅ P4（文本部分） |
| `system::clipboard` | `CopyKind`、`compose_bilingual()` | 复制译文 / 复制对照；空译文保护。`ClipboardWatcher`（剪贴板即翻）待 P5 | FR-1.5、FR-3.8 | 部分 |
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
   │                              │
   │  ViewportCommand:            │  进入时一次性下发：
   │  InnerSize, WindowLevel      │  Undecorated=true, AlwaysOnTop=true,
   │  Position, Visible           │  InnerSize=(460,96), Resizable=false
   ▼                              ▼
 拖拽结束 ──► ModeController.snap(rect, monitors)
                 │ 距最近边缘 < snap_threshold_px ?
                 ├─ 是 → 贴边：位置 = 边缘 - (尺寸 - dock_sliver_px)，记录 DockEdge
                 └─ 否 → DockEdge::None，自由浮动
              ──► 写回 Config.ui.{mode,full_rect|compact_rect,docked_edge} ──► 落盘
```

- 吸附计算是 `ice-core`/`ModeController` 的纯函数：输入（窗口矩形、监视器可用区、阈值、热区）→ 输出（目标矩形 + 边）。因此可以被单测覆盖，不需要真的拖动窗口。
- 启动时若 `Config` 记录的窗口位置不在任何监视器内（拔屏场景），用 `WindowRect::centered_in` 回到主屏中心（FR-3.6）。
- **无边框窗口的拖动**（FR-3.2）：简洁模式没有系统标题栏，把手区按下时下发 `ViewportCommand::StartDrag`，由窗口管理器接管移动。好处是不用在应用内自算鼠标增量、不会和吸附判定抢事件；代价是拿不到"拖动中"的连续回调，因此吸附判定放在拖完后的首个 `Moved`/`Focused` 帧上做一次。
- **形态切换的几何下发**（FR-3.9，2026-10-22 由"形变动画"改为瞬时）：切换时在同一帧内一次性下发 `Decorations(目标形态)` → `InnerSize(目标尺寸)` → `OuterPosition(目标位置)`，没有 `Morph` 状态、没有逐帧补间、切换后不再 `request_repaint()`。`Decorations` 与 `WindowLevel` 是原生属性，本来就只能整段设一次（逐帧设会闪烁）。下发的是**请求**而非同步生效（`InnerSize` 走 `request_inner_size`，异步），因此切换完的下一帧必须用 `mode::capture` 回读真实几何再落盘，不能拿目标值当已生效值。
- **几何持久化时机**（P1 实测：SIGTERM/强杀不走 `on_exit`，只靠退出保存会丢位置）：在①拖动结束（吸附判定完成时）②形态切换完成时③关窗/退出时 三处写盘，均为小文件（<1KB）原子替换，代价可忽略。

## 4. 并发模型

| 场景 | 机制 | 原因 |
|---|---|---|
| HTTP 翻译 | tokio `spawn`（多线程 runtime，由 `ice-app` 持有） | 不阻塞渲染 |
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
| core | `ConfigParse` | 启动时黄色提示条（应用仍能起来，FR-4.3） |

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

**与封存设计的两处偏差**：① 设计写"当帧结束就 save"，实现改为退避 500ms——字号滑块这类连续控件每帧都会标脏，逐帧写盘会把原子替换变成每帧一次 rename；② 设计没有"钥匙串探测回来再对齐 `key_configured`"这一步，因为当时假定 Key 在建窗前就读到了（该假定已被 P4 真机推翻，见第 4 节）。两者都不改任何 REQ。

## 7. 需求 → 模块映射（节选，完整矩阵见 `traceability.md`）

| 需求 | 主责模块 | 协作 |
|---|---|---|
| FR-1.3 双引擎 | `providers::dispatcher` | `providers::{machine,ai}` |
| FR-1.6 内容保护 | `providers::protect` | — |
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
| P4/P5 落地 | `state`、`mode`、`views::*`、`jobs`、`system` |
| P6/P7 落地 | `vision_sys`、`layout`、`bg`、`font` |
