# CHG-005 · 图片翻译 P6：macOS Vision 本地识别、行合并与可编辑识别结果

## 原始意图与来源

- 立项要求（2026-10-19，用户原话）："写个 rust 跨平台的客户端软件。哦，别忘了做两个界面……优先针对 mac。" `docs/00-charter.md:66` 把 P6 定为 `ice-vision` 的 OCR 阶段，验收判据原文是"**样图集 OCR 字符正确率达标并留档**"。
- 需求基线里本包承担的是三条 Must（`docs/01-requirements.md:46-48`，逐字）：
  - **FR-2.1**（M）"四种图片来源：拖拽图片文件、粘贴剪贴板图片、文件选择器、指定路径"，验收"每种路径各导入一张图，均能进入 OCR"；
  - **FR-2.2**（M）"本地 OCR（macOS Vision）：输出带**坐标框、文本、置信度**的块级结果，不做任何网络上传"，验收"对样图集运行，日志与网络抓包证明零外发请求；每块有 bbox 与 conf"；
  - **FR-2.3**（M）"识别结果以可编辑列表呈现，用户可修正错字、删除误识别块、合并/拆分块，修正后再翻译"，验收"手改一处错字，生成图反映修改"。
  这三条之间有一条**本包必须自己说清的切分**：FR-2.3 的验收判据后半句（"生成图反映修改"）落在 P7 的回填，本包只能兑现"修改确实进了将要交给 P7 的那份数据"，判据的前半句改由 `OcrDocument` 侧的断言承担，见 REQ-005 的"验证层次"与 REQ-008。
- 架构决定：`ADR-002`（v1 只接 Vision：`VNRecognizeTextRequest`、`revision >= 3`、`usesLanguageCorrection`、语言候选 `zh-Hans`/`en-US`/`ja`；非 macOS 返回 `VisionError::UnsupportedPlatform`；坐标是**归一化、原点左下**，必须在视觉层单独转换并单独写单测；调用在 `spawn_blocking` 的工作线程内完成，**不跨线程传 objc 对象**）。`ADR-003`（擦除 + 覆盖层兜底）整条属 P7，本包不做也不验收。
- 流水线来源：`docs/04-image-pipeline.md` 的 §1 阶段表（S0 载入 / S1 识别 / S2 坐标转换 / S3 行合并是本包的四段）、§2 行合并五条判据与 `MergeParams` 默认值、§5 度量表（"OCR 字符正确率 ≥ 0.92（排除艺术字与照片背景两类）"是本包唯一那条可自动计算的指标）。
- 前置状态：CHG-004（P5 简洁悬浮窗）仍在 IMPLEMENTING，本包**不带任何窗口/形态/吸附内容**——它的入场清单是 CHG-004 非目标表第 29 行主动划出来的："图片翻译全链路：OCR、识别结果编辑、擦除回填、覆盖层、导出（FR-2.1~2.10）| P6/P7 = CHG-005/006"。代码态（2026-10-28，提交 `1f8b824`）：440 passed / 0 failed / 5 ignored（ice-app 320 · ice-core 58 · ice-providers 62）。
- 本包的形成背景（2026-10-28，用户原话）："看看还有哪些没有改动的，今晚你列出计划一个个执行，没有执行完成不要关机。" 已发布的执行顺序第 ④ 项就是本包，且口径写明**只出规格包，不动 P6 业务代码**——批准动作收敛成一条 `./sdd approve`。

## 目标与非目标

**目标**：把图片页从 P4 留下的静态占位版式（`views/image.rs:4-5` 明写"这里刻意不放任何可点控件"）长成一条真能跑的**识别链**：四个入口都能把一张图变成待识别的像素缓冲（REQ-001）→ 本机 Vision 认出行级观察，带文本、置信度与归一化框，全程零外发（REQ-002）→ 归一化左下坐标翻成左上像素坐标（REQ-003）→ 行按五条判据合并成语义块，含表格防护与块上限（REQ-004）→ 识别结果以可编辑列表呈现，改字 / 删块 / 勾选 / 合并 / 拆分五种动作都作用在将要交给 P7 的那份 `OcrDocument` 上（REQ-005）→ 识别期间界面不冻、状态与耗时可见、用户点"取消"后不产出任何结果（REQ-006）→ 非 macOS 与拿不到系统框架时明确降级而不是崩溃（REQ-007）→ 一套可自动跑的度量与隐私取证，把"达标"这件事和"只有用户能定论"的那部分分开写清（REQ-008）。

**非目标**（各自有归属，本包不做也不验收）：

| 不做 | 归属 |
|---|---|
| 背景/前景估计、原文擦除、译文回填、覆盖层、字号自适应、颜色与对齐还原、导出（FR-2.4~2.8） | P7 = CHG-006（`ice-vision::composite` + `bg`，docs/04 §3/§4） |
| 三阶段进度与取消的**完整**语义（OCR / 翻译 / 合成三段各自推进与各自可取消，FR-2.9） | P7 = CHG-006；本包只做其中"识别"那一段（REQ-006 写清切分） |
| 把识别出的文字送去翻译（复用 P3 的引擎与 P5 的派发） | P7 = CHG-006：块 → `TranslationRequest` 的那条接线在回填侧才有意义 |
| 区域截图后直接进图片流程（FR-2.10，Could） | 延期项（`docs/01-requirements.md:55` 已标"列入延期项"） |
| 非 macOS 的 OCR 后端（Windows.Media.Ocr、tesseract） | 不做：`OcrEngine` trait 已为它们留位（`ice-vision/src/ocr.rs:18-27`），但 NFR-7 只要求"明确提示 + 文本功能不受影响" |
| HEIC/WebP 等 `image` 未开解码器的格式 | 见 Q-005：默认只承诺 PNG/JPEG（+ 剪贴板 TIFF），其余格式给出"不支持"的明确提示并登记为已知限制 L-7；要扩格式走系统 ImageIO，属你的决定 |
| 图片持久化、历史记录里出现图片、把 `OcrDocument` 写进配置目录 | 不做：识别结果是运行态，重开软件就没了（REQ-005 边界明写） |
| 主题包（#28）、界面多语言（#23）、滑块可读性、图钉角度等 P5 遗留待裁定项 | 各自待办，与本包无关 |

本包**不改**文本翻译链路、不改密钥边界（ADR-005：Key 只走钥匙串，日志只有 `key_len` + `key_hash8`）、不改配置持久化格式（**不新增任何配置字段**：识别参数、合并参数、置信度提示阈值全部是代码里的常量与默认值，见 Q-006）。

## 事实与假设

