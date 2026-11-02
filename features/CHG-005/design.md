Change: CHG-005
Version: v0.1
Spec-Hash: UNAPPROVED

# CHG-005 · Design

- Change: CHG-005 · v0.1 · 待用户批准（`./sdd approve CHG-005` 由用户本人跑；批准前本包只动规格与设计，不动 P6 业务代码）
- Repo: `ice-translator`（分支 `main`）
- 前置：CHG-004（P5）仍在 IMPLEMENTING，本包不带任何窗口/形态/吸附内容。代码态 `1f79f00`（2026-11-02）：**440 passed / 0 failed / 5 ignored**（ice-app 320 · ice-core 58 · ice-providers 62）
- 本包不改变任何已批准业务行为；要改变行为走 `./sdd revise` 并重新批准

## 实现方案

### 模块划分

| 文件 | 动作 | 内容 |
|---|---|---|
| `crates/ice-vision/Cargo.toml` | 改 | 打开那段注释掉的 `[target.'cfg(target_os = "macos")'.dependencies]`：`objc2-vision 0.3.2`（feature 逐个点名，见"依赖增量"）、`objc2-core-graphics`（default 已含 `CGColorSpace`/`CGDataProvider`，`CGImage::new` 那条 `cfg` 就是这两个）、`objc2-foundation`（`NSString`/`NSArray`）。**不新增任何带网络能力的 crate** |
| `crates/ice-vision/src/vision_sys.rs` | **新** | 唯一一处碰 Vision 对象的文件，整体 `#[cfg(target_os = "macos")]`。一个函数 `recognize_rows(&RgbaImage) -> VisionResult<Vec<RowObservation>>`：CGImage → `VNImageRequestHandler` → `VNRecognizeTextRequest` → `performRequests_error` → 逐条 observation 读成纯 Rust。**`Retained<_>` 全部在这一个函数体内生灭** |
| `crates/ice-vision/src/layout.rs` | **新** | S2 + S3 的全部纯函数：`flip_rect`（归一化左下 → 左上像素，含夹取）、`merge_rows`（五条判据 + 表格防护 + 两条上限 + 确定性排序 + 加权置信度）、`MergeParams`（docs/04 §2 的四个默认值）、`BlockDraft{text, bbox, confidence, source_rows}`。**零 IO、零 unsafe** |
| `crates/ice-vision/src/ocr.rs` | 改 | `VisionOcr::recognize` 由 `NotImplemented` 换成真实现：S1（`vision_sys`）→ S2/S3（`layout`）在 impl 内串完再返回 `OcrDocument`，**trait 签名一字不动**；新增 `RowObservation{text, normalized_box, confidence, row_index}` 与常量 `MAX_EDGE_PX=8000`、`LOW_CONFIDENCE_WARN=0.5` |
| `crates/ice-vision/src/import.rs` | **新** | S0：`admit(字节) / admit_rgba(像素缓冲) / admit_path(路径)` 三个入口汇到同一个内部函数，产出 `Admitted{image: RgbaImage, scaled: bool, width, height, label}`；解码只走 workspace 那份 `image`（PNG/JPEG，Q-005 选 (a)）；超 `MAX_EDGE_PX` 用 `image::thumbnail` 等比缩并置 `scaled=true`（Q-007：**不落盘**） |
| `crates/ice-core/src/document.rs` | 改 | `TextBlock` 加**运行态**字段 `source_rows: Vec<u32>`（`#[serde(default, skip_serializing_if = "Vec::is_empty")]`，不进配置 schema、不改既有字段顺序语义）+ 一个 `with_rows` 构造器。`enabled_blocks`/`merged_text` 的语义一个字不改（REQ-005 靠它们验"勾选与改动进了将要交给 P7 的那份数据"） |
| `crates/ice-app/src/system/panel.rs` | **新**（macOS） | `NSOpenPanel` 的**异步**打开：`beginWithCompletionHandler:`，回调里只把 `path` 交回帧循环（ channel），**禁 `runModal`**（会冻 eframe 的帧循环，REQ-001 明写）。沿用 P5 立下的两条纪律：每帧从控件现状读、`Retained` 自己存进线程局部（`unsafe_unretained` 不持有） |
| `crates/ice-app/src/system/clipboard.rs` | 改 | 加一条 `pub fn image() -> Option<(Vec<u8>, usize, usize)>`（`arboard::Clipboard::get_image()`，`image-data` 在 arboard 的 default 里，**Cargo.toml 不用改**）。文本读写那条路径不动 |
| `crates/ice-app/src/jobs.rs` | 改 | 加一支 `Job::ImageOcr{ id, image }` 与对应 `Outcome`；派发走 `runtime.spawn_blocking`，回收走现有 `drain`。**过期判定用图片侧自己的序号**（见 REQ-006 那节） |
| `crates/ice-app/src/state.rs` | 改 | 图片工作区：`Option<Admitted>`、`Option<OcrDocument>`、`RecognitionStage`（三段枚举，本包只实现第一段）、`image_request_id`、`elapsed_ms`、`scaled` 标记；五种编辑动作各一个方法（改字 / 勾选 / 删除 / 合并 / 拆分）+ 一次"换图即作废"清理 |
| `crates/ice-app/src/views/image.rs` | 重写 | 把 E/F/G/H/I 五个占位区换成真控件：E 四入口 + 路径输入、F 预览（含"已缩放"标记）、H 可编辑块列表（勾选框 / 序号 / 置信度 / ⚠ / 原文 / 译文列留空给 P7）、I 状态行（识别中 / 完成 N 块 / 未发现文字 / 识别失败 / 已忽略这次结果 + 耗时）。删掉那 2 条以"没有可点控件"为前提的测试，"图片翻译尚未启用"那块 `notice` 必须整体消失 |
| `crates/ice-vision/examples/` | **新** | 两个小工具：`self_metric.rs`（自绘已知文本 → 识别 → 逐图字符正确率，见 REQ-008）、`samples_run.rs`（逐张跑一个目录 → CSV，列只有文件名 / 行数 / 块数 / 耗时） |

