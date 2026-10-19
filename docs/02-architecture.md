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
| `machine` | `MachineTranslateEngine` | 零配置机翻通道，超时与重试 | FR-1.3、FR-1.7、NFR-1 |
| `ai` | `AiEngine`、`AiEndpoint` | OpenAI 兼容 chat 通道，提示词构造、分段与拼接 | FR-1.3、FR-1.9 |
| `protect` | `ProtectedText`、`protect()`/`restore()` | URL/邮箱/代码块/标识符/带单位数字的占位符保护 | FR-1.6 |
| `cache` | `TranslationCache`、`CacheKey`、`CachedValue` | LRU；AI 通道以术语表版本号加盐 | FR-1.8、FR-4.5 |
| `dispatcher` | `Translator` | 选引擎→查缓存→调用→术语后处理→回写缓存；`availability()` | FR-1.3、FR-1.7、FR-1.8 |

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

| 模块 | 关键类型 | 职责 | 需求 |
|---|---|---|---|
| `app` | `IceApp` | 实现 `eframe::App`；持有 `AppState`、`Translator`、tokio runtime handle | 全部 |
| `state` | `AppState` | 源文/译文/语言对/引擎/任务状态/图片工作区/历史查询 | FR-1、FR-2 |
| `mode` | `ModeController`、`DockState`、`Morph` | 模式切换与形变动画、无边框拖动（`StartDrag`）、置顶、边缘吸附与热区、几何持久化 | FR-3.2~3.9 |
| `views::full` | `FullView` | 全量布局：页签（文本/图片/历史/设置） | FR-3.1 |
| `views::compact` | `CompactView` | 窄条布局：输入行 + 译文行 + 动作图标 | FR-3.2 |
| `views::image` | `ImageView` | 图片工作区：识别结果列表 + 原图/结果对比 + 导出 | FR-2.3、FR-2.6 |
| `views::settings` | `SettingsView` | 引擎参数、密钥写入钥匙串、语言、主题、快捷键说明 | FR-4.1~4.2 |
| `jobs` | `JobScheduler`、`JobKind`、`JobMsg` | `spawn` 网络任务、`spawn_blocking` 图像任务，经 `mpsc` 回投 UI 并请求重绘 | FR-2.9、NFR-1 |
| `system` | `ClipboardWatcher`、`HotKeys`、`Secrets` | arboard 轮询剪贴板、应用内快捷键、钥匙串读写 | FR-3.8、FR-5.1~5.2、FR-4.2 |

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
- `protect` 失败（占位符被模型改写）时降级为"不保护直接返回原文"并在界面提示，绝不返回被破坏的文本。

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
- **形变动画**（FR-3.9）：`ModeController` 持有一个 `Morph` 状态（起止矩形、已用时长、ease-out 曲线），每帧 `request_repaint()` 并下发一次 `InnerSize`/`OuterPosition`；`Decorations` 与 `WindowLevel` 属于原生属性，只在时间轴 35% 处各下发一次。动画不改变任何业务状态，中途再次切换则把当前插值位置作为新起点反向播放。
- **几何持久化时机**（P1 实测：SIGTERM/强杀不走 `on_exit`，只靠退出保存会丢位置）：在①拖动结束（吸附判定完成时）②形态切换动画结束时③关窗/退出时 三处写盘，均为小文件（<1KB）原子替换，代价可忽略。

## 4. 并发模型

| 场景 | 机制 | 原因 |
|---|---|---|
| HTTP 翻译 | tokio `spawn`（多线程 runtime，由 `ice-app` 持有） | 不阻塞渲染 |
| OCR / 图像合成 / 字体加载 | `spawn_blocking` | Vision 与像素操作是 CPU 阻塞调用 |
| 结果回投 UI | `mpsc::unbounded` + `ctx.request_repaint()` | eframe 事件驱动，必须主动唤醒 |
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
| vision | `UnsupportedPlatform` | 图片页签禁用并说明"当前平台不支持" |
| | `NoTextFound` | 提示"未发现文字"，不算失败 |
| | `Font(msg)` | 结果区顶部警示条："缺字形，已回退" |
| core | `ConfigParse` | 启动时黄色提示条（应用仍能起来，FR-4.3） |

规则：错误一律向上带**人话文本**（thiserror `#[error]`），UI 不做 `match` 猜含义；只有需要"给出动作按钮"的分类才在 UI 层 `match` 具体变体。

## 6. 配置生命周期

```
main() ──► Config::load() ──► (cfg, Option<reason>)
             │ 不存在 → 默认值           │ 损坏 → 默认值 + reason（界面提示）
             ▼
        cfg.sanitized()  ──►  构建 AppState / ModeController
             │
             │ 运行中：任何改变即时更新内存副本
             ▼
        退出前 / 关窗时 save()（原子替换）
```

`recent_targets` 在切换目标语言时更新（`remember_target`），最多 5 条（FR-1.4）。

## 7. 需求 → 模块映射（节选，完整矩阵见 `traceability.md`）

| 需求 | 主责模块 | 协作 |
|---|---|---|
| FR-1.3 双引擎 | `providers::dispatcher` | `providers::{machine,ai}` |
| FR-1.6 内容保护 | `providers::protect` | — |
| FR-2.2 本地 OCR | `vision::{ocr,vision_sys}` | `core::document` |
| FR-2.4/2.5 回填与覆盖层 | `vision::composite` | `vision::bg` |
| FR-3.4 置顶 | `app::mode` | `core::config::UiConfig` |
| FR-3.5 边缘吸附 | `app::mode::snap`（纯函数） | `core::config::WindowRect` |
| FR-4.2 钥匙串 | `app::system::Secrets` | `core::config` 常量 |
| NFR-6 分层 | 全 workspace | `cargo tree` 校验 |

## 8. 设计自检（P2 验收）

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 每条 Must 需求有主责模块 | ✅ 见第 7 节与 `traceability.md` |
| 2 | 无"设计了但没人调用"的模块 | ✅ 骨架只落地被调用的接口；回写侧的中间抽象（块级排版计划等）等 P7 有真实文字测量需求时再定义，不预先设计 |
| 3 | 并发与取消语义明确 | ✅ 第 4 节 |
| 4 | 错误到界面的映射穷举 | ✅ 第 5 节，覆盖全部已定义变体 |
| 5 | 关键取舍有 ADR | ✅ ADR-001/002/003/005/006；ADR-004（机翻端点）待 P3 实测后补 |
| 6 | 可测试性：纯函数与 IO 分离 | ✅ 吸附计算、几何合并、缓存、术语替换均为纯逻辑 |

**验收结论（P2）**：待补——需在骨架编译通过后正式签发。