| 标签 | 结论 | 来源 / commit / 环境 / 时间 |
|---|---|---|
| FACT | **视觉层是骨架，不是半成品**：`ice-vision` 共 168 行四个文件，`VisionOcr::recognize()` 与 `Renderer::compose()` 都直接返回 `VisionError::NotImplemented`，`Cargo.toml` 里 Vision 依赖整段是注释（"P6 接入 macOS Vision 时再启用"）。但**接口形状已经定好且不需要改**：`OcrEngine` trait（`name` / `recognize(&RgbaImage) -> VisionResult<OcrDocument>` / `is_supported`）已带 `Send + Sync` 超约束，`native_ocr_supported() = cfg!(target_os = "macos")` 已存在，八个错误变体（含 `NoTextFound`、`TooLarge`、`Decode`）已覆盖 docs/04 §1 失败列的措辞 | `crates/ice-vision/src/{lib.rs,ocr.rs,composite.rs,error.rs,Cargo.toml}`；查阅 2026-10-28 |
| FACT | **领域数据结构够用**：`ice_core::document` 已有 `PixelRect`（含 `intersects_horizontal`/`vertical_gap`/`union`/`scaled` 四条行合并要用的几何方法）、`TextBlock{id,text,bbox,polygon,confidence,translation,degraded,enabled}` 与 `OcrDocument{width,height,blocks,source_label}`（含 `enabled_blocks`/`merged_text`）。REQ-004 的合并算法要的三条比较（左边界对齐、垂直间隙、水平重叠）**全部有现成方法**，不需要新写几何 | `crates/ice-core/src/document.rs:5-144`；查阅 2026-10-28 |
| FACT | **"拆分回原行"目前拿不到原料**：`OcrDocument` 只存合并后的块，**不存 Vision 返回的行级观察**。FR-2.3 的"拆分块"要能把一个块拆回它由哪几行组成，因此块必须携带行号回溯（运行态字段，不进配置 schema）。这是 REQ-005 的实现前提，也是本包对 `ice-core::document` 唯一一处结构改动 | `crates/ice-core/src/document.rs:79-120`（`TextBlock` 无行级来源字段）；docs/01 FR-2.3 的"合并/拆分" |
| FACT | **`objc2-vision` 存在且与本仓现有 objc2 版本兼容**：crates.io 索引里最新为 **0.3.2**，按类开关 feature（含本项目需要的 `VNRecognizeTextRequest`、`VNRequestHandler`、`VNObservation`、`VNRequest`、`VNTypes`、`VNDefines`、`VNError`，共 55 个 feature，**没有 default**）；依赖约束 `objc2 >=0.6.2,<0.8`、`objc2-foundation ^0.3.2`，与本仓已锁的 `objc2 0.6.5` / `objc2-foundation 0.3.2` 一致，`block2 >=0.6.1,<0.8` 也已满足（P5 任务 #25 刚引入 0.6） | `curl https://index.crates.io/ob/jc/objc2-vision` 的 0.3.2 条目（features 与 deps 逐条读出）；`Cargo.lock` 现有版本；2026-10-28 实测网络可达（`index.crates.io=200`、`static.crates.io=200`） |
| FACT | **要调的 Vision API 在绑定里都有、签名已核对**（把 crate 解包到 `/tmp` 逐条读）：`VNRecognizeTextRequest::new()`、`setRecognitionLevel(VNRequestTextRecognitionLevel{Accurate,Fast})`、`setRecognitionLanguages(&NSArray<NSString>)`、`setUsesLanguageCorrection(bool)`、`setAutomaticallyDetectsLanguage(bool)`（注释明写"仅 `VNRecognizeTextRequestRevision3` 起有效，之前是 no-op"）、`results() -> Option<Retained<NSArray<VNRecognizedTextObservation>>>`；`VNRecognizedTextObservation::topCandidates(max_candidate_count: NSUInteger) -> Retained<NSArray<VNRecognizedText>>`（**不返回 Result**，注释：最多 10 条、可按更少返回）；`VNObservation::boundingBox() -> CGRect`（归一化坐标）；`VNRecognizedText::string() -> Retained<NSString>`、`confidence() -> VNConfidence`；`VNImageRequestHandler::initWithCGImage_options(...)`、`performRequests_error(&NSArray<VNRequest>) -> Result<(), Retained<NSError>>` | `/tmp/ice-vision-probe/objc2-vision-0.3.2/src/generated/{VNRecognizeTextRequest.rs:72-197,VNRequestHandler.rs:161/418,VNObservation.rs:150/1007/1013/1076}`；2026-10-28 解包核对 |
| FACT | **一次识别必须整段待在同一条线程**：整个 crate 里**没有任何** `unsafe impl Send`/`unsafe impl Sync`（对 `VNObservation.rs`、`VNRequestHandler.rs`、`VNRecognizeTextRequest.rs` 全文 grep 为空）→ `Retained<VNImageRequestHandler>` 与 `Retained<VNRecognizeTextRequest>` 在 Rust 类型系统里是 `!Send`。因此 handler + request 只能在 `spawn_blocking` 的那个闭包里创建、执行、并在闭包内把结果读成纯 Rust 的 `OcrDocument` 再返回——**这不是保守写法，是类型逼出来的唯一形状**，也正是 ADR-002 "不跨线程传 objc 对象"那句话的机器可验形式。生产侧前提已具备：主运行时是 `Builder::new_multi_thread()`（`main.rs:218`），且 `runtime.spawn_blocking` 这条路在本机已由钥匙串探测跑通（`app.rs:66`） | objc2-vision 0.3.2 源码 grep；`crates/ice-app/src/main.rs:218`、`app.rs:54-70`；ADR-002 §影响 |
| FACT | **像素缓冲可以直接变成 `CGImage`，不需要先编码成文件**：`objc2-core-graphics 0.3.2`（**本机 registry 缓存里已经有**，无需联网）暴露 `CGImage::new(width, height, bits_per_component, bits_per_pixel, bytes_per_row, Option<&CGColorSpace>, CGBitmapInfo, Option<&CGDataProvider>, *const CGFloat, bool, CGColorRenderingIntent) -> Option<CFRetained<CGImage>>`（即 `CGImageCreate`），且它要求的 `CGColorSpace` 与 `CGDataProvider` 两个 feature 都在该 crate 的 **default** 列表里。`initWithCGImage_options` 就是它的对口入口 | `~/.cargo/registry/src/…/objc2-core-graphics-0.3.2/src/generated/CGImage.rs:218-255`（含 `#[cfg(all(feature = "CGColorSpace", feature = "CGDataProvider"))]`）与其 `Cargo.toml` 的 default features；2026-10-28 核对 |
| FACT | **拖拽入口是现成的，且"拖进来时高亮"也有原料**：egui-winit 0.37 把 `WindowEvent::HoveredFile` 写进 `egui_input.hovered_files`、`HoveredFileCancelled` 清空它、`DroppedFile(path)` 清空 hovered 并把 `Arc::new(NativeFile::from(path))` 推入 `dropped_files`（`egui-winit-0.37.0/src/lib.rs:513-545`，且 `#[cfg(not(target_arch = "wasm32"))]` 那条正是桌面路径）。注意它给的是**路径**不是字节，所以拖拽后仍要 `std::fs::read` + 解码 | 同上；2026-10-28 核对 |
| FACT | **剪贴板图片不需要动 Cargo.toml，但解码走的是 TIFF**：arboard 3.7 的 `get_image()/set_image()` 挂在 `image-data` feature 后面，而 **`image-data` 就在 arboard 的 `default` 里**（其 `Cargo.toml`：`default = ["image-data"]`）——本仓 `arboard = "3.7"` 一行没改就已经能用。macOS 侧实现（`platform/osx.rs:223-241`）读 `NSPasteboardTypeTIFF`，再用它自己那份 `image` 以 `ImageFormat::Tiff` 解码并 `into_rgba8()`。两个连带事实：① 我们**读到的图必然是 TIFF 解出来的**，与本项目 `image` 的 feature 集无关；② 若 ice-vision 自己也想解 TIFF（例如把剪贴板那条路径统一走我们的解码器），`tiff 0.11.3` **已在 `Cargo.lock` 里**（被 arboard 的 image 依赖拉进来），给 ice-vision 开 `image` 的 `"tiff"` feature 不新增任何传递 crate | `~/.cargo/registry/src/…/arboard-3.7.0/{Cargo.toml,src/lib.rs:129-148,src/platform/osx.rs}`；`Cargo.lock:4125-4136`；2026-10-28 核对 |
| FACT | **文件选择器不需要新 crate**：`objc2-app-kit 0.3.2`（本仓已依赖，只是没开这些 feature）里有 `generated/NSOpenPanel.rs`（`URLs() -> Retained<NSArray<NSURL>>`、`canChooseFiles` 等），而 `runModal() -> NSModalResponse` 与 `beginWithCompletionHandler:` 挂在它的父类 `NSSavePanel`（`NSSavePanel.rs:423/435`）。`NSOpenPanel` feature 需要 `NSArray/NSCoder/NSGeometry/NSObject/NSString/NSURL/objc2-core-foundation`——本仓的 `objc2-foundation` 目前只开 `NSGeometry/NSObject/NSString/NSAffineTransform/NSArray`，因此**要补 `NSURL`（外加 app-kit 侧的 `NSOpenPanel`/`NSSavePanel`/`NSPanel`）**。`block2` 已经为任务 #25 打开，异步那条 `beginWithCompletionHandler:` 的 block 参数有着落 | `~/.cargo/registry/src/…/objc2-app-kit-0.3.2/src/generated/{NSOpenPanel.rs,NSSavePanel.rs}` 与其 `Cargo.toml` features；`crates/ice-app/Cargo.toml` 的 feature 清单；2026-10-28 核对 |
| FACT | **本仓解码器只认 PNG 与 JPEG**：workspace 的 `image = { version = "0.25", default-features = false, features = ["png", "jpeg"] }`（`Cargo.toml:27`）→ BMP/GIF/WebP/TIFF/HEIC 都解不了；`DynamicImage::apply_orientation` 存在（`image-0.25.10/src/images/dynimage.rs:1161`）但需要解码器提供 EXIF 方向。系统框架侧另有路：`CGImageSourceCreateWithData`（`objc2-image-io`，是 objc2-vision 0.3.2 的一个可选依赖）能解 macOS 支持的全部格式（含 HEIC）——这是 Q-005 那条分叉的技术前提 | `Cargo.toml:27`、`crates/ice-app/Cargo.toml`；2026-10-28 核对 |
| FACT | **"样图集达标"这件事现在拿不到原料**：`docs/06-test-cases.md:219-236` 定义了 IMG-1~IMG-12 十二张样图与"排除 IMG-9/12 后平均字符正确率 ≥ 0.92"的判据（`TC-2.2-a`），但 `assets/samples/` 目录**是空的**（2026-10-28 实测 `ls assets/samples` 无输出，只有 `assets/icon/`），也没有任何人工标注文件。真实截图只能来自这台机器（报错弹窗、PPT、说明书、深色模式截图……），不是我能凭空造的 → 这条判据的**真实数字**属"只有用户能定论"的部分，本包据此把度量拆成两层（REQ-008），并把样图来源写成 Q-003 + 停止条件 1 | `docs/06-test-cases.md` §4；`assets/` 目录实测；docs/04 §5 度量表 |
| FACT | **取消一次正在跑的识别在类型上做不到**：`VNRequest::cancel()` 在绑定里存在（`generated/VNRequest.rs:141-144`，注释"Tries to abort the request as soon as possible. Results will be nil"），但要从界面线程调它就得把 `Retained<VNRecognizeTextRequest>` 交给另一条线程，而上一行 FACT 已确证这些类 `!Send`；`performRequests_error` 本身是同步阻塞（crate 文档注释原话："performRequests is not an asynchronous method"）。所以本包的"取消"**只能是丢弃结果**：不再回灌界面、不再产出任何下游动作，正在跑的这一次自己跑完（视觉上是即时响应，代价是那百毫秒级 CPU 照旧花掉）。这条边界写进 REQ-006 与 Q-008，不在文档里含糊成"可取消" | objc2-vision 0.3.2 `generated/VNRequest.rs:141-144`、`VNRequest.rs:16`（类文档注释）、`VNRequestHandler.rs:405-422`；2026-10-28 核对 |
| FACT | **占位页的既有测试会在本包被替换，不是被扩充**：`views/image.rs` 的 4 条测试断言的是"五区渲染不 panic""不改状态""脚注显示语言对"，其中 `placeholder_page_renders_the_five_regions_without_panicking` 与 `placeholder_page_never_changes_state` 的前提（"没有可点控件"）在 P6 必然作废——模块注释第 4-5 行写明了当初为什么不放控件（"走查时点到没反应的按钮只会浪费验收时间"），本包正是那条注释说的"P6/P7 接入 `ice-vision`"的时刻。改动本身要在 REQ-006 里带上"占位说明文字必须消失，不许留下'尚未启用'这种过时提示" | `crates/ice-app/src/views/image.rs:1-129` 与其 `mod tests`；2026-10-28 核对 |
| FACT | **零外发在结构上是成立的，但"成立"要可取证**：`ice-vision/Cargo.toml` 的依赖只有 `ice-core/thiserror/tracing/image`（**没有 reqwest**），crate 文档注释第 3 行写"本 crate 不发起任何网络请求"；`Cargo.lock` 里 reqwest 只挂在 ice-providers。所以 FR-2.2 那句"不做任何网络上传"有两层证据：静态一层（依赖图 + 编译期 `cargo tree`）、运行时一层（TC-2.2-b 的 `nettop -p <pid>` 抓包）。两层都要在 REQ-008 里跑一遍并留档 | `crates/ice-vision/Cargo.toml`、`Cargo.lock` 的 reqwest 归属；docs/06 TC-2.2-b |
| HYPOTHESIS | **Vision 在这台机器上对真实截图能达到 ≥ 0.92 的字符正确率**：ADR-002 选它的理由是"截图类图像正是 Vision 的强项（Live Text 同源能力）"，但**本机从未跑过一次**。这条只能由真机样图定论（REQ-008 的第二层），无头测不了；若实测明显低于判据，按停止条件 1 回来找你，而不是把判据调低 | ADR-002 §理由；本机 2026-10-28 之前无 Vision 运行记录 |
| HYPOTHESIS | **一次识别的耗时可接受（单张 1080p 截图 < 1s）**：docs/04 §1 没给数字，`OcrProgress{blocks, elapsed_ms}` 这个结构就是为"要量这件事"而留的。真机上如果慢到界面必须做进度条而不是"识别中"一行字，属观感与需求侧的新信息，回来登记。无头只能测"派发不阻塞 UI 线程"（TC-2.1-d 那类手法已有） | `crates/ice-vision/src/ocr.rs:7-12`；`crates/ice-app/src/jobs.rs:402-419`（同形状的"慢任务不阻塞"断言） |
| HYPOTHESIS | **合并参数默认值（`row_gap_factor=1.3`、`align_tol_factor=0.6`、`overlap_min=0.4`、`fill_ratio_para_break=0.85`）对真实截图合理**：docs/04 §2 明写"在样图集上调，调完写进验收报告"，即**这些数就是要改的**。本包按原值实现并把每条判据写成纯函数单测，调参只改数值不改结构；若出现"某一档必须改判据形状才能合对"，属停止条件 3 | docs/04 §2；需求 FR-2.2/2.3 的质量前提 |