### 依赖增量（批准前要先看清这一段）

| 增量 | 位置 | 理由 | 是否影响产物 |
|---|---|---|---|
| `objc2-vision 0.3.2` | ice-vision 的 macos 目标依赖 | ADR-002 的唯一识别后端；crate 已在 crates.io，feature 表 55 项无 default，按类开关 | 是（macOS 二进制） |
| `objc2-core-graphics` | 同上 | `RgbaImage` 直接变 `CGImage`，不落中间文件（spec 事实表已核对 `CGImage::new` 的参数表与 default feature） | 是 |
| `objc2-app-kit` 的 `NSOpenPanel`/`NSSavePanel`/`NSPanel` + `objc2-foundation` 的 `NSURL` | ice-app 的 macos 目标 feature | Q-002：自接面板，不引入 `rfd` | 否（crate 已在依赖图，只多开模块） |
| `ab_glyph_rasterizer` + `owned_ttf_parser` | **ice-vision 的 dev-dependencies only** | REQ-008 那条"把已知文本渲染成 PNG"需要一条能画字的路径；本仓现有依赖里没有任何字形光栅器（egui 的字形只在 GPU 管线里成形，拿不到位图）。这两个纯 Rust 小 crate 能读现成的 `.ttc`（含 P5 已经用过的"第 1 面才是黑体-简"那个面索引），且跨平台，将来 P7 的 S7 绘制可直接复用。**代价**：本包只让 `examples/` 用，产品依赖图不受影响；若不接受新增 crate，退路是停止条件 6 —— 自动度量缺位，FR-2.2 的达标状态记"未闭环"而不是 ✅ | 否（dev-only） |

`image` 的 feature **不动**（Q-005 默认 (a)：只承诺 PNG/JPEG，其余给明确"不支持"提示并登记 L-7）。`arboard` 的 `image-data` 本来就在 default 里，剪贴板图片不需要任何 feature 变更。

### 数据流（S0~S3 一条链，S4 之后属 P7）

