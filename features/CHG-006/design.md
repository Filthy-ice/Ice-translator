Change: CHG-006
Version: v0.1
Spec-Hash: UNAPPROVED

# CHG-006 · Design

- Change: CHG-006 · v0.1 · 待用户批准（`./sdd approve CHG-006` 由用户本人跑；批准前本包只动规格、设计与计划，不动 P7 业务代码）
- Repo: `ice-translator`（分支 `main`）
- 前置：CHG-005（P6 识别链）在 IMPLEMENTING、同一批真机走查未逐项定论；代码态 `9a933b5`（2026-11-04 夜间）：**525 passed / 0 failed / 5 ignored**（ice-app 369 · ice-core 58 · ice-providers 65 · ice-vision 33）
- 本包不改变任何已批准业务行为；要改变行为走 `./sdd revise` 并重新批准

## 实现方案

### 模块划分

| 文件 | 动作 | 内容 |
|---|---|---|
| `crates/ice-vision/Cargo.toml` | 改 | 把 `ab_glyph_rasterizer 0.1.10` + `owned_ttf_parser 0.25.1` 从 `[dev-dependencies]` 移到 `[dependencies]`（纯 Rust、无 feature 门、不在任何网络路径上；两 crate 均已在 `Cargo.lock` ⇒ **lockfile 不变、不需要联网**，但 `-e normal` 那张正常依赖树会**多出三个名字**——`ab_glyph_rasterizer`（依赖表为空）+ `owned_ttf_parser` + 它的传递依赖 `ttf-parser 0.25.1`（依赖表也为空），实测今天的正常树里这两个关键字零命中）。`image` 的 feature 一个都不加（PNG/JPEG 编码本来就在那个收窄集合里）。**移动后必须重跑一次 `cargo tree -p ice-vision -e normal` 的零网络取证**——`-e normal` 排除 dev 依赖，提升前那张输出不能再交差 |
| `crates/ice-vision/src/bg.rs` | **新** | S5 全部纯函数：`estimate(&RgbaImage, &PixelRect, &CompositeOptions) -> BackgroundEstimate{ bg, fg, variance, kind, corners }`、`BackgroundKind{Uniform,Gradient,Complex}`、采样带构造（贴边收缩）、4bit/通道量化直方图、alpha 加权、WCAG 相对亮度与对比度。零 IO、零 `unsafe`、平台无关（NFR-6：`ice-vision` 不依赖任何 UI crate） |
| `crates/ice-vision/src/typeset.rs` | **新** | S6 全部纯函数：`plan(&TextBlock, &BackgroundEstimate, &Metrics, &CompositeOptions, mode) -> BlockPlan{ kind: InPlace\|Overlay\|Skip, font_size, lines, align, degraded }`、断行（CJK 按字符 / 拉丁按词 / 标点禁则）、0.95 步进缩字号、对齐推断、覆盖层底色（对比度 ≥ 4.5:1 的那个循环）。**测量与栅格化交给注入进来的 `Metrics` trait**，于是本文件的每条判据都能在无头里测（用假度量），不必等字体加载成功 |
| `crates/ice-vision/src/paint.rs` | **新** | 唯一碰系统字体文件与逐像素写的地方：`FileMetrics`（实现 `typeset::Metrics`，走 `owned_ttf_parser` 的 `glyph_metrics`/`outline_glyph`）、`draw_text(...)`（`ab_glyph_rasterizer` 的 coverage 做 src-over 混合）、`erase(...)`（三档）、`overlay_plate(...)`、`count_missing(...)`。字体字节进 `OnceLock<Vec<u8>>` 一次读盘；**缓存 `Vec<u8>` 而不是缓存 `Face`**（`Face` 的 `Sync` 是上游实现细节而非承诺） |
| `crates/ice-vision/src/composite.rs` | 重写 | `Renderer::compose(&RgbaImage, &OcrDocument, CompositeMode) -> VisionResult<Composed>`：S5（逐块 `bg::estimate`）→ S6（逐块 `typeset::plan`）→ **先全部擦除再全部绘制**两个独立循环 → `Composed{ image, per_block: Vec<(u32, PlanKind)>, replaced, overlaid, skipped, degraded }`。`CompositeOutput` 那个二选一 enum 按 Q-001 退场（全仓零引用已证）；`CompositeOptions` 的四个数原样留下并**补两个新常量**（见"docs/04 要补的四个数"） |
| `crates/ice-vision/src/export.rs` | **新** | 像素 → 字节的唯一一条路：`Format{Png,Jpeg}`、`encode(&RgbaImage, Format) -> VisionResult<Vec<u8>>`、`save(&RgbaImage, &Path, Format) -> VisionResult<()>`（JPEG 质量常量 `JPEG_QUALITY = 85`）。放在视觉层而不是界面层的理由：编码是像素的事，"默认不落盘、只写用户选中的路径"这条隐私纪律要跟它住同一层 |
| `crates/ice-core/src/font.rs` | **新**（Q-003） | 字体路径与 face 号的**唯一出处**：`HEITI_PATH` + `HEITI_SC_FACE_INDEX = 1` + `GOTHIC_PATH`（后备，谚文/西里尔/假名/拉丁那一片，`examples/self_metric.rs` 已证它缺字形会先报出来）。`ice-app::fonts`、`ice-vision::paint`、`examples/self_metric.rs` 三处改引用它 |
| `crates/ice-app/src/jobs.rs` | 改 | 加两支：`Job::ImageTranslate{ id, engine, pair, blocks: Vec<(u32,String)>, cancel: Arc<AtomicBool> }`（tokio 里**串行**逐块调 `Translator::translate`，每轮检查 `cancel`，单块 Err 不中断循环）与 `Job::ImageCompose{ id, source, document, mode }`（`spawn_blocking`）。两条各一个 `Outcome` 变体，`ImageTranslated` 带 `Vec<(u32, Result<String, ProviderError>)>`——**落日志时只记变体名**（`jobs.rs` 已有 `vision_variant_name` 的同族纪律） |
| `crates/ice-app/src/state.rs` | 改 | 图片工作区扩两段状态与一条停止：`TranslateStage`/`ComposeStage`（与既有 `RecognitionStage` **并列不合并**，五态那条一个字不改）、`composed: Option<Composed>`、`failed_blocks: Vec<u32>`（运行态，**不进 document、不进配置**：一次尝试的失败不是那份文档的性质）、`compose_request_id`（第三次独立过期序号）、`image_cancel: Arc<AtomicBool>`、`CompositeMode`（自动/强制回填/强制覆盖层，运行态默认"自动"） |
| `crates/ice-app/src/system/panel.rs` | 改 | 加 `request_save(default_name, allowed_types)`：复用同一套 `Reply{Picked,Cancelled,Failed}` + `classify` + 容量 4 的 `try_send` + `beginWithCompletionHandler:`（**禁 `runModal`**，P6 已立）。`NSSavePanel` 的 feature 在 P6 就已经点开且零引用，本包是它第一次被用 |
| `crates/ice-app/src/system/clipboard.rs` | 改 | 加 `write_image(&RgbaImage) -> Result<(), ClipboardError>`（`arboard::set_image`，先校验 `宽×高×4 == bytes.len()`，与既有 `read_image` 对称）；在那条函数的文档注释里写死一件事：macOS 实现先 `clipboard.clear()`，**复制图片会抹掉剪贴板里的文本**（Q-004） |
| `crates/ice-app/src/views/image.rs` | 改（半页） | E 区第二排四颗（翻译全部 / 生成图片 / 保存 / 复制图片）+ 模式单选三档、G 区由那句 `"译文回填后的图（下一版）"` 换成真预览（`degraded` 橙描边、纹理 ID 带换代序号）、F 区叠加识别框 + 点击选中、I 区三段状态与那颗按段改变含义的停止按钮。**模块注释第 3 行"G 结果预览在这一版还是空的一格"必须删**（与 CHG-005 删那条"刻意不放可点控件"同一条纪律：过时的说明比没有说明更坏） |
| `crates/ice-vision/examples/` | **新** | 两个小工具，与 P6 那一对同形：`compose_metric.rs`（自绘五类已知背景的干净样图 → 走完整 S5~S7、译文用内置对照表**不打网络** ⇒ 可复现 → 报三档判定对不对、擦除后与真背景的色差、越框面积、degraded 出现率）、`samples_compose.rs`（逐张跑 `assets/samples/` 真图 → CSV 只有 `filename,blocks,replaced,overlaid,degraded,elapsed_ms`）；再造一张构造样图 IMG-13（窄框 + 已知德语长译文，Q-008） |