## 行为与验收

### REQ-001: 四种图片来源都落到同一个待识别缓冲（FR-2.1）

WHEN 用户把一张图片文件拖到窗口任意位置
THEN 系统 SHALL 从 `dropped_files` 取到该文件的**路径**（winit 给的是路径不是字节），读盘、解码、转成 `RgbaImage`，并把这条图交给识别流程；SHALL NOT 让拖拽事件同时触发任何文本粘贴语义（图片页之外拖进来图片时，本包只保证"不 panic、不误当文本"，不承诺跨页签的智能路由）。

WHEN 用户按"粘贴"（或 `Cmd+V` 落在图片页的导入区）且剪贴板里是图片
THEN 系统 SHALL 走 `arboard::Clipboard::get_image()` 拿到 RGBA 宽高并转成同一形状的 `RgbaImage`；当剪贴板里**没有**图片（只有文本，或空）时 SHALL 给一条 500ms 淡出的提示条而不是报错弹窗（沿用 P5 变更 I 的提示条规则），SHALL NOT 清空已有图片。

WHEN 用户点"打开图片"
THEN 系统 SHALL 弹出**原生**文件选择面板（`NSOpenPanel`），只允许选文件、允许可解码的格式，选中后走与拖拽同一条解码路径；SHALL NOT 用 `runModal`（那会阻塞 eframe 的帧循环、窗口看起来像死了）， SHALL 用 `beginWithCompletionHandler:` 的异步形态，回调里只把路径交回帧循环。