```
四入口（拖拽 / 粘贴 / 面板 / 路径）
        │  字节 or RGBA 或 path
        ▼
ice-vision::import::admit*  ──►  Admitted{ RgbaImage, scaled, w, h, label }
        │  在 spawn_blocking 里
        ▼
VisionOcr::recognize(&RgbaImage)
   ├─ S1  vision_sys::recognize_rows      → Vec<RowObservation>（文本 / 归一化框 / 置信度 / 行号）
   ├─ S2  layout::flip_rect(×w, ×h)       → 左上像素框（夹进 [0,w]×[0,h]）
   └─ S3  layout::merge_rows(params)      → Vec<BlockDraft> → OcrDocument{ blocks(含 source_rows) }
        ▼
AppState.图片工作区（OcrDocument 只活在内存里，不持久化）
        ▼
views::image 的 H 区列表 ◄── 五种编辑动作（改字 / 勾选 / 删除 / 合并 / 拆分）
        │
        └─► merged_text() / enabled_blocks()  ← 本包对"修正后再翻译"可验的那一半（P7 接这一份去翻译与回填）
```

## REQ-001 实现与验证

**汇聚点是唯一一处解码**。四个入口各自只负责"把原料搞到手"，之后一律调 `ice_vision::import` 的三个 `admit*`，共用同一个 `VisionError` 类型、同一个 `MAX_EDGE_PX` 判据、同一条 `scaled` 分支：

| 入口 | 原料来源 | 落地细节 |
|---|---|---|
| 拖拽 | `ctx.input(|i| i.dropped_files)`（egui-winit 在桌面端把 `DroppedFile(path)` 写成 `NativeFile::from(path)`，**给的是路径不是字节**） | 图片页每帧读 `dropped_files`，取到路径后走 `admit_path`；读盘仍要 `std::fs::read` + 解码。图片页之外拖进图片：本包只保证"不 panic、不误当文本"，不做跨页签智能路由 |
| 粘贴 | `system::clipboard::image()` → `arboard::get_image()`（macOS 侧读 `NSPasteboardTypeTIFF` 并**已由 arboard 解成 RGBA**） | 拿到 RGBA 后走 `admit_rgba`，因此我们的解码器根本不需要 TIFF feature。剪贴板里没有图片（只有文本或空）→ 一条 `TOAST_LIFETIME` 提示，**不清空已有图片** |
| 面板 | `system::panel::open_async()`（`NSOpenPanel` + `beginWithCompletionHandler:`） | 回调线程不是主线程：回调里只把 `path` 塞进一个 channel，帧循环下一帧取走再走 `admit_path`。`canChooseDirectories=false`、`allowedContentTypes` 只放 PNG/JPEG（Q-005 (a)） |
| 路径 | H 区上方一个 `TextEdit` + 「载入」按钮 | 与拖拽共用 `admit_path`；错误文案分两档：**"找不到文件"**（`NotFound`/是目录/无权限）与**"这张图解不开"**（`Decode`），不把 OS 错误原文整段甩到界面 |

`admit*` 内部唯一的一条缩放分支：任一边 > `MAX_EDGE_PX` → `image::thumbnail(MAX_EDGE_PX, MAX_EDGE_PX)`（等比、不裁切）→ `Admitted.scaled = true`，**缩放后的尺寸就是 `OcrDocument.width/height`**（REQ-003 的换算基准由这条锁住，不用原图尺寸）。

错误映射：`Decode → "图片格式不支持或已损坏（本版本支持 PNG / JPEG）"`；`TooLarge` 在本包**不作为用户可见错误**（超上限是缩放而不是拒绝，`VisionError::TooLarge` 留给"缩放后仍无法处理"的极端情况，若真出现属停止条件 2）。解码与缩放属 CPU 活，SHALL 与识别同段在 `spawn_blocking` 里做；UI 线程只取路径/取剪贴板句柄。

**验证**：无头——四条入口各喂一份内存里的原料（PNG 字节 / RGBA / 不存在的路径 / 坏字节）断言落到同一个 `Admitted`、同一个错误类型；"缩放后 `w/h` 就是文档尺寸"一条断言；"超过上限必须缩、且不静默"一条断言。**真机**：W-3.1 拖一张、W-3.2 粘一张截图、W-3.3 面板选一张并确认**面板弹得出来且窗口没冻**（停止条件 5 就在这条上）。

## REQ-002 实现与验证

`vision_sys::recognize_rows` 的函数体就是整条 S1，形状被 `!Send` 逼定（spec 事实表：objc2-vision 全文没有一处 `unsafe impl Send/Sync`）：