`ice-providers` **一个文件都不改**（`TranslationRequest` 的单字符串形状、`map_segments` 的整批失败语义、`MAX_IN_FLIGHT=4` 都留在原处，逐块隔离靠 `ice-app` 那条新 job 的循环实现）。`ice-core::document` **也不改**：`translation`/`degraded` 两字段早就在（`document.rs:86/88`），本包是它们第一个生产侧写入者。

### 依赖增量（批准前要先看清这一段）

| 增量 | 位置 | 理由 | 是否影响产物 |
|---|---|---|---|
| `ab_glyph_rasterizer` + `owned_ttf_parser` 从 dev 提为正常依赖 | `crates/ice-vision/Cargo.toml` | FR-2.4 的"绘制译文"需要一条能把字符串变成位图的路，而 egui 的字形只在 GPU 管线里成形、拿不到位图（这条理由 P6 已经写在 `Cargo.toml:67-71` 的注释里）。**不新增 lockfile 条目**：两个都在 `Cargo.lock`（Linux 的 `sctk-adwaita` → `ab_glyph` 路径带进来的），提升只是把它们从 dev 面搬到正常面，另带一个传递依赖 `ttf-parser`（依赖表为空）。所以 V-007 那条取证的判据是"正常树里新增的就是这三行、且仍无一个带网络能力的 crate"，不是"树完全没变"，源码已在 `~/.cargo/registry/src` 解包，API 已由 `examples/self_metric.rs` 在真机跑通过一整轮（画得出、`glyph_index` 判缺字形得出 0） | 是（第一次进 macOS 产品编译面；两者都不带网络、都不依赖 UI crate，NFR-3/NFR-6 不受影响） |
| `NSSavePanel` feature | **零增量** | `crates/ice-app/Cargo.toml:108` 在 P6 就点开了，注释里写明它买到的符号，至今生产代码零引用。本包只是第一次用它 | 否 |
| PNG/JPEG 编码器 | **零增量** | workspace 那份 `image`（`Cargo.toml:27`）的 `features = ["png","jpeg"]` 同时覆盖解码与编码；全仓 `src/` 生产代码目前没有一处 `image` 编码调用（`.save(`/`.write_to(` 零命中；同名不同事的 `write_to_config` 是术语表写配置），本包是第一次 | 否 |
| `arboard::set_image` | **零增量** | `image-data` 在 arboard 的 `default` 里（P6 事实表已核），`Cargo.toml` 一行不改 | 否 |
| 明确**不**引入 | — | `rfd`（面板自接，Q-002 于 P6 已决）、`ab_glyph`（那是 `owned_ttf_parser` + `ab_glyph_rasterizer` 之上的一层，本包已有更底层那两块）、`resvg`/`raqm`/harfbuzz 绑定（复杂文本排版不需要：断行自己做，字距用 advance width）、任何学习式修补或图像模型（ADR-003）、任何带网络的 crate | — |

### docs/04 要补的四个数（实现时随文档回写，SHALL NOT 只在代码里出现）

`docs/04` 的伪码有四处留白，本包必须补上数值并写明它是"可调的数"而不是新算法。四个都不进配置（延续 CHG-005 Q-006 的纪律）：