WHEN 用户在图片页输入一个路径（或由命令行/其他入口给出一个路径）
THEN 系统 SHALL 按路径读盘解码；路径不存在、是目录、无读权限时 SHALL 给出**一句说明原因**的提示（区分"找不到文件"与"这张图解不开"），SHALL NOT 把 OS 错误原文整段甩到界面上。

四条路径 SHALL 汇聚到**同一个**导入函数（同一个错误类型、同一个上限判据、同一条"缩放后继续处理"的分支），差异只在"字节从哪来"。判据：SHALL NOT 出现第二条解码实现（否则"每种路径各导入一张图，均能进入 OCR"这条验收就得逐条重测）。

WHEN 一张图 decode 失败
THEN SHALL 给 `VisionError::Decode` 对应的提示文案（"图片格式不支持或已损坏"，docs/04 §1 S0 的原文），并列出本版本实际支持的格式（PNG / JPEG / 剪贴板 TIFF）；SHALL NOT panic，SHALL NOT 把原始字节写进日志（NFR-2、NFR-3）。

WHEN 一张图的任一边超过 8000px
THEN 系统 SHALL 按比例缩到 8000px 上限**后再**进入识别，并在界面上明确"已缩放"（docs/04 §1 S0 的行为）；SHALL NOT 静默按原尺寸处理，也 SHALL NOT 因为缩放而丢弃识别结果的可回溯坐标（缩放后的尺寸就是 `OcrDocument.width/height`，REQ-003 的换算 SHALL 用它，不能用原图尺寸）。

边界：本包**不引入新的配置字段**——图片来源路径、上次打开的目录、缩放上限都不进 `config.toml`（8000 是 `ice-vision` 里的常量）。HEIC 等未开解码器的格式按 Q-005 的裁定处理，默认是"提示不支持"，SHALL NOT 用"猜测的降级图"糊过去。
并发：解码与缩放属 CPU 活，SHALL 与识别同段跑在 `spawn_blocking` 里（大图 `into_rgba8` + 缩放不该在 UI 线程做）；四条入口本身在 UI 线程只取路径/取剪贴板句柄。
验证层次：解码、超限缩放、四类错误各用无头单测（喂内存里的字节，不碰真实文件）；"拖拽真的能把文件送进窗口""原生面板真的弹出来""剪贴板里的截图真的能粘进来"三条**只有真机能定论**，走查 W-3.1~W-3.3；`Cmd+V` 与面板的键盘可达性同批走查。

### REQ-002: 识别全程在本机完成，产出带文本、置信度、归一化框的行级观察（FR-2.2、NFR-3）

WHEN 一张 `RgbaImage` 交给 `VisionOcr::recognize`
THEN 系统 SHALL 在同一条工作线程内完成"造 `CGImage` → 造 `VNImageRequestHandler` → 造 `VNRecognizeTextRequest` → `performRequests_error` → 把 observations 读成纯 Rust"，并 SHALL 只把纯 Rust 的结果（行观察集合）交回调用线程；SHALL NOT 让任何 `Retained<_>` 离开这段闭包（objc2 的 `!Send` 会在编译期挡住，本条是把这件事写成纪律而不是依赖巧合）。

WHEN 构造识别请求
THEN SHALL 使用 `Accurate` 档（`VNRequestTextRecognitionLevelAccurate`），SHALL 设语言候选为 `zh-Hans`、`en`、`ja`（ADR-002 的三条），SHALL 打开 `usesLanguageCorrection`；`automaticallyDetectsLanguage` 的取舍写进 design 并说明理由（crate 注释：仅 revision3 起有效）。取每条 observation 的 `topCandidates(1)` 作为读出文本，SHALL NOT 把 10 条候选全塞进 UI 数据。

WHEN 每行落进结果
THEN 该行 SHALL 带：文本（`VNRecognizedText::string`）、置信度（`VNRecognizedText::confidence`，`f32`，落在 `[0,1]`）、归一化框（`boundingBox()`，**原点左下**，见 REQ-003）。SHALL NOT 在这一层做坐标翻转或合并（S1 只识别、S2 只换算、S3 只合并——把三段混在一个函数里，REQ-004 的判据就没法单独测）。

WHEN Vision 返回空结果（图里确实没文字，或全是认不出的图形）
THEN SHALL 返回 `VisionError::NoTextFound`，界面提示"未发现文字"——这是**正常结果而不是错误**（docs/04 §1 S1 的失败列明写"不是错误"），SHALL NOT 记 WARN，SHALL NOT 在界面上弹红色失败态。

WHEN `performRequests_error` 返回 Err
THEN SHALL 转成 `VisionError::Recognition`，日志只记错误码与 localized description 的**截断长度**（`desc_len`），SHALL NOT 记任何识别出的文本内容；界面给一句"识别失败"加重试入口。

WHEN 识别在进行中
THEN 系统 SHALL NOT 发起任何网络请求（结构上：`ice-vision` 不依赖 reqwest，见事实表）；SHALL NOT 把图片字节写到本目录以外的任何地方；SHALL NOT 把识别出的文本写进日志（NFR-3 的"内容不进日志"这条纪律在图片侧同样成立，日志只允许 `rows`、`blocks`、`elapsed_ms`、尺寸）。

边界：`OcrEngine` trait 的形状不改（`recognize(&RgbaImage) -> VisionResult<OcrDocument>`）——若实现发现"行级观察必须先出 S1、S3 才能出 `OcrDocument`"，则**在 impl 内部**串完 S2/S3 再返回文档，而不是改 trait 签名去暴露中间态；拆分所需的行回溯信息按 REQ-005 放进块字段。
并发：同一时刻 SHALL 只允许一次识别在飞（与文本侧 `submit_while_in_flight_is_ignored` 同一条纪律）；识别期间界面照常可操作（页签切换、窗口缩放）。
验证层次：`Vision` 调用本身**无头测不到**（真实框架、真实线程），本包的无头断言落在"行观察 → 文档"的转换、坐标、合并、编辑四处纯函数上；识别真伪与耗时由真机走查 W-3.4 定论，网络零外发由 REQ-008 的两层取证定论。