```rust
// 伪码，实现在 vision_sys.rs，整体 cfg(target_os = "macos")
let cg = CGImage::new(w, h, 8, 32, w*4, &srgb_colorspace, alpha_premultiplied_none, &data_provider, &[0.0], false, .default())?;
let handler = VNImageRequestHandler::initWithCGImage_options(&cg, &options)?;
let request = VNRecognizeTextRequest::new();
request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
request.setRecognitionLanguages(&["zh-Hans", "en", "ja"]);   // ADR-002 三条候选
request.setUsesLanguageCorrection(true);
request.setAutomaticallyDetectsLanguage(false);             // 取舍见下
handler.performRequests_error(&[&request])?;                // 同步阻塞
let rows: Vec<RowObservation> = request.results()
    .map(|obs| obs.iter().enumerate().filter_map(|(i, o)| {
        let t = o.topCandidates(1).first()?;                // 只取第一名候选
        Some(RowObservation { text: t.string().to_string(),
                              normalized_box: t.boundingBox(),
                              confidence: t.confidence(), row_index: i })
    }));
Ok(rows)                                                     // 纯 Rust 出栈，Retained 全部在此生灭
```

三个口径：

1. **`automaticallyDetectsLanguage` 关**。绑定注释明写它只在 `VNRecognizeTextRequestRevision3` 起有效、之前是 no-op；开着它等于把"语言候选"这条可控变量变成框架自由发挥，度量结果不可复现。真实截图若出现候选表覆盖不到的语言（纯日文竖排之类），属停止条件 1 的样本证据，回来再议。
2. **`topCandidates(1)`，不是 10**。候选全塞进 UI 数据等于把"哪条是识别结果"这件事推给用户；FR-2.2 要的是块级 `文本 + 置信度 + bbox`，一名候选就够。
3. **`NoTextFound` 不是错误**。observations 为空 → `VisionError::NoTextFound`，界面走"未发现文字"这一档中性提示，**不记 WARN、不弹红色失败态**（docs/04 §1 S1 失败列原文就是这个语义）。`performRequests_error` 返回 Err → `Recognition`，日志只记错误码与 `desc_len`（描述长度），**识别出的文本一个字都不进日志**（NFR-3 在图片侧同样成立；日志允许的字段只有 `rows`/`blocks`/`elapsed_ms`/尺寸/错误码/`desc_len`）。

零外发在结构上是现成的（`ice-vision/Cargo.toml` 没有 reqwest），本包要把它写成可断言的形式：见 REQ-008 的两层取证 + `Cargo.toml` 那条"不许新增带网络能力的 crate"的设计约束。

`S1→S2→S3` 的串联发生在 `VisionOcr::recognize` 内部，**trait 签名 `recognize(&RgbaImage) -> VisionResult<OcrDocument>` 不改**：中间态（行观察）不进 trait，只在 impl 内出现；行回溯信息通过 `TextBlock.source_rows` 带出去（REQ-005 的拆分靠它）。

**验证**：无头测不到真实框架调用（这是本包的诚实边界），落在"行观察 → 文档"的转换（喂造好的 `RowObservation`）与错误映射；识别真伪、耗时、`Accurate` 档是否够快由 W-3.4 真机定论。

## REQ-003 实现与验证

`layout::flip_rect(NormalizedRect{ x, y, w, h }, img_w, img_h) -> PixelRect`，四行算术，一条都不能写错的那种：

```
x_px = x × W
y_px = (1 − y − h) × H      // ← 不是 H − y×H：漏掉 − h 会把整行往上挪一个行高
w_px = w × W
h_px = h × H
```

夹取是**保守方向**：`x_px/w_px` 夹进 `[0, W]`、`y_px/h_px` 夹进 `[0, H]`，越界时同步收窄宽高而不是挪 origin（P7 要按这个框擦像素，宁可少擦几像素也不擦到框外）。Vision 偶尔给出 0/1 之外的极小余量，夹取因此不是防御性代码而是必需步骤。产出 SHALL NOT 出现负坐标或负尺寸。

多边形：`TextBlock.polygon` 本包**允许为 `None`**（只填 `bbox`），因为 `VNRecognizedTextObservation` 的 polygon 在绑定的哪个版本、倾斜块的逆仿射处理属 P7 的 L-5。若实现时 polygon 拿得到，翻转与夹取 SHALL 复用同一次换算、四点顺序保持不变。

缩放基准：换算一律用 `Admitted` 之后的尺寸，即 `OcrDocument.width/height`（REQ-001 那条）。