| 留白 | 本包补的数 | 为什么是这个数 |
|---|---|---|
| §3 "采样带内与 bg 距离 > **阈值** 的像素占比" | `BG_DISTANCE_THRESHOLD = 12`（三通道绝对差取 max，切比雪夫距离，0~255 尺度） | 4bit/通道量化的一格是 17，`12` 卡在"量化噪声以内"与"肉眼可辨"之间；`variance` 的两个档界（0.15/0.35）是 docs/04 给的，本包不动它 |
| §3 "`fg` = 框内像素中与 `bg` 距离最大的**那一簇**的众数" | `FG_CLUSTER_RATIO = 0.6`：取距离 ≥ `最大距离 × 0.6` 的像素为前景簇，再取其 4bit 量化众数 | 不写死这一条，实现会在"取最远那一个桶"（抗噪差）与"取上四分位"（多色文字的描边会被算进来）之间各凭喜好 |
| §4 没有行高模型 | `LINE_HEIGHT_RATIO = 1.25`（行高 = 字号 × 1.25） | 常规正文行距 1.2~1.5 的下沿；回填要的是"别压线"，宁可略松。真实截图上若必须改，属调数不改结构 |
| §4 第 5 条的对齐推断 | `ALIGN_CENTER_TOLERANCE = 0.15 × 块宽`：首行中心与块框中心之差在此内判居中，否则按左/右贴边判 | docs/04 只写了"依据首行左边界与块框中心的相对位置推断"，没给容差；没有容差这条判据无法实现 |

### 数据流（S4~S8 一条链，S0~S3 属已落地的 P6）

```
AppState 的 OcrDocument（用户已在 H 区改过字、勾过块）
        │  enabled_blocks() —— 勾掉的块从这里就开始消失（兑现 TC-2.3-b 的"不绘制"半句）
        ▼
[翻译全部]  Job::ImageTranslate{ blocks: Vec<(id, text)>, cancel }
        │     tokio 串行：一块一次 Translator::translate，Err 记下继续走下一块
        ▼     Outcome → 逐块写 TextBlock.translation；failed_blocks 单独存 AppState（运行态）
[生成图片]  Job::ImageCompose{ source: RgbaImage（内存里那份，不是磁盘）, document, mode }
        │     spawn_blocking ──► Renderer::compose
        │        ├─ S5  bg::estimate(每块)          → BackgroundEstimate{bg,fg,variance,kind,corners}
        │        ├─ S6  typeset::plan(每块)         → BlockPlan{InPlace|Overlay|Skip, font_size, lines, align, degraded}
        │        ├─     canvas = source.clone()     ← 输入永不改写（FR-2.4 的哈希判据靠这一条）
        │        ├─     for 全部块: erase(仅 InPlace) ← 先全部擦除
        │        └─     for 全部块: draw / plate     ← 再全部绘制（docs/04 §4 第 8 条）
        ▼
Composed{ image, per_block, replaced, overlaid, skipped, degraded }
        ├─► G 区预览（纹理 ID 含 compose 代序号）＋ degraded 块橙描边
        ├─► [保存] system::panel::request_save → export::save(PNG|JPEG) → 用户选中的路径
        └─► [复制图片] system::clipboard::write_image → arboard::set_image（会 clear 剪贴板，界面明说）
```

`nettop` 那条零外发取证在本包的口径 SHALL 收窄并写清：**"图片字节与像素全程不出网"仍然成立**（NFR-3 原文"图片 OCR 与合成全程本地"），但**识别出的文本经 P3 通道送出去是本包的正常行为**（NFR-3 原文"文本只在用户主动触发时外发"——用户点「翻译全部」就是那次主动触发）。`V` 里那条运行时取证的说明必须这样写，SHALL NOT 把 P6 的"识别期间零外发"照抄成"整条图片链零外发"。

## REQ-001 实现与验证

**逐块 = 一块一次请求，串行，Err 不中断**。`Job::ImageTranslate` 的循环体形状：

```
for (id, text) in blocks:
    if cancel.load(Ordering::Relaxed) { 记下"停在第 k 块"; break }
    match translator.lock().await.translate(engine, request_for(text, pair)).await {
        Ok(result) => per_block.push((id, Ok(result.text))),
        Err(err)   => per_block.push((id, Err(err))),      // ← 不 break：docs/04 §1 S4 的"单块失败不影响其它块"
    }
send(Outcome::ImageTranslated{ id, per_block })
```

`enabled_blocks()` 决定发哪几块——勾掉的块连请求都不发（这是"不翻译"那一半在 P7 的延续，CHG-005 已在 `merged_text()` 侧锁过一次）。重试一块复用**同一个 job 类型**，`blocks` 只给那一个元素，界面上那颗"重试这一块"是 `Job::ImageTranslate{blocks: vec![(id,text)]}`，不新增第三种 job。

`blocks` 的原料在派发时快照（`Vec<(u32,String)>`），任务不回读 `AppState`：帧循环与工作线程之间只走"交出去的副本 + 回来的结果"，避免 await 期间用户又改了字。**结果落地时按 `id` 写回**，块若已被删除则那一条被丢弃（与 P6 的晚到结果丢弃同一条纪律）；同一块若已有译文则被新结果覆盖（重试的语义就是覆盖）。

失败态不进 `TextBlock`：`translation = None` 同时表达"还没翻"和"翻失败"两种情况，但那是**这次尝试的性质而不是那份文档的性质**，所以放 `AppState.failed_blocks: Vec<u32>` + 一句由 `ProviderError` 的既有 `Display` 得来的原因（文本页早已有同一句映射，不新写文案）。重开界面即消失，与识别结果同为运行态。

`cancel: Arc<AtomicBool>` 与 `image_request_id` 序号**都要**，管两件不同的事：旗标让循环不再发**下一块**（真停，省额度、省等待），序号让**已经发出去的那一块**的晚到结果作废（tokio 里那个 await 不能半途撤，撤它就得 abort 整个 future，而 abort 会把同批其他结果一起丢掉）。这条与 P6 的 Q-008 一脉相承：能真停的地方真停，撤不回的老实说撤不回。

**验证**：无头——用一个假引擎（`StubEngine` 那条既有路）注入"三块里第二块 Err"，断言 `per_block` 三条各有归属、第一三条的译文写进对应块、第二条留 `None` 且 `failed_blocks == [那个 id]`；断言"请求次数 == 启用块数"（勾掉一块就少一次）；断言 `cancel` 置起后**不再发**下一块且已写入的译文不被清（Q-006 的默认）；断言停止后没有 `Composed` 产生（不残留半成品）。真机——真实端点下 N 块的耗时与限流表现，走查 W-4.2。

## REQ-002 实现与验证

`bg::estimate` 是纯函数，签名 `(&RgbaImage, &PixelRect, &CompositeOptions) -> BackgroundEstimate`，四步：