### REQ-003: 归一化左下坐标翻成左上像素坐标，且单独可测（FR-2.2、docs/04 §1 S2、ADR-002）

WHEN 一个归一化框 `CGRect{x,y,w,h}`（原点左下、单位 1）配上文档尺寸 `W×H`
THEN 系统 SHALL 用**一个纯函数**产出 `PixelRect`（原点左上、单位像素）：`x_px = x × W`、`y_px = (1 − y − h) × H`、`w_px = w × W`、`h_px = h × H`；SHALL NOT 写成 `H − y×H`（漏掉高度那一步会把行整体上移一个行高，是这类换算最典型的错）。

WHEN 框贴边或略越界（Vision 会给到 0/1 之外的极小余量）
THEN 结果 SHALL 被夹在 `[0, W] × [0, H]` 内（宽/高同步夹），SHALL NOT 产出负坐标或负尺寸的块；夹取 SHALL 是**保守方向**（宁可少几像素也不让框跑到图外，因为 P7 要按这个框擦像素）。

WHEN 一张图被 REQ-001 缩放后再识别
THEN 换算 SHALL 用**缩放后**的尺寸（也就是 `OcrDocument.width/height`），SHALL NOT 用原图尺寸——否则所有框按比例错位。

WHEN 多边形（`polygon`）可用
THEN SHALL 与框走同一次翻转与同一套夹取，四个点的顺序 SHALL 保持不变（`TextBlock.polygon` 是 `Option`，本包允许只填 `bbox` 而把 `polygon` 留 `None`；倾斜块处理属 P7 的 L-5）。

边界：本 REQ 不产生任何 IO、不碰 Vision 对象，是纯粹的数值函数——正因如此它是本包最硬的一条无头判据。
验证层次：单测覆盖 `y=0` 与 `y=1` 两个极端分别对应**图像底行**与**图像顶行**（docs/06 TC-2.2-d 的原文判据）、非单位缩放因子、越界夹取、零高零宽框；再一条"翻转幂等方向"断言：顶边与底边换算后 SHALL 保持 `top < bottom` 的像素次序。

### REQ-004: 行按五条判据合并成语义块，表格不跨列，块不吞整张图（FR-2.2/2.3 的质量前提、docs/04 §2）

WHEN 一组已换算成像素坐标的行进入合并
THEN 系统 SHALL 用纯函数 `merge_rows(rows, MergeParams, 图像尺寸) -> Vec<BlockDraft>`，判据**逐条可测**，且全部满足才合并两行：① 左边界对齐 `|a.x − b.x| < align_tol_factor × 平均字高`（默认 0.6）；② 垂直间隙 `gap < row_gap_factor × 行高`（默认 1.3）；③ 水平投影重叠 > 较窄行的 `overlap_min`（默认 0.4）；④ 不是新段落起始（`b` 首行缩进 **且** 上一行未填满宽度 `fill_ratio_para_break`=0.85 时停止合并）；⑤ 表格防护：同一 y 带上存在 x 方向不连续的 ≥2 组行时判为表格行，**不跨列合并**。

WHEN 合并中
THEN 单块 SHALL NOT 超过 12 行且 SHALL NOT 超过 1200 字符（docs/04 §2 的"上限"），超出即断开成新块——这条硬上限的作用是"一次翻译不吞掉整张图"，属 P7 的输入安全，本包先把闸口放在合并侧。

WHEN 两个候选块的判据处在毫厘之间（阈值边界）
THEN 结果 SHALL 是**确定性的**：同一输入 SHALL 永远产出同一分组（不许依赖 `HashMap` 迭代序、不许依赖不稳定排序），行的处理顺序 SHALL 按"自上而下、自左而右"的确定次序（像素坐标 y 主、x 次）。理由：这条链路的产物要拿去比对样图打分，分组不可复现就等于指标不可复现。

WHEN 一块由多行合成
THEN 该块 SHALL 记录它来自哪几行（REQ-005 的行回溯），文本按行序以换行拼接，框取各行 `union`，置信度取**按字数加权的平均**（不是简单平均：一行 2 个字的低置信不该把一行 40 个字的正常识别拉低；具体取法写进 design 并给出理由）。

边界：合并参数**不进配置**，是 `MergeParams::default()`（值同 docs/04 §2）；调参在真样图上做，结论写进 P6 验收报告并同步 docs/04 §2（那是它的出处），不在本包里加一个"设置页调参"的入口。
验证层次：全部无头——五条判据各一组正反例、表格防护的三列样图、12 行/1200 字符两条上限、阈值边界两侧的确定性、`union` 后框真的包住所有行、加权置信度那一条**必须能区分"加权"与"简单平均"**（用一行 1 字低置信 + 一行 40 字高置信的例子锁死）。合并正确率的**人工判定**（docs/04 §5"段落合并正确率 ≥ 0.85"）属 REQ-008 第二层，本包不能自证。

### REQ-005: 识别结果以可编辑列表呈现，五种动作都改到将要交给 P7 的那份数据（FR-2.3）

WHEN 一次识别完成
THEN 图片页 H 区 SHALL 列出块，每块 SHALL 显示：勾选框、序号、置信度、原文、（翻译后）译文；置信度低于提示阈值的行 SHALL 带一个可见的 ⚠ 标记（阈值默认值与是否进设置页见 Q-006；docs/06 对 IMG-11 那条要求的就是"⚠ 提示"）。

WHEN 用户改某块的文本
THEN 系统 SHALL 就地更新该块的 `text`，SHALL NOT 因为它而重算分组或改动其它块；改完 SHALL 立刻反映在 `OcrDocument::merged_text()` 上——**这就是本包对 FR-2.3"修正后再翻译"的可验部分**（"生成图反映修改"落 P7，见本 REQ 的验证层次）。

WHEN 用户取消勾选某块
THEN 该块 SHALL 保持 `enabled = false`，`enabled_blocks()` 与 `merged_text()` SHALL 跳过它（现有语义已如此，本包只要求界面上那颗勾选框真的接到这个字段），并 SHALL NOT 删除数据（用户重新勾选就回来）。

WHEN 用户删除某块
THEN 系统 SHALL 从列表移除该块；删除 SHALL 是**明确动作**而不是"清空文本框的副作用"（清空文本 = 改成空串，与删除是两件事，空串块 P7 要跳过、删除块不存在）。

WHEN 用户选中相邻两块点"合并"
THEN 系统 SHALL 合成一块：文本按行序拼接、框取 `union`、置信度按 REQ-004 同一条规则合并、行回溯取并集；"合并"SHALL NOT 强依赖 REQ-004 的判据（用户手动点就是要越过判据）。

WHEN 用户选中一个块点"拆分"
THEN 系统 SHALL 按它的**行回溯**拆回原来的行，各行重新成块（有 ≥2 行时可用；单行块点拆分 SHALL 给提示而不是产出一个空列表）。SHALL NOT 在拆分时丢失用户已改过的文本——改过的行拆分后 SHALL 保持用户那段文字（按原行的字符比例切分是不诚实的做法，做不到就整体保留在第一行并给提示，具体取舍写进 design 与 Q-004）。