**验证**（本包最硬的一条无头判据，`crates/ice-vision/src/layout.rs` 的 `mod tests`）：`y=0` 落**图像底行**（`y_px` 接近 H）、`y=1` 落**图像顶行**（`y_px` 接近 0，docs/06 TC-2.2-d 的原话）、非单位缩放（W≠H）、贴边与越界夹取、零宽零高框、"翻转保序"（顶边 SHALL 永远小于底边）、以及一条反向断言——`H − y×H` 那个典型错写法必须能被这组用例区分出来（构造一个 `h` 非零的框，让两种写法给出不同结果）。

## REQ-004 实现与验证

`layout::merge_rows(&[RowObservation], &MergeParams, W, H) -> Vec<BlockDraft>`，五条判据逐条独立可测，全部满足才合并（docs/04 §2 的参数原值，**本包不预先调参**）：

| # | 判据 | 具体取法 |
|---|---|---|
| ① | 左边界对齐 | `abs(a.x − b.x) < align_tol_factor(0.6) × 平均字高`，平均字高取两行 `h` 的算术平均 |
| ② | 垂直间隙 | `a.vertical_gap(b) < row_gap_factor(1.3) × 行高`；`vertical_gap` 返回 −1.0（上下重叠）时判"间隙不合格"——重叠行属表格那一档或乱序，不靠它合并 |
| ③ | 水平重叠 | `overlap_len(a,b) > overlap_min(0.4) × min(a.w, b.w)` |
| ④ | 不是新段落 | `b` 相对块左边界有缩进（超出 ① 的容差）**且** `a.w < fill_ratio_para_break(0.85) × 参考宽度` 时停止合并。**参考宽度取"同列内已合并行的最大 w"**，没有列信息时退化为全图最大行宽——用整张图的宽度当分母会把右对齐的短行误判成"未填满" |
| ⑤ | 表格防护 | 先按 y 重叠把行切成"行带"，同带内 x 方向不连续的组数 ≥2 即判为表格：合并只允许发生在**同一 x 组内**，绝不跨列 |

两条硬上限：单块 ≤ **12 行**、≤ **1200 字符**（docs/04 §2 的"上限"），任一超出即断开成新块。这条闸门的意义是"一次翻译不吞掉整张图"，本包把它放在合并侧（P7 因此不必再防）。

**确定性**（这条是度量可复现的前提，不是洁癖）：处理顺序按"自上而下、自左而右"，用 `f32::total_cmp` 先比 `y_px` 再比 `x_px`，完全相等时按 `row_index` 破平——不用 `HashMap` 迭代序、不依赖不稳定排序。同一输入 SHALL 永远产出同一分组。

**块级置信度取按字符数加权**：`Σ(confᵢ × charsᵢ) / Σcharsᵢ`（`chars` 是 `text.chars().count()`，不是字节数——中文一行 20 字节不是 20 字）。取这条而不是简单平均的理由：一行 2 个字的低置信不该拉低一行 40 个字的正常识别；测试里 SHALL 用"1 字 conf 0.2 + 40 字 conf 0.9"这种例子，使加权（≈0.883）与简单平均（0.55）**分得开**。`BlockDraft.source_rows` 记录该块由哪几行号组成（REQ-005 拆分用）。

**验证**：全部无头。五条判据各一组正反例；表格防护用一张三列样图（造出来的行矩形）；两条上限各一条"第 13 行断开""第 1201 字符断开"；阈值边界两侧各一条并断言两次运行结果相同（确定性）；`union` 后的框真的包住所有成员行；加权与简单平均分得开。**段落合并正确率 ≥ 0.85 的人工判定属 REQ-008 第二层，本包不自证**。

## REQ-005 实现与验证

数据结构用现成的 `OcrDocument`/`TextBlock`（`enabled`、`merged_text`、`enabled_blocks` 语义一字不改），唯一新增是 `source_rows`。五个动作全部落在 `AppState`，`views::image` 只调这些方法、不自己改字段：