1. **采样带**：块框外扩 `padding`（2.0）得到外框，带 = 外框像素集减去内框像素集（**不含框内**——把字本身算进背景，深色模式截图必错，docs/06 的 IMG-6 就是打这个的）。贴边时把外框夹进图像范围；带内像素数低于 `MIN_BAND_PIXELS = 16` 时按 `padding/2` 再试一次；仍不足（1px 贴角那种极端）判 **`Complex`**——保守方向是"不擦"，因为擦错比不擦更难看了没人能改回来。
2. **`bg`**：带内像素按 4bit/通道量化成 4096 桶，取**计数加权（alpha 加权，`w = a/255`）的众数**；同分时保留先遇到的桶。遍历是行主序固定顺序，因此确定性成立（度量可复现的前提，与 `merge_rows` 那条 `total_cmp` 排序同一条理由）。桶用 `Vec<u32; 4096]`，不用 `HashMap`（快，且迭代序不是哈希序）。
3. **`variance`** 与三档：`max(|Δr|,|Δg|,|Δb|) > BG_DISTANCE_THRESHOLD(12)` 的带内像素占比；`< 0.15 → Uniform`、`0.15 ≤ v ≤ 0.35 → Gradient`、`> 0.35 → Complex`。**边界归哪一档写死在测试里**：`0.15` 属 `Gradient`（"≥"那一侧）、`0.35` 也属 `Gradient`（`Complex` 用严格大于），这条一旦定下不许在绘制侧再解释一遍。
4. **`fg` 与 `corners`**：`fg` 按 `FG_CLUSTER_RATIO(0.6)` 取框内距离上簇的量化众数；`corners: [Rgb;4]` 取**擦除前**图上的四个角各 3×3 均值（`Gradient` 的双线性填充要四个角，docs/04 §4 第 6 条写了双线性但 §3 的伪码里没这一项——本包补上并回写 §3）。**必须在擦除之前取**，合成侧一次性算完所有块的估计再动手改像素，否则后擦的块会把先擦的块的角读成背景（两框重叠时是真实场景，不属防御性代码）。

alpha：全图 alpha 加权统计，`export` 的 PNG 保留 alpha、JPEG 丢 alpha（REQ-008 的提示那条）。

**验证**：全部无头、零 IO。纯色带 → `Uniform` 且 `bg` 等于那个色；线性渐变 → `Gradient` 且 `variance` 落在中段；噪声/棋盘格 → `Complex`；`0.15`/`0.35` 两侧各一条并断言档位；贴边块（框贴左缘、贴角）带内仍取得到样本、极端情况判 `Complex`；深底白字取出的 `fg` 是白（这一条正好是 FR-2.8 验收"回填后文字颜色仍为白"的**可自动那半**，另一半"真图上"属 W-4.4）；alpha 加权那条（一半透明的带不该把透明像素当背景）；`corners` 取自擦除前（两块重叠时二次调用的结果与一次调用相同）。

## REQ-003 实现与验证

**输入不可变**：`compose` 第一句 `let mut canvas = source.clone();`，此后所有像素写只打在 `canvas`。无头有两条硬断言：`compose` 前后 `source` 逐像素相等；以及一条用真文件的——写一张 PNG 到 `tempfile`（`ice-vision` 的既有 dev 依赖）里、读进缓冲、`compose`、再 `std::fs::read` 一次断言**字节完全相等**。这条比哈希强：TC-2.4-a 的验收原话是"校验原图文件哈希不变"，实现里用逐字节相等做无头断言，真机走查时用 `shasum -a 256` 前后各打一次给用户看（不为此引哈希 crate）。

**三档擦除**（只打 `InPlace` 的块）：

| 档 | 打法 |
|---|---|
| `Uniform` | `bg` 实心矩形填**框内**；采样带那一圈像素一个都不动（改了它就把"背景证据"擦掉了，第二个块的 `corners` 会读到假背景） |
| `Gradient` | 四角 `corners` 双线性插值填满框内，再对**擦除区外扩 1px 的那一圈**做 3×3 均值（docs/04 §4 第 6 条"再加 1px 模糊边缘"）。SHALL NOT 用一条均值色糊过去——那正是补丁感的来源 |
| `Complex` | **一个像素都不擦**（ADR-003 的兜底，也是 L-1 的边界）；该块在计划阶段已被改判 `Overlay` |

**先全部擦除、再全部绘制**：两个独立 `for`，各自按 `block.id` 升序（顺序确定，结果可复现）。这条不是风格：docs/04 §4 第 8 条写得很直白——"避免后画的字被前块的擦除盖掉"，一个块一块地"擦完就画"在两框上下相邻时必错。无头有一例专门压它：两块垂直相邻、下块的框伸进上块的字形区，逐块处理会擦掉上块刚画的一行，两段处理不会（断言那块区域最终仍有非背景像素）。

`enabled = false` 的块不进计划（`enabled_blocks()` 已经挡在外面）；`degraded` / `Overlay` / `Skip` 三种都不擦。`Skip`（无译文，含失败块）的行为是**那块区域原样保留**——原文还在图上，这比擦掉却不写字要诚实（擦了就永久丢信息，用户还没拿到译文）。

**验证**：三档各自的像素结果（纯色擦完仍是纯色；渐变擦完仍是渐变且角色对得上；`Complex` 那一块框内外都字节不变）、"框外一圈像素一个都不动"、先擦后画次序、`Skip` 不擦、输入不变、真文件字节不变。"擦得干净"只有真机看（W-4.1 / L-1）。

## REQ-004 实现与验证

**测量走注入的 `typeset::Metrics` trait**（`text_width(text, size) -> f32`、`line_height(size) -> f32`），于是版式判据全部能在无头里测且**不依赖系统字体文件存在**；真字体那份实现是 `paint::FileMetrics`。这条是"可测性"决定，不是多余抽象——它让"缩字号缩到越框"这类断言可以用一把尺子（`宽度 = 字数 × 10px`）写得极清楚。

`plan` 的流程（docs/04 §4 第 1~5 条原序）：