WHEN 列表改动后用户再次点识别或换一张图
THEN 系统 SHALL 明确丢弃旧列表（改一份就废），SHALL NOT 让用户在"旧列表配新图"的状态下继续操作。

边界：识别结果**不持久化**——`OcrDocument` 只活在进程里，不写配置目录、不写历史（历史页记的是文本翻译记录，图片链路的记录属 P7 之后再议，本包不新增字段）。列表的编辑深度按 FR-2.3 的四件（改字、删块、合并、拆分）+ 勾选，不额外做"编辑历史/撤销"。
验证层次：五种动作全部无头可测（数据层断言 + 一帧渲染量矩形）；`enabled=false` 那条 SHALL 用现成的 `merged_text()` 断言；"手改一处错字，生成图反映修改"里的**生成图**属 P7，本包把判据写成"改动进了 `OcrDocument` 且 `TranslationRequest` 的输入来自它"，这条切分 SHALL 在 `docs/traceability.md` 的 FR-2.3 行登记（谁验收哪半句，写清而不是一起打勾）。

### REQ-006: 识别期间界面不冻、状态与耗时可见、点取消后不产出任何结果（FR-2.9 的识别段）

WHEN 用户触发一次识别
THEN 系统 SHALL 立刻把图片页 I 区置为"识别中"状态，并 SHALL 继续正常渲染帧（页签可切、窗口可拖、其它按钮不响应重复识别）；识别 SHALL 跑在 `spawn_blocking`，UI 线程 SHALL NOT 出现任何 `performRequests` 的直接调用。

WHEN 结果（或失败）到达
THEN 帧循环 SHALL 在回收那一次把状态从"识别中"改成"完成 N 块 / 未发现文字 / 识别失败"，并 SHALL 显示**耗时**（`OcrProgress.elapsed_ms` 就是这个字段的存在理由）；结果到达 SHALL 触发一次重绘（沿用 P4/P5 的"结果到达才重绘"策略，不逐帧续期，NFR-2 的空闲 CPU 纪律不变）。

WHEN 用户在识别中点"取消"
THEN 系统 SHALL 做到三件：状态立刻回到空闲、界面**不因这次识别再产生任何数据或提示**、后续结果即便到达也被丢弃（按"任务序号"判过期，沿用文本侧 `stale_outcome_is_dropped_without_touching_the_ui` 那条已验证的机制）。SHALL NOT 声称中断了底层调用——按事实表那行，`VNRequest::cancel()` 因 `!Send` 拿不到，正在跑的那次会自己跑完（百毫秒级 CPU 照旧花掉）。界面文案 SHALL NOT 写"已取消识别"这种暗示底层被掐断的话，写"已忽略这次结果"或等价的诚实表达（Q-008）。

WHEN 识别期间用户切换图片来源或再次导入
THEN 系统 SHALL 按"新的那一次"为准，旧一次的结果 SHALL 作废（同一个过期机制，不新写一套）。

边界：本 REQ 只覆盖"识别"这一段；docs/01 FR-2.9 要的三阶段（OCR/翻译/合成）与每阶段各自取消属 P7，本包 SHALL 把状态机的形状做成**可扩展的三段枚举**而不是只塞一个 bool（否则 P7 要重写），但 SHALL NOT 在本包实现后两段。
验证层次：无头断言"派发立刻返回""结果晚到能落盘""过期结果被丢弃且不碰界面"（这三条文本侧已有同类，本包复用手法不复用代码路径）；"界面真的没冻""耗时数字看着合理"走查 W-3.5；真取消拿不到这件事的取证就是本 REQ 文案那两条断言。

### REQ-007: 非 macOS 或拿不到系统框架时明确降级，不假装可用（NFR-7、docs/04 §6 L-6）

WHEN 在非 macOS 平台构建并运行
THEN 图片页 SHALL 显示"当前平台不支持图片翻译"（docs/01 FR-2.2 与 ADR-002 都要求这句），`default_engine()` 返回 `None`，四个导入入口 SHALL 呈不可用状态（点了给同一句提示），文本翻译与其余三页 SHALL 完全不受影响（TC-2.2-e 的判据）。

WHEN 在 macOS 上但 Vision 调用整体失败（系统框架异常、CGImage 造不出来、内存压力）
THEN 系统 SHALL 记一条 `Recognition` 失败并按 REQ-002 的失败路径给出提示，SHALL NOT 因为"这是 macOS"就假定识别必然成功；SHALL NOT 在每帧重试（一次导入一次调用）。

WHEN 图片解码成功但 `image` 的 feature 集里没有该格式
THEN SHALL 走 REQ-001 的 `Decode` 提示（含"本版本支持 PNG/JPEG"这一句实际能力），SHALL NOT 报"未发现文字"（那是把能力缺口说成图片内容问题，属误导）。

边界：本 REQ 是 NFR-7 的一条（macOS 优先，其余平台"明确不可用而不是崩溃"）；非 macOS 侧 SHALL 至少保证编译通过（NFR-7 的既有口径），本包不为 Windows/Linux 引入 OCR 后端。
验证层次：`native_ocr_supported()` 与 `default_engine()` 的分支用无头断言（含"非 macOS 下四个入口不响应"这一条走 `Effects` 断言）；跨平台编译由 P8 的三目标核查（本包只在 CI 层面保持 `cargo check` 不回归，不承诺运行时）。

### REQ-008: 度量分两层——可自动算的先跑出来，只有用户能定论的不自证（验收方法）

WHEN 本包实现完成
THEN 系统 SHALL 有一条**可重复运行**的自动度量：用一个 `examples/` 里的小工具在**本机**把已知文本渲染成 PNG（字体与字号已知 → **标注由构造给出，不需要人工**），再跑识别，按"1 − 归一化编辑距离 / 标注长度"逐图算字符正确率并汇总。SHALL 覆盖：纯英文行、纯中文行、中英混排、多行同段落（考合并）、两列表格（考表格防护）。SHALL NOT 把这条自动度量的结果当作"FR-2.2 达标"的全部依据（下一行）。

WHEN 报告这条自动度量的意义
THEN 文档与验收 SHALL 明写它是**下限测试**：程序自绘的干净文字是 Vision 最容易的一类，它证明的是"链路通、坐标对、合并不串、零外发、不 panic"，**不证明**真实截图（艺术字、低对比度、照片背景、说明书小字）的质量。真实样图集（IMG-1~IMG-12）的 ≥0.92 判据 SHALL 由用户提供的截图 + 人工标注来算（`assets/samples/` 当前为空，见事实表），本包 SHALL 提供一个"把某目录里的图片逐张跑识别、结果落 CSV（只有文件名、行数、块数、耗时，不落文本内容）"的工具，让那一步只需用户放图 + 按脚本打分，不需要我再写代码。

WHEN 跑自动度量与真实样图
THEN 系统 SHALL 同时留下**零外发的运行时取证**：识别进行期间用 `nettop -p <pid>` 采样，输出里 SHALL NOT 出现本进程的出站长连接（TC-2.2-b 的原文判据）；静态一层用 `cargo tree -p ice-vision` 证明依赖图里没有 reqwest。两层都进 `evidence/`。

WHEN 任何一条度量达不到
THEN SHALL 按停止条件 1/2/3 回来找你，SHALL NOT 自动调低阈值、SHALL NOT 把"排除 IMG-9/12"扩大成"排除更多张"、SHALL NOT 修改已记录的证据件（这是 CHG-004 起就立着的门禁纪律）。