| 动作 | 方法 | 关键约束 |
|---|---|---|
| 改字 | `edit_block_text(id, new)` | 只改该块 `text`；**不重算分组、不动别的块**；改完 `merged_text()` SHALL 立刻反映（这就是"修正后再翻译"在本包可验的那一半） |
| 勾选 | `set_block_enabled(id, bool)` | 接 `enabled` 字段；关掉的块**不删数据**，重新勾选就回来 |
| 删除 | `remove_block(id)` | 从 `blocks` 里移除。**与"把文本清空"是两件事**：空串块仍在列表里（P7 要跳过），删除块不存在——这条区分要有断言 |
| 合并 | `merge_blocks([id_a, id_b])` | 文本按行序换行拼接、框取 `union`、置信度按 REQ-004 同一条加权、`source_rows` 取并集。**用户手动点合并就是要越过 REQ-004 的判据**，所以这里不跑判据（否则那颗按钮永远点不动） |
| 拆分 | `split_block(id)` | 用 `source_rows` 拆回原行成多个块；`source_rows.len() < 2` 时给提示不产出空列表。**改过的文本不猜字符比例**（Q-004）：未改 → 按行回溯如实还原；已改 → 用户那整段留在第一块、其余块给空串 + 一句"这块改过文字，拆分后请核对" |

列表呈现：勾选框 / 序号 / 置信度 / 原文 / 译文（P7 才填）；置信度 < `LOW_CONFIDENCE_WARN`(0.5) 的行带一个可见 ⚠（docs/06 对 IMG-11 的要求）。⚠ 的判据是块级加权置信度，阈值是常量（Q-006：不进设置页——一个用户调不动也解释不清的阈值比写死的更糟）。

**换图/重识别即作废**：`clear_document()` 在下一次 `Admitted` 落地时调用，SHALL NOT 出现"旧列表配新图"的状态。

**边界**：识别结果不持久化（不写 `config.toml`、不写历史）。不做撤销/编辑历史。

**验证**：五种动作全部无头（数据层断言 + 一帧渲染量矩形）；`enabled=false` 那条直接用现成 `merged_text()` 断言；"手改一处错字，生成图反映修改"里的**生成图**属 P7——这条切分 SHALL 写进 `docs/traceability.md` 的 FR-2.3 行（谁验收哪半句写清，不是一起打勾）。W-3.6 判"列表看着能用"。

## REQ-006 实现与验证

状态机三段可扩展、本包只实现第一段（FR-2.9 的形状要求）：

```
enum RecognitionStage { Idle, Running{id}, Done{blocks, elapsed_ms}, Empty, Failed{reason} }
```

（`Empty` 单列一档，因为"未发现文字"不是失败；P7 会加 `Translating`/`Composing` 两支，本包留枚举位但不实现。）

派发：`Job::ImageOcr` → `runtime.spawn_blocking` → `Outcome` 进 channel → 帧循环 `drain` 回收。**UI 线程 SHALL NOT 出现任何 `performRequests` 直接调用**；`Running` 期间界面照常可操作（页签可切、窗口可拖），重复识别请求被忽略（与文本侧 `submit_while_in_flight_is_ignored` 同一条纪律，同一时刻只允许一次在飞）。

**过期判定用图片侧自己的序号 `image_request_id`，不复用文本侧的 `request_id`**。理由：两者寿命不同——用户识别中途去做一次文本翻译是正常的，若共用一个计数器，那次翻译会**误废掉**图片结果。同一个"序号判过期"的手法（机制复用、计数器分开），与文本侧 `stale_outcome_is_dropped_without_touching_the_ui` 同形。

取消的诚实形式（Q-008）：点「取消」→ 置 `Idle` + `image_request_id += 1` → 晚到的结果在 `apply` 处被丢弃，**不产出任何数据或提示**；底层那次调用继续跑完（`VNRequest::cancel()` 拿不到，`!Send`）。因此界面文案 SHALL 写「已忽略这次结果」，SHALL NOT 写「已取消识别」——这条写成断言（枚举里那个 `Ignored` 档的显示串必须含"忽略"、必须不含"已取消"），因为它是**文案级的诚实约束**，只能靠测试钉住。

结果到达 SHALL 触发一次重绘（沿用"结果到达才重绘"，不逐帧续期，NFR-2 空闲纪律不变）；`OcrProgress.elapsed_ms` 就是状态行那个耗时数字的来源。

**验证**：无头三条——派发立刻返回（不阻塞）、结果晚到能落地、过期结果被丢弃且不碰界面（含"文本侧 request_id 变了不影响图片侧"那条反向断言）；文案两条断言。W-3.5 判"界面真的没冻""耗时数字看着合理"。