1. 起始字号 = `TextBlock::estimated_font_size()`（= `bbox.h × 0.75`，下限 8.0；`document.rs:129` 已有，本包不重写）。
2. 断行：CJK（`Language::breaks_by_char`）按字符、拉丁按词，标点禁则用两个常量集 `LINE_START_FORBIDDEN = "，。、；：！？）》」』"、LINE_END_FORBIDDEN = "（《「『"`——行首不许出现前者、行尾不许出现后者。**SHALL NOT 用 `Language::width_factor` 当尺子**（docs/04 §4 第 2 条明写它只在预算不足时做初筛；本包有真实度量，那条系数留给文本页的输入框自适应用）。
3. 容量：`bbox` 内高除以 `line_height(字号)` 得可行数；`N 行 > 可行数` 时字号 × 0.95 重来，直到 `字号 ≥ 起始字号 × min_font_scale(0.55)`。
4. 缩到下限仍放不下 → 按 `CompositeMode` 分岔：
   - `Auto` / `OverlayAll` → 改判 **`Overlay`** 且 `degraded = true`（FR-2.7 的"标记该块为'降级'并用覆盖层绘制"逐字对上）；
   - `ReplaceOnly`（用户显式选了"只用回填"，即 `fallback_to_overlay = false` 的那条档）→ **照画，允许越框**，仍标 `degraded`。这里做了一个取舍并写进设计：FR-2.7 的验收是"无文字被裁切丢失"，宁可越框也不裁字；越框的块在 G 区有橙描边，用户看得见。docs/04 §1 S6 那句"超容量且不允许覆盖 → 标 `degraded`"就是这个分支，`Skip` **不是**它的出口。
5. `Skip` 只有一个来路：`translation = None`（未翻或翻失败）。REQ-004 的边界因此在 spec 里写成一句不含糊的话——SHALL NOT 把"画不下"做成静默丢字。
6. 对齐推断：首行中心与块框中心之差 ≤ `ALIGN_CENTER_TOLERANCE(0.15 × 块宽)` 判居中；否则按首行左/右边界贴哪一侧判 `Left`/`Right`。回填与覆盖层用同一个对齐。

**覆盖层底色**（REQ-006 的公式在这里，因为它是版式的一部分）：`plate = 按 fg/bg 相对亮度决定加深或变亮的 bg`，加深/变亮按步长迭代直到 `contrast(plate, text_color) ≥ 4.5`（WCAG 相对亮度公式，`bg.rs` 提供）；最坏情况落到纯黑或纯白，那必然满足（黑底白字 = 21:1），所以这个循环有界。矩形直角、内缩 `OVERLAY_INSET = 3.0`（docs/04 没要求圆角；观感属人工评分，不自己发明）。

**验证**：全部无头（假度量）。0.95 步进走几步、55% 处必须停住并改判、"放不下必须 degraded 而不是裁切"（断言计划里的行数 == 断行结果行数，一个字符都没少）、CJK 与拉丁两条断行路各一例、标点禁则把 `，` 顶到行首的断点造一例并断言它被推回上一行末、`ReplaceOnly` 档下越框但 `degraded = true`、对齐三档（居中例、左对齐例、右对齐例）、`Skip` 只由 `translation = None` 产生、对比度循环在极端输入（浅灰底浅灰字）仍 ≥ 4.5。**四项人工评分属 REQ-010，本包不自证**。

## REQ-005 实现与验证

**字形面**（`paint::FileMetrics`）：`std::fs::read(ice_core::font::HEITI_PATH)` 进 `OnceLock<Vec<u8>>`（一次读盘，之后每个 compose 都免费）；每次 `owned_ttf_parser::Face::parse(&BYTES[..], HEITI_SC_FACE)` 现场解析——**不缓存 `Face`**：它的 `Sync` 是上游实现细节而不是承诺，而缓存一个 `Vec<u8>` 是明确安全的。`Face::parse` 只读表目录，一次的成本与逐字栅格化比可以忽略；真测出来慢再缓存（不在本包预先设计）。face 0 是繁体这条已被 P6 踩过一次（`self_metric.rs` 的注释），本包继续用 1。

**度量**：`face.glyph_metrics(size, gid).advance_width()` 求宽、`ascender()/descender()` 定基线；行高用 `LINE_HEIGHT_RATIO × 字号`（REQ-004 的常量），不用字面上下伸部——那会让中英混排的同一行抖动。

**栅格化与混合**：`face.outline_glyph(gid, &mut ToPixels)` → `ab_glyph_rasterizer::Rasterizer` 的 `draw_line/draw_quad/draw_cubic` → `for_each_pixel_2d(|x, y| coverage)`，coverage 当 alpha 做 src-over 混进画布（前景色 = `BackgroundEstimate.fg`，覆盖层上则用那块的 `text_color`）。这条路 `examples/self_metric.rs` 已在真机跑通（画出来的图被 Vision 认回 1.0000 字符正确率，说明字形形状是对的）。

**缺字形如实报告**（ADR-003 那条纪律）：绘制前 `count_missing(text, face)`；有缺 → 该字符交给后备 `GOTHIC_PATH`（`self_metric.rs` 已证它一面覆盖谚文/西里尔/假名/拉丁）；两者都缺 → **不画豆腐块**，跳过那个字符并累计计数，日志记 `missing_glyphs = N`（SHALL NOT 记是哪个字——那等于把用户内容写进日志，NFR-3），界面给一句"该块有 N 个字符本机字体没有"。字体文件读不到 → `VisionError::Font`，界面上走 `state.rs:343` 既有那句"本机处理失败，请重试"（零新文案），已完成的识别与译文不消失。

**性能与线程**：栅格化是 CPU 活，整个 `compose` 在 `spawn_blocking` 里（P6 已为识别段立过同一条；那条"纹理按代缓存、否则 8000px 逐帧缩放能把 CPU 顶到听见风扇声"的教训在这里更重）。一次成品的换代是原子的：`Composed` 生成完才交给界面，中间态不外泄（REQ-009 的"不残留半成品"在这一层成立）。

**验证**：无头——自绘一张纯色底 → `compose` 画一行已知字 → 回读那块区域，断言"出现非背景像素"且"其外接框落在计划给的那一格里"、"行宽与 `Metrics::text_width` 一致"（同一把尺两头量）；缺字形计数（用 `ab_glyph_rasterizer` 测一个真不在 face 里的字符，或直接把 face 换成一个小 face 的 fixture——`tempfile` + 自造最小 TTF 不可行时，退到"后备也不覆盖时跳过并计数"这一条用桩 `Metrics` 测）。真机——"回填的字在原来那个位置、颜色对不对"（TC-2.8-a 的 ±8 通道误差）与 W-4.1。