边界：本 REQ 不改任何业务行为，它管的是"这个包的 ✅ 怎么来的"；`docs/06-test-cases.md` §4 现有 16 行 P6/P7 用例里，属本包的那 6 行（TC-2.1-a..d、TC-2.2-a/b/c/d、TC-2.3-b 的数据侧）在实现阶段逐条补"结论"，其余留给 P7。
验证层次：`VERIFIED_CODE` = 自动度量脚本 + 依赖图断言 + 全部无头单测（一条命令能跑完）；`VERIFIED_RUNTIME` = `nettop` 采样与真实样图打分（需要真实运行，不能由 mock 代替）；`BUSINESS_CONFIRMED` = "识别看着可用"与"合并看着合理"这两条由你走查后定。

## 未决问题

| id | 问题 | 阻塞 | 结论 |
|---|---|---|---|
| Q-001 | 本包与 P7 的切分线画在哪？FR-2.1~2.3 是 Must，但 FR-2.3 的验收判据"生成图反映修改"必须靠 P7 的回填才能看 | 否 | **已决（沿革：CHG-004 非目标表第 29 行 + charter 的 P6/P7 分档）**：识别链（S0~S3 + 结果编辑 + 识别段的状态/取消）归本包 = CHG-005；回填链（S4~S8：翻译、背景估计、擦除/覆盖、版式、合成、导出，FR-2.4~2.9）归 P7 = CHG-006。FR-2.3 那一行的判据**拆两半登记**：本包验"改动进了交给 P7 的那份数据"，P7 验"生成图反映修改"（REQ-005 验证层次） |
| Q-002 | 文件选择器怎么来：自接 `NSOpenPanel`（objc2-app-kit 已在依赖里，只差 feature）还是引入 `rfd` 这类新 crate？ | 否 | **已决（实现选择，不改业务行为）**：自接 `NSOpenPanel` + `beginWithCompletionHandler:`。理由：① 绑定已在仓（`generated/NSOpenPanel.rs`、`NSSavePanel.rs:423/435` 已核对），只需补 feature；② 与 P5 已有的六处 objc2 直调同源，不引入第二套原生抽象；③ `rfd` 会为选一个文件拖进整套 panel/UTI 依赖。**代价写清**：要补 `objc2-foundation` 的 `NSURL` feature，且面板必须走异步回调（`runModal` 会冻帧，REQ-001 已禁） |
| Q-003 | **真实样图集谁来出？** `docs/06` 定义了 IMG-1~IMG-12 与 ≥0.92 的判据，但 `assets/samples/` 是空的，也没有标注文件。没有真实截图，FR-2.2 那条 Must 的达标数字就只能来自程序自绘的图（最容易的一类） | 否 | **待你决定（Agent 不替你定达标依据）**，三条可选：**(a)** 你截 10~12 张放进 `assets/samples/`，我给一个"逐张跑识别 + 落 CSV（只有文件名/行数/块数/耗时）"的工具，标注与打分由你或我按脚本做——最贴合需求；**(b)** 本包只用自绘合成图出自动指标，真实样图达标推到 P8 一起判——最快，但 FR-2.2 的 Must 数字本包就是没有；**(c)** 你先随手给 2~3 张最常用场景的截图（报错弹窗 / 深色截图 / 中英混排），我跑出观察值并如实登记"样本不足，不作达标判据"。**默认按 (c) 起步 + 邀请 (a)**；无论哪条，REQ-008 的两层写法不变，我不动 docs/06 里那条 0.92 |
| Q-004 | 拆分要能拆回原行，就得在块里留着"我来自哪几行"；可用户改过文本之后再拆，行与字的对应已经不可靠。怎么处理？ | 否 | **已决（Agent 定，属实现选择）**：块带运行态行回溯（`Vec<u32>` 行号，不进配置 schema）。拆分时：文本**未被用户改过** → 按行回溯如实还原各行；**已被改过** → 不猜字符比例，把用户那整段留在拆出的第一块、其余块给空串并给一句提示"这块改过文字，拆分后请核对"。理由：宁可不完美也不静默改坏用户手打的字。相关判据进 REQ-005 |
| Q-005 | HEIC 怎么办？macOS 照片默认存 HEIC，而本仓的 `image` 只开 png+jpeg；系统 ImageIO（`CGImageSourceCreateWithData`，`objc2-image-io` 是 objc2-vision 的可选依赖）本来就能解 | 否 | **待你决定（会改变 FR-2.1 的"每张图都能进 OCR"到底覆盖哪些格式）**：**(a)** 本包只承诺 PNG/JPEG + 剪贴板 TIFF，其余明确提示"格式不支持"，登记为已知限制 L-7（默认，改动最小）；**(b)** 图片解码统一走系统 ImageIO，一次拿到"macOS 支持的全部格式"（含 HEIC/WebP/TIFF），代价是 ice-vision 加一个绑定 crate、解码那半不再跨平台。你选 (b) 我就把 REQ-001 的解码那句改写，并在 `docs/04` 的已知限制里删掉 L-7 |
| Q-006 | 低置信 ⚠ 的阈值定多少、要不要进设置页？docs/06 对 IMG-11 只写了"置信度阈值与 ⚠ 提示"，没给数 | 否 | **已决（Agent 定默认，进代码常量不进配置）**：⚠ 提示阈值 **0.5**（Vision 自己给的候选置信度已经过排序，低于 0.5 的行在截图类图上基本就是认歪了）；块级加权置信度同样按这条提示。**不加设置项**：一个用户调不动也解释不清的阈值比一个写死的更糟；真样图跑完如果发现 0.5 太松/太紧，就改常量并在验收报告里写依据（同 REQ-004 那条"调参改数值不改结构"） |
| Q-007 | 8000px 上限那条"按比例缩到上限"由谁做、用什么插值？缩放后的图要不要留盘？ | 否 | **已决（按 docs/04 §1 S0 的原文）**：解码后在内存里缩，用 `image` 的 `thumbnail`（保比例、不裁切）；**不落盘**（识别结果只对应内存里这一份，缩放后尺寸就是 `OcrDocument` 的尺寸，REQ-003 已锁这条换算基准）；界面提示"已缩放"，导出（P7）时用户拿到的是**缩放后那张的合成结果**——这一点现在就要写明，否则 P7 会以为能画回原尺寸 |
| Q-008 | FR-2.9 那句"可取消"在本包能做到什么程度？`VNRequest::cancel()` 在绑定里存在，但那些类 `!Send`，而 `performRequests_error` 是同步阻塞 | 否 | **已决（Agent 依事实定，且不许在文案里含糊）**：本包的取消 = **结果过期**（立即回到空闲、界面上不产生任何数据或提示、晚到的结果按序号丢弃）。正在跑的底层调用跑完（百毫秒级 CPU 花掉，不可回收）。真正的中断要么跨线程传 `!Send` 对象（不安全也未证明），要么走 Vision 的异步处理入口（`VNVideoProcessor` 才有 `cancel` 那类，绑定里没有可取消的异步 perform）。P7 的翻译段是网络请求，那边**可以**真取消——两段语义不同，REQ-006 已要求把文案写成"已忽略这次结果"而不是"已取消识别" |

## 允许动作与停止条件