## REQ-007 实现与验证

非 macOS：`native_ocr_supported()` 已是 `cfg!(target_os = "macos")`，`default_engine()` 已返回 `None`——本包把这两处接进界面而不是重写它们。图片页顶部显示"当前平台不支持图片翻译"，四入口呈不可用态（点了给同一句提示），`RecognitionStage` 恒 `Idle`；**文本翻译与其余三页完全不受影响**（TC-2.2-e 的判据）。

macOS 但框架整体失败（CGImage 造不出来 / `performRequests_error` 恒错）：走 `Recognition` 失败路径，一次导入一次调用，**SHALL NOT 每帧重试**（无自动重试这条要写成断言，否则一帧一个失败请求等于自己造 DOS）。

能解码但格式不在 feature 集里（HEIC/GIF/WebP，Q-005 (a)）：走 `Decode` 那档文案并**列出实际支持的格式**（PNG/JPEG），SHALL NOT 报"未发现文字"——把能力缺口说成图片内容问题属误导，这条也写成断言。

跨平台编译：本包只保证 `cargo check` 不回归（NFR-7 的既有口径，S 档），不为 Windows/Linux 引入 OCR 后端。新增的 `objc2-*` 全部挂在 `[target.'cfg(target_os = "macos")'.dependencies]` 下，非 macOS 目标不编译它们——**这是 REQ-007 能被 `cargo check --target` 验的结构前提**（2026-11-02 的 NFR-7 核查已把"本机只有一个 target、编译级记未验证"这件事记进 `docs/06` TC-P-7，本包沿用同一口径，不提前宣布通过）。

**验证**：无头——`native_ocr_supported()`/`default_engine()` 分支 + "非 macOS 下四入口不响应"（走 `Effects` 断言）+ "解码器没有该格式时报 Decode 而不是 NoTextFound"。三目标 `cargo check` 属 P8（TC-P-7），不在本包承诺范围。

## REQ-008 实现与验证

度量分两层，**混在一起就是自欺**：

**第一层（可自动算，一条命令跑完）**：`crates/ice-vision/examples/self_metric.rs` 在本机把已知字符串画成 PNG（字体与字号已知 → **标注由构造给出，不需要人工**），跑 `VisionOcr::recognize`，逐图算 `1 − 归一化编辑距离 / 标注长度` 并汇总。覆盖五类：纯英文行、纯中文行、中英混排、多行同段落（考 REQ-004 的合并）、两列表格（考表格防护）。绘制路径见"依赖增量"那行 dev-only 的 `ab_glyph_rasterizer` + `owned_ttf_parser`（读 `.ttc` 时 SHALL 显式给面索引——P5 的教训：`.ttc` 第 1 面才是黑体-简，选错面不崩、不缺字，只有肉眼看得出）。

这层 SHALL 在文档里明写它是**下限测试**：程序自绘的干净文字是 Vision 最容易的一类，它证明"链路通、坐标对、合并不串、零外发、不 panic"，**不证明**真实截图（艺术字、低对比度、照片背景、说明书小字）的质量。

**第二层（只有用户能定论）**：真实样图集 IMG-1~IMG-12 与 ≥0.92 的判据在 `docs/06` 里，但 `assets/samples/` 是空的、没有标注文件（spec 事实表）。本包交付一个 `examples/samples_run.rs`：逐张跑识别 → 落 CSV，**列只有文件名 / 行数 / 块数 / 耗时**（不落任何识别文本，隐私边界与日志一致），让"放图 + 打分"这一步不需要我再写代码。Q-003 的 (a)/(c) 两支由他选；拿不到真实截图时按停止条件 6 收尾——**FR-2.2 记"未闭环"而不是 ✅**。

**零外发两层取证**：静态 `cargo tree -p ice-vision -e normal`（**明确排除 dev 依赖**，否则上面那两个度量 crate 会混进图里造成误读）断言无 `reqwest`/`hyper`；运行时识别期间 `nettop -p <pid>` 采样，输出里 SHALL NOT 出现本进程出站长连接（TC-2.2-b 原话）。两层都进 `features/CHG-005/evidence/`。