## REQ-006 实现与验证

`CompositeMode{Auto, ReplaceOnly, OverlayAll}` 是**界面单选的运行态值**（默认 `Auto`，不进 `config.toml`）：`Auto` = 块级由 `BackgroundKind` 决定（`Complex` → 覆盖层）、`ReplaceOnly` = 强制回填（`fallback_to_overlay = false` 那条档）、`OverlayAll` = 全部块走覆盖层不擦任何像素。切换只重算计划、**不重新识别、不重新翻译**（源图与 `translation` 都在内存里；这正是需求那句"一键切换"的兑现，也是它不该花网络额度的理由）。

`OverlayAll` 有一条硬判据（TC-2.5-b 原文："全部块用覆盖层绘制，**不修改原图像素**"）：成品在**所有块框之外**与源图逐字节相同。这条比"没有擦除痕迹"精确得多，也能被无头锁住——断言用一张全图随机噪声作源，跑 `OverlayAll` 后逐像素比对块外区域。

`Composed.per_block: Vec<(u32, PlanKind)>` 让"同一张图混合两种画法"可表达（Q-001）；I 区那句话用它给出如实计数（"回填 N 块 · 覆盖层 M 块 · 跳过 K 块"），`mode_label()` 那个整图二选一的字符串随 enum 一起退场（全仓零引用已证，改它不牵动任何调用点）。

覆盖层底色的对比度、内缩、字色三条公式都在 REQ-004 那节，本节的额外判据是：`Overlay` 块 SHALL NOT 出现在擦除清单里（`Complex → Overlay` 与 `超容量 → Overlay` 两条来路汇成同一个画法，测试要能看出它们走的是同一条绘制路）。

**验证**：无头——三档单选各自产出的计划表（同一张图，`Auto` 给混合、`ReplaceOnly` 全 `InPlace`、`OverlayAll` 全 `Overlay`）、对比度 ≥ 4.5 在浅灰底浅灰字这种极端输入下仍成立、`OverlayAll` 的"块外逐字节不变"、`Complex` 与"超容量"两条改判来路画出的结果形状相同、真实渐变样图（构造，同 IMG-8 那一类）走一遍不崩。FR-2.5 的验收"同一张渐变背景图，覆盖层模式输出无可读性损失"与"覆盖层可用率 ≥ 0.9"属人工（W-4.1 / REQ-010）。

## REQ-007 实现与验证

**意图收成一条枚举，不堆四颗布尔**（用户"代码不能一直堆叠"那条纪律在这里的具体形态）：`Effects.image: Option<ImageIntent>`，`ImageIntent{ TranslateAll{retry: Option<u32>}, Compose, Save, Copy, Stop }`。模式单选与块选中是**直接写 `AppState`**（与 P6 那五种编辑动作同一条形状：界面层不写 `enabled`/`text` 字段，动作全走 `AppState` 方法），因此无头断言沿用 P6 那两条形状——"每颗入口只给自己那一条意图且互不越界"、"光是渲染一条意图都不给"。

四颗按钮的前置条件（禁用而不是消失，与 P6 的"入口在原地但禁用"同一条形态）：

| 控件 | 可用条件 | 不满足时状态行说什么 |
|---|---|---|
| 模式单选 | 有 `enabled` 块 | 三档都能选（选择只是运行态），但"生成图片"不可点 |
| 翻译全部 | 有 `enabled` 块且当前没在翻译 | "先导入一张图并识别" |
| 生成图片 | 至少一块有 `translation` | "还没有译文，先点「翻译全部」" |
| 保存 / 复制图片 | 有 `Composed` | "先生成图片" |

`degraded` 块的橙描边画在 G 区预览上（用 `theme::WARN`，与 I-03 那条 ⚠ 同一个颜色出处，不再抄一个橙）；F 区的叠加识别框按 `PixelRect` 等比映射到预览尺度（与 F 区那张纹理同一个缩放基准，`PREVIEW_MAX_EDGE`/`PREVIEW_MAX_HEIGHT` 现成），点击用 `ui.interact(...)` 命中块 → 写 `state.selected_block` → H 区那行高亮并 `scroll_to_rect` 保证可见。

**纹理换代**是这条 REQ 的缺陷预防：P6 已经把"内容 + 本代序号"当纹理 ID 的口径立过一次，本包再叠一条——`compose` 每成功一次，`compose_request_id` 递增，G 区纹理键含它。否则同一尺寸的新成品被 `load_texture_cached` 吃掉，用户看到旧图（这条一旦漏掉，用户会报"生成图片没反应"，而这正是本包最容易自证失败的地方）。

**验证**：无头——前置条件矩阵（有图/有块/有译文/有成品 × 五颗控件，逐格断言可点/禁用与那条说明文案）、`ImageIntent` 五值互不越界、渲染一帧零意图、degraded 那一格画得出描边且颜色 = `theme::WARN`、纹理键随 `compose_request_id` 变、F 区点击某个框矩形 → `selected_block` 等于那个 id 且**不派发任何 job**。真机——"框点得准、预览看着对不对"属 W-4.1。

## REQ-008 实现与验证

**保存**：`system::panel::request_save(default_name, ...)` 与 `request_open` 同一条协议（同一个 `Reply` 三态、`classify` 同一套、容量 4 的 `try_send`——**面板回信时信道满就丢回信，不阻塞回调线程**；`beginWithCompletionHandler:`，禁 `runModal`）。`NSSavePanel` 的两个专属差异要写清：`nameFieldStringValue` 给默认名（用源图文件名 + `-译文.png`，不带用户原文内容）与 `allowedContentTypes` 给 png/jpeg 两种。**同名覆盖交给原生面板自己的确认语义**（那是系统行为，不自己实现一套自动改名 `-1`——用户会以为没保存上）。