**批准即允许**：本地实现与验证；在 `ice-vision` 内接入 `objc2-vision` 0.3.2 及所需 feature（含 `ice-app/Cargo.toml` 里 objc2-app-kit/objc2-foundation 的 feature 补齐，属编译期开关不是新能力）；给 `image` 的解码 feature 做裁定范围内的增删（Q-005 选 (a) 则**不加**任何格式 feature；选 (b) 则引 `objc2-image-io` 并把解码统一到 ImageIO）；给 `ice-core::document` 加运行态行回溯字段（不改配置 schema、不加配置字段，Q-004）；把 `views/image.rs` 的占位版式换成真控件页并替换它那 2 条"没有可点控件"前提的测试；新建 `examples/` 下的度量小工具（自绘样图 + 逐张跑识别落 CSV）；跑 `nettop -p`、`cargo tree` 取证；读写 `assets/samples/`（新增文件，只读用户放进去的截图，**不改不删它们的原件**）；按 REQ-008 把度量结果与调参结论回写 `docs/04` §2/§5、`docs/06` §4（逐条补结论列 + 新增 W-3.x 走查行）、`docs/01` 不改（FR-2.1~2.3 的文字一个字不动）、`docs/02` 模块表（`ice-vision::{ocr,vision_sys,layout,bg}` 的本包部分）、`docs/03` §2 图片页控件清单（E/F/G/H/I 由占位转真控件）、`docs/05`（新增"objc2 类 `!Send` ⇒ 一次调用整段待在同一线程"这条硬规则 + 测试计数）、`docs/traceability.md` 的 FR-2.1/2.2/2.3 三行（含 FR-2.3 判据拆两半那句）、`docs/07-acceptance.md` 变更登记与 P6 段；阶段验收后 commit + push。

**不允许**：未经登记就改需求基线（含把 ≥0.92、8000px、12 行/1200 字符这些 docs/04 的数自己调小）；把图片、图片字节或识别出的文本上传到任何第三方，或写进日志/证据/提交（日志只允许 `rows`、`blocks`、`elapsed_ms`、尺寸、错误码与 `desc_len`；CSV 只允许文件名与计数）；未经确认执行 `rm`；改动 `assets/samples/` 里用户文件的原件；把自动度量的结果当"FR-2.2 达标"的唯一依据、或替用户提供真实样图之前宣布字符正确率通过；伪造 `./sdd record` 的运行时/人工证据；为了过门禁修改已记录的证据件；把剪贴板轮询或 `clipboard_watch` 默认值改掉（默认仍 `false`）；在界面或文档里把"结果过期"写成"已取消底层识别"。

**停止条件**（触发即记 `SPEC_DEVIATION` 退回评审，不自行改需求或降级 Must）：

1. **真实样图字符正确率明显低于 0.92**（无论样本来自 Q-003 的哪一支）：FR-2.2 是 Must，而这条数字只有真机能定论。触发时的选项是"换识别档位（Fast↔Accurate）"、"调整语言候选"、"接受并登记为已知限制并降 FR-2.2 的 Must"——第三条是需求降级，我不会自己选。
2. **Vision 调用在本机根本跑不起来**（CGImage 构造失败、`performRequests_error` 恒错、objc2-vision 与现有 objc2 版本有未预见的编译冲突、系统框架要求某项我没申请到的权限）：此时图片翻译在本机等于没有。选项是"换 `initWithURL_options` 走文件而不自造 CGImage"、"改走系统 ImageIO 解码 + CVPixelBuffer"、"引入外部 OCR（违背 ADR-002 的零分发与体积前提）"——第三条要重开 ADR，得你决定。
3. **合并判据按 docs/04 §2 的五条真跑不动真实截图**（例如表格防护那条把两列正常段落也判成表格、或段落断点判据在小字号密排图上失效）：说明 §2 的算法形状本身要改，而不只是调参。改算法形状属设计变更，要回来对齐。
4. **`spawn_blocking` 那条路出现跨线程/生命周期死结**（例如为读 observations 必须把 `Retained` 交回 UI 线程）：本包的前提是"整段在同一线程内读完"，若实现逼出例外，说明我对 objc2 `!Send` 的判定不足以覆盖实际用法。
5. **文件面板异步回调与帧循环的接缝拿不到**（`beginWithCompletionHandler:` 的回调线程不是主线程、或回调里发 egui 命令没有生效）：退回 `rfd` 或"只留路径输入 + 拖拽，去掉文件选择器"第二条改动的是 FR-2.1 的四入口 Must，需你批准。
6. **Q-003 长期拿不到真实截图**：本包可以带着"只有自动度量"的结论收尾，但 FR-2.2 的达标状态 SHALL 记为**未闭环**而不是 ✅；这条不算缺陷，算验收口径，写在 `docs/07` 的 P6 段。
7. **走查发现图片页在最小窗口（640×420）下版式塌了**（列表与预览互相压边、按钮溢出）：与 P5 的 REQ-012/REQ-013 同类缺陷。改版式属设计，回来对齐 `docs/03` §2 的图片页定义。

## 时间估算

- 开发 9~13 小时。分块：REQ-002 的 Vision 接线约 2.5~3.5 小时（不确定性最高：CGImage 构造、feature 开关、`!Send` 边界、空结果与错误映射，前四条停止条件都落在这块）；REQ-003 坐标纯函数约 0.5 小时；REQ-004 合并纯函数约 2 小时（五条判据 + 表格防护 + 两条上限 + 确定性排序，测试量大过实现）；REQ-001 四入口约 2 小时（拖拽与剪贴板现成，面板那条要试异步回调与帧循环的接缝 = 停止条件 5）；REQ-005 列表与五种动作约 2 小时；REQ-006 状态与过期约 0.8 小时（机制与文本侧同形，主要是新写一份序号过期）；REQ-008 度量工具约 1 小时（自绘样图那个小工具反而最费事：要把文字画进 PNG 且已知标注）。预计新增 90~130 项测试、约 1,600~2,200 行（含 `ice-app::system::imageio`/面板接线与 `views/image.rs` 的整页重写）。
- 文档回写约 1.5 小时（`docs/04` §2/§5 的调参结论与格式支持面、`docs/06` §4 的 6 行结论 + W-3.1~W-3.6 新增、`docs/02` 模块表、`docs/03` 图片页控件清单、`docs/05` 的 `!Send` 硬规则与计数、`traceability` 三行、`07-acceptance` P6 段与变更登记）。
- 验证 1.5~2.5 小时：`cargo test --workspace` + `clippy -D warnings` + `fmt --check` + `./sdd check` 一遍；自动度量脚本一轮；`nettop` + `cargo tree` 取证一次；真机走查一轮（需要你配合约 30~40 分钟：四个入口各进一张图、一张纯文字图看块与 ⚠、改一处字看 `merged_text`、识别中点取消、拖窗切页签不卡）。
- 外部等待：你的 spec 批准一轮（本包一条 `./sdd approve`）、真实样图提供（Q-003，可能多轮）、走查一到两轮、验收签名一轮——不计入开发工时。
- 依据：P4 的 9 个 REQ 用约两个工作日（含四轮走查、五个缺陷）；P5 的 12 个 REQ 因"真机才能定论"占比高，走了十轮改动。本包的无头可测比例明显更高（坐标、合并、列表、过期都是纯函数与数据层），风险集中在两处：Vision 接线那一段的本机未知数，以及"真实达标数字需要你的截图"这件事——后者不是工时，是外部依赖，所以本包估时不含等图的时间。