**任何一条达不到**：按停止条件 1/2/3 回来找他，SHALL NOT 自动调低阈值、SHALL NOT 把"排除 IMG-9/12"扩大成"排除更多张"、SHALL NOT 改已记录的证据件。

**验证**：`VERIFIED_CODE` = 自动度量脚本 + 依赖图断言 + 全部无头单测；`VERIFIED_RUNTIME` = `nettop` 采样 + 真实样图打分；`BUSINESS_CONFIRMED` = "识别看着可用"与"合并看着合理"由他走查后定。`docs/06` §4 属本包的那 6 行（TC-2.1-a..d、TC-2.2-a/b/c/d、TC-2.3-b 数据侧）在实现阶段逐条补结论，其余留 P7。

## 影响与回滚

- **涉及仓库/文件**：只动 `ice-translator`（`main`）。`ice-vision`（4 个既有文件 + `vision_sys`/`layout`/`import` 三个新文件 + 2 个 example）、`ice-core::document`（加一个运行态字段）、`ice-app`（`system/panel.rs` 新增、`system/clipboard.rs` 加一个函数、`jobs.rs` 加一支、`state.rs` 加图片工作区、`views/image.rs` 整页重写）、`Cargo.toml`（macos 目标 feature/依赖 + ice-vision 的 dev-dependencies）。
- **配置与数据**：**不新增任何配置字段**、不改 `config.toml` 格式、不写历史。识别结果是运行态，重开即无。`assets/samples/` 只新增文件、**不改不删用户放进去的原件**。
- **兼容性**：`TextBlock` 加字段带 `#[serde(default)]`，任何既有（若有）序列化数据仍能读出；`OcrEngine` trait 签名不变，P7 无需改接线；`enabled_blocks`/`merged_text` 语义不变。
- **测试**：P5 的 440 项**不许有回退**——尤其 `views/image.rs` 那 2 条"没有可点控件"前提的测试是**被替换**而不是被扩充（当初不放控件的理由在模块注释第 4-5 行，本包正是那条注释说的"P6 接入 `ice-vision`"的时刻），替换时 SHALL 同时删掉那句注释。预计新增 90~130 项。
- **回滚**：本包全部改动集中在图片页与 `ice-vision`，回滚 = `git revert` 那批提交；`ice-core::document` 那个字段是纯增，revert 后无残留。文本链路不受影响是**设计约束而不是运气**（REQ-007 明写"其余三页完全不受影响"，并有断言）。
- **SDD 操作注意**：他 `./sdd approve CHG-005` 之后先跑 `./sdd plan-stamp`（写 design 的 Spec-Hash 与 tasks/traceability 的 baseline）、再 `./sdd refresh`（tasks.md 的生成头里也带 Spec-Hash，不刷新会让 `validate` 判视图不一致），最后 `./sdd seal-plan` 锁 `plan_hash`。顺序错了要动计划就 `./sdd replan`（见 [[sdd-plan-hash-drift]] 那条教训）。

## 验证层次

| 层 | 手段 | 覆盖 | 谁判 |
|---|---|---|---|
| `VERIFIED_CODE`（local/mock） | `cargo test --workspace` / `-p ice-vision` / `-p ice-core` / `-p ice-app`、`clippy -D warnings`、`fmt --check`、`check_traceability.py`、`./sdd scan`、密钥形态 grep | REQ-001~008 的纯函数、数据层、状态机、版式矩形、错误映射、文案断言 | Agent（无头） |
| `VERIFIED_CODE`（static） | `cargo tree -p ice-vision -e normal`（无 reqwest）、`Cargo.toml` 审查 | REQ-002/REQ-008 的"结构上不外发" | Agent，结论写进证据件 |
| `VERIFIED_RUNTIME`（sandbox） | 本机 `examples/self_metric.rs` 一轮、识别期间 `nettop -p <pid>` 采样一轮；真实样图打分（待 Q-003） | REQ-002 的识别与耗时、REQ-008 的两层取证 | Agent 跑，数字如实记 |
| `BUSINESS_CONFIRMED`（business） | W-3.1~W-3.6：四入口各进一张图、纯文字图看块与 ⚠、改一处字、识别中点取消、拖窗切页签不卡、最小窗口（640×420）下版式不塌 | 只有真机能定的那部分 | **只有用户**（停止条件 1/3/6/7 都落在这里） |