`export::save(&RgbaImage, &Path, Format)` 按扩展名选编码器（无扩展名补 `.png`，Q-005）；JPEG 走 `JPEG_QUALITY = 85` 并在源图有 alpha < 255 时给一句"JPEG 会丢掉透明"的提示（REQ-002 的"导出保留 alpha"只对 PNG 成立）。写失败（权限、目录、磁盘满）保留 `Composed` 在内存、给一句说明原因的话、重试入口——docs/04 §1 S8 的失败列原文，一字不缩。

**复制图片**：`clipboard::write_image` 构造 `arboard::ImageData{ width, height, bytes: Cow::Owned(buf) }`，先校验 `width * height * 4 == bytes.len()`（与既有 `read_image` 的 `validate_rgba` 对称，同一条"字节对不上就不交出去"的纪律）。写之前不动剪贴板、写之后不还原（Q-004）；按钮 hover 与成功提示条都写"复制图片会覆盖剪贴板里的文本"，那条提示条按既有 `TOAST_LIFETIME`（1s）淡出、没有关闭按钮（用户 2026-10-28 与同日变更 L 定下的口径）。

**隐私边界**：导出只写用户显式选中的路径；SHALL NOT 默认落盘到仓库目录、SHALL NOT 把成品写进 `features/*/evidence/`（图片内容属用户数据）；SHALL NOT 新增配置字段记住"上次保存目录"（`NSSavePanel` 自己记，那是系统行为）。

**验证**：无头——扩展名 → `Format` 的推断表（png/jpg/jpeg/大写/无扩展名/`archive.tar.gz` 这种两点名）、`nameFieldStringValue` 那个默认名的组装规则、面板协议复用那四条既有断言（取消 ≠ 失败、OK 无路径算失败、未知响应 fail-closed、信道满丢回信）、`write_image` 的字节长度校验（错的长度必须被拒）、写失败时 `Composed` 仍在。**"文件真能写出来、能被预览打开、粘到别处得到同一张图"三条只有真机能定论**（TC-2.6-a → W-4.5）。

## REQ-009 实现与验证

**三段并列，不重写 `RecognitionStage`**：`识别`（既有五态 + `OcrProgress.elapsed_ms`，一个字不改）、`翻译`（`TranslateStage{Idle, Running{done,total}, Done{ms}, Failed(原因), Stopped(k/总数)}`）、`合成`（`ComposeStage{Idle, Running, Done{回填 N · 覆盖层 M · 跳过 K, ms}, Failed}`）。I 区那一行按"当前最活跃的那段"给主语，三段各自耗时；这三段与 docs/06 的 TC-2.9-a"进度按三阶段推进且块数递增"逐字对上。

**停止那颗按钮只有一个，含义按段分**：识别段 = 忽略这次结果（撤不回，P6 已定）；翻译段 = **真停**（置 `cancel`，循环不再发下一块）；合成段 = 忽略这次结果（`spawn_blocking` 里那次计算撤不回，与识别段同理）。hover 文案必须把"这一段是真停了还是这一次作废"说清，SHALL NOT 在三段上都写"已取消"。已到手的东西不销毁：翻译段停止后 `translation` 留在块里（Q-006 的默认）、`Composed` 不产生；识别/合成段停止后按序号丢弃晚到结果、界面零变化。

**三段序号独立**：`image_request_id`（识别）、翻译 job 的 `id`、`compose_request_id`（合成）三者互不作废。这条在 P6 已锁过一次（"两条序号互不作废"），本包是第三条序号；判据同样写成一条反向断言——**合成结果晚到时，若用户中间又点了一次翻译，那条译文照旧生效而合成结果按代作废**，反之亦然。

**半成品不出现**：`Composed` 只在 `spawn_blocking` 的那个闭包算完之后整体交给界面（`apply` 那一刻换代），界面永远看不到擦了一半或画了三行的图。这是 REQ-005 那条"整体生成再换代"的下游收益，本包把它写成一条可测判据：`apply_image_compose` 的入参要么 `None` 要么完整的 `Composed`，没有中间态类型。

**验证**：无头——三态文案与耗时字段、翻译段停止后不再发请求且 `translation` 不被清、识别/合成段"忽略这次"后界面零变化、三条序号互不作废（各造一条晚到结果）、那颗按钮只在有活跃段时画得出（P6 那条 `only_a_recognition_in_flight_offers_the_ignore_button` 的同族扩展）、不出现半成品（无中间态类型这条由类型系统保证，测试断言 `Composed` 只有整体交换）。真机——三段连着走一遍的观感与耗时属 W-4.6。

## REQ-010 实现与验证

**第一层（可自动算的下限）= `examples/compose_metric.rs`**：五类自绘样图（纯色浅底深字、纯色深底浅字、水平渐变、棋盘格噪声、窄框 + 德语长译文即 IMG-13），每类的已知文本、已知背景色、已知前景色都是我造图时定下来的，所以有真值可比。译文用**内置对照表**（不经网络）⇒ 同一命令两次运行逐字节相同。报的四个数：三档判定对不对（`kind` vs 造图时的真档）、擦除后与真背景的色差（`Uniform`/`Gradient` 档，按通道 max 距离）、回填越框面积（`InPlace` 块的行盒并集与框的差集）、`degraded` 出现率与漏标数。**它是下限测试不是达标证据**：干净自绘图是这一类算法最容易的情形，与 P6 那条同一口径。`--save-dir` 只另存**自绘的输入图**，默认不落盘，SHALL NOT 存成品（成品含译文内容）。

**第二层（只有真人能定）= `examples/samples_compose.rs` + 评分表**：逐张跑 `assets/samples/` 的真图走完整链，CSV 列只有 `filename,blocks,replaced,overlaid,degraded,elapsed_ms`（**没有文本、没有译文、没有图片字节**）；打分记录进 `docs/evidence/p7-scoring.md`（`docs/04:110` 点名的留档处，那张表与它上面的指标表都在 docs/04 §5）里的四项由用户给：回填可读性 ≥4.0、回填位置准确性 ≥3.5、观感自然度 ≥3.0 三格走 `docs/06:293` 那张 1~5 分表，覆盖层可用率 ≥0.9 是**比例**、那张表还没有它的列，本包收口时补一格或由用户直接给数。目录为空时它如实输出"样本不足，FR-2.4~2.8 的人工评分记为未闭环"，**不产出任何数字**（与 `samples_run.rs` 同一条处理）。

四个数与 0.92 一个字都不动；拿自绘指标当达标证据是这条 REQ 存在的唯一理由（P6 已经防过一次，本包继续防）。

**验证**：`VERIFIED_CODE` = 全部无头单测 + 依赖图取证（提升那两个 crate 后重跑）+ `check_traceability.py` + `./sdd scan` + 密钥形态 grep；`VERIFIED_RUNTIME` = 自绘度量一轮 + 真图跑批一轮（有图的话）+ 一次完整链的三段耗时 + `nettop` 复查**图片字节零外发**（并按"数据流"那节末尾那句收窄后的口径说明文本出网属用户主动触发）；`BUSINESS_CONFIRMED` = W-4.1~W-4.6 六条走查 + 四项人工评分。

## 影响与回滚

- **涉及仓库/文件**：只动 `ice-translator`。`ice-vision`（`bg.rs`/`typeset.rs`/`paint.rs`/`export.rs` 四个新文件 + `composite.rs` 重写 + `Cargo.toml` 那段依赖位置移动 + 两个新 example）、`ice-core`（新增 `font.rs` 一个常量模块，Q-003）、`ice-app`（`jobs.rs` 两支、`state.rs` 两段状态 + 第三条序号、`views/image.rs` 半页、`views/mod.rs` 那条 `ImageIntent`、`system/panel.rs` 加 `request_save`、`system/clipboard.rs` 加 `write_image`、`fonts.rs` 改引用常量）。**`ice-providers` 零改动**（`TranslationRequest`/`dispatcher`/`parallel`/`protect` 全部原样，本包只在 `ice-app` 侧加一条循环）。
- **配置与数据**：**不新增任何配置字段**、不改 `config.toml` 格式、不写历史。`CompositeMode`、`failed_blocks`、`Composed` 全是运行态，重开即无。`assets/samples/` 只新增文件，**不改不删用户放进去的原件**；成品只写用户显式选中的路径。
- **兼容性**：`CompositeOutput`/`Renderer::compose` 的形状改变**不影响任何现有代码**（全仓零引用已证，Q-001）；`TextBlock` 两字段是既有字段第一次有写入者，serde 形状不变；`RecognitionStage` 五态不动；`Effects` 新增一条 `image` 意图字段而不是四个布尔，既有那些 `open_panel`/`paste_image`/`ignore_image` 字段不动（P6 的测试因此全部原样）。
- **测试**：现有 **525 项不许有回退**。`views/image.rs` 里以"G 区是空的一格""E 区没有回写控件"为前提的断言会随本包改写（P6 那条 `each_entry_gives_its_own_intent_and_nothing_else` 要扩到五颗入口，`only_a_recognition_in_flight_offers_the_ignore_button` 要扩成按段判断），其余 P4/P5/P6 断言全绿是硬判据。预计新增 **90~140 项、约 2,000~2,800 行**。
- **回滚**：本包改动集中在图片页与 `ice-vision`，回滚 = `git revert` 那批提交；`ice-core::font` 是纯新文件、`ice-app::fonts` 那一处改引用要一起 revert；`Cargo.toml` 的依赖位置移动 revert 后回到 dev-only 状态，无残留。文本翻译链路与识别链路（P6）不受影响是**设计约束而不是运气**（REQ-001/REQ-002 都不动 `ice-providers` 与 `vision_sys`）。
- **SDD 操作注意**：他 `./sdd approve CHG-006` 之后先 `./sdd plan-stamp`（写 design 的 Spec-Hash 与 tasks/traceability 的 baseline）、再 `./sdd refresh`（`tasks.md` 的生成头也带 Spec-Hash，不刷会让 `validate` 判视图不一致），最后 `./sdd seal-plan` 锁 `plan_hash`。顺序错了要动计划就 `./sdd replan`（见 [[sdd-plan-hash-drift]] 的教训）；`docs/` 在树指纹里，证据 `check` + `record` 要在文档定稿后一次性做完（见 [[sdd-evidence-staleness]]）。

## 验证层次

| 层 | 手段 | 覆盖 | 谁判 |
|---|---|---|---|
| `VERIFIED_CODE`（local/mock） | `cargo test --workspace`（525 项零回退）、`-p ice-vision`（`bg`/`typeset`/`paint`/`composite`/`export` 五组纯函数）、`-p ice-app`（意图矩阵、三段状态、面板协议、字节校验、纹理换代）、`-p ice-core`（字体常量单点）、`clippy -D warnings`、`fmt --check`、`check_traceability.py` | REQ-002/003/004/005/006/007/008/009 的纯函数、数据层、状态机、错误映射与文案断言；REQ-001 的假引擎隔离断言 | Agent（无头） |
| `VERIFIED_CODE`（static） | `cargo tree -p ice-vision -e normal`（提升依赖后**重跑**，仍须零网络）、`./sdd scan`、密钥形态 grep | REQ-005/REQ-010 的"结构上不外发、不泄密" | Agent，结论写进证据件 |
| `VERIFIED_RUNTIME`（sandbox） | 本机 `examples/compose_metric.rs` 一轮（自绘五类 + IMG-13）、`examples/samples_compose.rs` 一轮（真图，有则跑）、识别+合成期间 `nettop -p <pid>`、原图 `shasum -a 256` 前后、一次完整链的三段耗时实测 | REQ-001 的真实耗时与限流、REQ-003 的哈希判据、REQ-005 的字体在真机可用、REQ-009 的三段耗时、REQ-010 的第一层 | Agent 跑，数字如实记（自绘指标 SHALL NOT 冒充人工评分） |
| `BUSINESS_CONFIRMED`（business） | W-4.1~W-4.6：改一处错字看**生成图反映修改**（TC-2.3-a 欠的那半句）、三档模式各生成一次看观感、保存与复制两条出路各拿到成品并在预览里打开、深色截图看回填色、渐变按钮看覆盖层兜底、三阶段中途停止、最小窗口（640×420）下 E 区两排不塌；四项人工评分（4.0 / 3.5 / 3.0 / 0.9） | 只有真机能定的那部分 | **只有用户**（停止条件 1/2/3/4/6/8/9 都落在这里） |
