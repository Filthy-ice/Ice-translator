# CHG-006 · 图片翻译 P7：原文擦除与译文回填、覆盖层降级、导出与三阶段进度

## 原始意图与来源

- 立项要求（2026-10-19，用户原话）："写个 rust 跨平台的客户端软件。哦，别忘了做两个界面……优先针对 mac。" `docs/00-charter.md:67` 把 P7 定为"回写与合成"，验收判据原文是"**样图集人工评分达标并留档**"。
- 需求基线里本包承担的是三条 Must + 三条 Should（`docs/01-requirements.md:49-54`，逐字）：
  - **FR-2.4**（M）"译文回填：擦除原文并绘制译文，生成**新图副本**，原图文件不被修改"，验收"生成后校验原图文件哈希不变；输出图可见中文"；
  - **FR-2.5**（M）"覆盖层模式：在原文位置之上绘制带底色的译文块，不做像素擦除；复杂背景可用此模式兜底；回填效果可一键切换到此模式"，验收"同一张渐变背景图，覆盖层模式输出无可读性损失"；
  - **FR-2.6**（M）"导出：保存 PNG/JPEG 到用户选择路径，或复制图片到剪贴板"，验收"两种方式都能拿到成品并可被预览打开"；
  - **FR-2.7**（S）"字号自适应：译文超出原框时先自动缩字号（下限为原字号 55%），仍放不下则标记该块为'降级'并用覆盖层绘制"，验收"中→德长译文样图，无文字被裁切丢失"；
  - **FR-2.8**（S）"颜色与对齐还原：从原区域采样主前景色/背景色，保持左/中/右对齐"，验收"白字深底样图，回填后文字颜色仍为白"；
  - **FR-2.9**（S）"处理进度可见：分'OCR / 翻译 / 合成'三阶段显示状态与耗时，可取消"，验收"处理中能取消且不残留半成品"。
- 与 CHG-005 的接缝已经写死在两份文档里：`features/CHG-005/spec.md` 的 Q-001 把切分线定为"识别链 S0~S3 + 结果编辑归 P6；回填链 S4~S8（FR-2.4~2.9）归 P7"，它的非目标表第 1 行逐字写着"背景/前景估计、原文擦除、译文回填、覆盖层、字号自适应、颜色与对齐还原、导出（FR-2.4~2.8）| P7 = CHG-006（`ice-vision::composite` + `bg`，docs/04 §3/§4）"。CHG-005 还欠本包两句判据：**FR-2.3 的"生成图反映修改"**（它只验到"改动进了交给 P7 的那份数据"）与 **TC-2.3-a 后半句"生成图"**（那一格明写"⏸ 待 P7（CHG-006 的回填链），本包没有那条链，这一格不许打勾"）。本包兑现这两句。
- 架构决定：`ADR-003`（启发式擦除 + 覆盖层兜底，**不引学习式修补**；阈值集中在 `CompositeOptions`；CJK 字体缺字形要**如实报告**而不是静默画豆腐块；字体缺失回退并记警告 = 风险 R-6）。`ADR-002`（识别只在本地）在本包继续成立：回填链一个字节都不出网，唯一的网络行为是**逐块翻译复用 P3 的引擎**。
- 流水线来源：`docs/04-image-pipeline.md` §1 阶段表的 S4~S8 五段、§3 背景/前景估计那七行伪码（采样带 = 块框外扩 `padding(2px)`、直方图众数量化到 4bit/通道、`variance < 0.15 → Uniform / 0.15~0.35 → Gradient / > 0.35 → Complex`）、§4 版式与绘制八条（字号 0.75×框高、按实际字体度量测宽、CJK 按字符断行、0.95 步进缩到 0.55、对齐推断、三档擦除、倾斜块、**先全部擦除再全部绘制**）、§5 度量表里属本包的四项人工指标、§6 的 L-1/L-2/L-3/L-4/L-5/L-8。
- 前置状态：CHG-005 在 IMPLEMENTING（识别链已落地，2026-11-03 批准，同一批真机走查 W-3.1~W-3.6 尚未逐项定论），CHG-004 与 CHG-008 仍在收口。代码态（2026-11-04 夜间）：`cargo test --workspace` **525 passed / 0 failed / 5 ignored**（ice-app 369 · ice-core 58 · ice-providers 65 · ice-vision 33）。
- 本包**不带**任何窗口/形态/吸附/托盘内容，也不改 P5 与 CHG-008 定下的界面纪律（提示条 1s 淡出、主题滑块只在 D 状态条、圆角让位 4px）。

## 目标与非目标

**目标**：把图片页从"认得出、能改字"长成"改完能出一张成品图"的完整闭环：勾选与改过的 `OcrDocument` 逐块送进 P3 的翻译通道，单块失败不牵连其他块（REQ-001）→ 每个块框先估背景/前景与纹理档位（REQ-002）→ 按档位擦除原文，且**原图文件一个字节都不动**（REQ-003）→ 用真实字体度量做版式计划，缩字号到 55% 下限仍放不下就标 `degraded` 并改判覆盖层（REQ-004）→ 把译文画进图里，缺字形如实报告（REQ-005）→ 覆盖层模式与全局一键切换（REQ-006）→ G 区出现成品预览、F 区出现可点选中的识别框（REQ-007）→ 保存到用户选的路径或复制进剪贴板（REQ-008）→ OCR/翻译/合成三段各自的状态与耗时、"不残留半成品"的停止语义（REQ-009）→ 度量继续分两层，把"可自动算的下限"和"只有你的样图能定论的四项评分"分开写清（REQ-010）。

**非目标**（各自有归属，本包不做也不验收）：

| 不做 | 归属 |
|---|---|
| 识别质量本身（行数/块数/字符正确率 ≥0.92、合并分段是否合理） | CHG-005 的 P6；那条 Must 至今**未闭环**（`assets/samples/` 仍为空），本包不替它补数字 |
| 结果列表的五种编辑动作与 `source_rows` 行回溯 | CHG-005 REQ-005，已落地 |
| 区域截图后直接进图片流程（FR-2.10，Could） | 延期项（`docs/01-requirements.md:54` 已标"列入延期项"） |
| 学习式/扩散式图像修补 | 不做：`ADR-003` 已否（体积、离线可运行、可解释三条都不利），兜底是覆盖层 |
| 倾斜与竖排块的正仿射回贴 | L-5；本包按 Q-007 的默认**统一走覆盖层**，改版式算法形状属停止条件 3 |
| 表格块输出顺序的破平规则（同 y 带两列交错） | L-8，属 `merge_rows` 的结构改动；**本包不受影响**（回填按块框坐标绘制，不依赖列表顺序），要动也是另一个包 |
| 非 macOS 的 OCR 后端；非 mac 的图片整条链 | L-6 已定形态（入口在原地但禁用）；本包的合成器本身跨平台，但没有 OCR 就没有块可画，真机验证排 P8 的 TC-P-7 |
| HEIC/WebP 等解码（L-7）、图片持久化与历史记录里出现图片、主题包（#28）、界面多语言（#23） | 各自归属，与本包无关 |

本包**不改**文本翻译链路、不改密钥边界（ADR-005：Key 只走钥匙串，日志只有 `key_len` + `key_hash8`）、**不新增任何配置字段**（`CompositeOptions` 那四个数、置信度阈值、导出默认格式全部是代码里的常量与默认值——延续 CHG-005 Q-006 那条纪律：一个用户调不动也解释不清的阈值比一个写死的更糟）。

## 事实与假设

| 标签 | 结论 | 来源 / commit / 环境 / 时间 |
|---|---|---|
| FACT | **回写层是一个零引用的桩，形状可以自由改**：`ice-vision` 里没有 `bg.rs`（`ls crates/ice-vision/src/` = composite/error/import/layout/lib/ocr/vision_sys 七个文件，docs/04 §1 与 CHG-005 非目标表点名的那个 `bg` 模块尚未建立）；`Renderer::compose` 的函数体只有一行 `Err(VisionError::NotImplemented("译文回写在 P7 阶段接入"))`。全仓 grep（排除 `target/` 与 `.git`）`CompositeOutput` / `Renderer` / `mode_label` **只命中两处**：`composite.rs` 自身与 `lib.rs:13` 那行 re-export——**没有任何调用点、没有任何测试引用它们**。所以本包改这个 enum 的形状不是"改动已交付的接口"，是第一次真正用它 | `crates/ice-vision/src/{composite.rs:30-78,lib.rs:13}`；2026-11-04 grep 实测；`docs/04-image-pipeline.md` §1/§3 |
| FACT | **`CompositeOptions` 的四个数与 docs/04 一一对上，`mode_label` 的两个中文名字已经写好**：`fallback_to_overlay=true`、`min_font_scale=0.55`、`padding=2.0`、`border_sample_ratio=0.12`（`composite.rs:8-28`）——0.55 就是 FR-2.7 那句"下限为原字号 55%"，2px 就是 §3 伪码里的 `padding(2px)`；`CompositeOutput::mode_label()` 返回"回填"/"覆盖层"，正是 FR-2.5 与 docs/03 I-02 那三档的中文措辞。**`border_sample_ratio` 的 0.12 是 docs/04 没有的一个数**（§3 只给了"块框向外扩 padding 的一圈"），它是 P2 骨架留下的，本包要么用它定义"短边 12% 为采样带宽度上限"、要么在 §3 里把它删掉——不许留着一个没人解释的数 | `crates/ice-vision/src/composite.rs:8-60`；`docs/04` §3/§4；`docs/01-requirements.md:52` FR-2.7 |
| FACT | **失败面两层都已备好，只是还没有生产侧构造者**：`VisionError` 现有八变体，其中 `Composite(String)`、`Font(String)`、`NotImplemented(&'static str)` 三个是为本包预留的（`error.rs:22-31`）；`ice-app` 侧早已把它们映射成界面上那句"本机处理失败，请重试"（`state.rs:343`）与日志里的变体名（`jobs.rs:247-249`，注释写明"错误文本里带 `code=`/`desc_len=` 这类排障字段，那些留在 `ice-vision` 自己的日志里"）。**结论：本包新增的错误不必扩 enum、不必改文案分档**；`grep VisionError::(Composite\|Font)` 在 `crates/*/src` 里除 `error.rs` 定义外零命中，说明这两个变体至今是"接口已开、生产侧无人构造"的状态 | `crates/ice-vision/src/error.rs:1-35`、`crates/ice-app/src/state.rs:338-347`、`crates/ice-app/src/jobs.rs:238-251`；2026-11-04 grep |
| FACT | **`TextBlock` 上那两个字段就是为本包留的，且不需要改 schema**：`translation: Option<String>`（`document.rs:86`）与 `degraded: bool`（:88）默认 `None`/`false`（:107-108），**生产代码里没有任何地方写它们**（CHG-005 的 H 区把译文列恒显示 `—`，`docs/03` I-03 原文："译文列本包恒为 `—`，留这一格是为了让 P7 有地方长出来"）。`estimated_font_size()` = `bbox.h × 0.75`、下限 8.0（`document.rs:129`）已经就是 docs/04 §4 第 1 条那个起始字号。`enabled_blocks()`（:153）与 `merged_text()`（:159）给了"勾掉的块不翻不画"的现成入口 | `crates/ice-core/src/document.rs:79-160`；`docs/03-ui-design.md` I-03；2026-11-04 查阅 |
| FACT | **逐块翻译不能复用 `parallel::map_segments`**：`TranslationRequest` 收的是**一个字符串**（`request.rs:7` `{text, pair, glossary_hint, context_tail}`），`parallel.rs:24` 的 `MAX_IN_FLIGHT = 4` 与 `map_segments(items, limit, run) -> ProviderResult<Vec<String>>` 是**整批失败**语义（任一段出错整批 Err）。docs/04 §1 S4 的失败列写的是"单块失败不影响其它块；失败块标红并可单独重试"——两条语义正面冲突。可行形状是**不动 `ice-providers`**：在 `ice-app` 新建一条图片侧的 job，对每个启用块调一次 `Translator::translate(kind, req)`（`dispatcher.rs:74`），把每块的 `Ok/Err` 分别收下；`map_segments` 是 `pub(crate)`，本来就够不着 | `crates/ice-providers/src/{request.rs:7,dispatcher.rs:74,parallel.rs:20-45}`；`docs/04` §1 S4；2026-11-04 查阅 |
| FACT | **P2 已经替"翻译全部"定过并发口径**：`docs/03` I-04 原文"逐块走 `Translator`，**行间不并发（避免限流）**"。而文本页的长文提速走的是另一条（任务 #27：按句切分 + 分段并行 + `MAX_IN_FLIGHT=4`）。两者不矛盾：一个是"一次点按钮要发 N 个独立请求"，一个是"一次请求内部把长文切段"。本包按 I-004 的默认串行实现，把"N 块到底要不要并行"作为 Q-002 交回，因为它决定的是用户在电脑前等多久与撞不撞免费额度 | `docs/03-ui-design.md:148`（I-04）；`crates/ice-providers/src/parallel.rs:24`；2026-11-04 |
| FACT | **文字光栅化这条路在本仓已经跑通，且是可提升的正常依赖**：`crates/ice-vision/examples/self_metric.rs` 用 `owned_ttf_parser::{Face, OutlineBuilder}` + `ab_glyph_rasterizer::Rasterizer` 把字符串画成 `RgbaImage`（`FONT_PATH = "/System/Library/Fonts/STHeiti Medium.ttc"`、`HEITI_SC_FACE: u32 = 1`——face 0 是繁体，注释写明；`PIXEL_SIZE = 28.0`；后备 `AppleGothic.ttf` 覆盖谚文/西里尔/假名/拉丁；`count_missing()` 先按 `face.glyph_index(ch).is_none()` 数缺字形）。这两个 crate 现在挂在 `[dev-dependencies]`（`ice-vision/Cargo.toml:66-75`），那里的注释已经把提升的代价说清：它们早已在 `Cargo.lock`（Linux 的 `sctk-adwaita` → `ab_glyph` 带进来的），**不是新增 crate 名**，真实变化只有"第一次在 macOS 的正常编译面上被编译"；`cargo tree -p ice-vision -e normal` 明确排除 dev 依赖，所以 V-007 那条零网络取证在提升后仍然成立——但**要在提升后重跑一次**并留档，不能拿提升前那张输出交差 | `crates/ice-vision/examples/self_metric.rs`、`crates/ice-vision/Cargo.toml:66-75`；2026-11-04 查阅 |
| FACT | **同一份字体路径与 face 号在仓里已经有两处**：`crates/ice-app/src/fonts.rs:35-36` 的 `HEITI_PATH` + face 1（给 egui 的 UI 字体）与 `self_metric.rs` 里那份（给光栅化）。用途不同但字面量相同，正是用户 2026-10-19 之后反复强调的"代码不能一直堆叠"那一类。本包的默认做法见 Q-003 | `crates/ice-app/src/fonts.rs:30-40`、`examples/self_metric.rs` 头部；2026-11-04 |
| FACT | **保存与复制这两条导出路只差最后一步，且都不需要新 crate**：① `image` 的 workspace 声明是 `default-features = false, features = ["png", "jpeg"]`（`Cargo.toml:27`）——**PNG 与 JPEG 的编码器本来就在**，FR-2.6 要的两种格式零新增依赖；全仓 `src/` 生产代码里没有任何 `save`/`write_to`（只在 examples/tests 出现）。② `objc2-app-kit` 的 `NSSavePanel` feature **在 P6 就已经点开**（`crates/ice-app/Cargo.toml:108`，注释里写明它买到的符号），但生产代码零引用；`system/panel.rs` 只有 `request_open`，其协议形状（`Reply{Picked,Cancelled,Failed}`:13、`classify`:40、容量 4 的 `try_send` 信道:26、`ALLOWED_TYPES = png/jpg/jpeg`:51、**禁 `runModal`**）本包直接复用。③ 剪贴板写图：`arboard 3.7` 的 `set_image(&mut self, image: ImageData)` 存在（`lib.rs:146`），`ImageData{width: usize, height: usize, bytes: Cow<[u8]>}`（`common.rs:127`），macOS 实现 `osx.rs:343` 先 `clipboard.clear()` 再 `writeObjects`；本仓 `system/clipboard.rs` 已有 `read_image()`（走 `get_image`）但**没有 `write_image`**。`osx.rs:343` 那行 `clear()` 是一条**必须写进界面文案的副作用**：复制图片会抹掉剪贴板里的文本，见 Q-004 | `Cargo.toml:27`、`crates/ice-app/Cargo.toml:108`、`crates/ice-app/src/system/{panel.rs,clipboard.rs}`、`~/.cargo/registry/src/…/arboard-3.7.0/src/{lib.rs:146,common.rs:127,platform/osx.rs:343}`；2026-11-04 查阅 |
| FACT | **界面侧 P7 那一半是零控件、一处占位文字**：`views/image.rs:102-111` 的 `preview_row` 把 G 区画成固定标题 + `ui.colored_label(theme::MUTED, "译文回填后的图（下一版）")`，模块注释第 3 行明写"G 结果预览在这一版还是空的一格——它画的是'回填译文之后的图'，那是 P7 的产物"；F 区只有等比预览（`preview_pixels`:159 → `ColorImage::from_rgba_unmultiplied`:167 → `load_texture_cached("ocr_source_preview")`:125，`PREVIEW_MAX_EDGE=320`、`PREVIEW_MAX_HEIGHT=220`），**没有叠加识别框、没有点击选中**。`Effects`（`views/mod.rs:100-140`）里与本包相关的字段一个都不存在：只有 `open_panel`/`paste_image`/`ignore_image`/`job`/`request(...)`。docs/03 §2 那张 P2 版式（第 96-110 行）把 E 区第二排写成 `[翻译全部][生成图片][保存][复制图片]`、模式单选写成"默认'自动'（块级决定）"（I-02），G 区写成"结果预览（同尺寸对齐）│ 降级块用橙色描边标记" | `crates/ice-app/src/views/{image.rs:1-19,102-125,349,mod.rs:100-140}`；`docs/03-ui-design.md:92-153`；2026-11-04 |
| FACT | **进度与过期机制有一条同形状的现成路可扩**：`RecognitionStage` 五态（`state.rs:266`，空闲/识别中/完成 N 块/未发现文字/识别失败）+ 图片侧独立序号 `image_request_id`（`submit_image`:601、`apply_image`:630、`ignore_image_result`:621，与文本侧计数器互不作废）+ `OcrProgress{blocks, elapsed_ms}`（`ocr.rs:7-12`）。FR-2.9 要的"三阶段"是**在这条链上再加两段状态**，不是重写它；CHG-005 的 Q-008 已经给出两段的语义差别——识别段撤不回来（objc2 `!Send`），而**翻译段是 tokio 里的网络任务，可以在块边界真停**。合成段是 `spawn_blocking` 的 CPU 活，语义与识别段相同（丢弃结果，不假装中断了那次计算） | `crates/ice-app/src/state.rs:266,601,621,630`、`crates/ice-vision/src/ocr.rs:7-12`、`features/CHG-005/spec.md` Q-008；2026-11-04 |
| FACT | **本包要兑现的两条判据至今在文档里挂着未合**：`docs/06-test-cases.md` §4 里 TC-2.4-a（原图哈希不变 + 中文出现在原区域）、TC-2.5-a/b（覆盖层、强制全局覆盖层）、TC-2.6-a（保存 + 复制可被预览打开）、TC-2.7-a（中→德长译文窄块，**要一张构造出来的 IMG-13**）、TC-2.8-a（IMG-6 深色截图，回填色与原前景色 ±8 通道误差）、TC-2.9-a/b 六行全部写着"待 P7"；TC-2.3-a 的后半句与 TC-2.3-b 的"不绘制"半句各自明写"⏸ 待 P7"。§4 的样图集 IMG-1~12 中 IMG-9/12 是**故意纳入的失败样例**，不参与达标均值但必须给降级而不是崩溃；`docs/04` §5 的四项人工指标（回填可读性 ≥4.0、位置准确 ≥3.5、观感 ≥3.0、覆盖层可用率 ≥0.9）在 `docs/06:291-299` 的评分表**是空模板** | `docs/06-test-cases.md` §4（TC-2.4~2.9 各行、IMG 表、评分表）；2026-11-04 查阅 |
| FACT | **真实样图仍然缺原料**：`assets/samples/` 自 2026-10-28 起是空目录，CHG-005 的 Q-003 到本轮仍未解（其 TC-2.2-a 那格记的是"⛔ 未闭环，不是通过"）。这条对本包的直接后果有两条：① FR-2.4~2.8 的四项人工评分同样拿不到真实数字，只能与 P6 一样分两层；② `docs/06` TC-2.7-a 那张 IMG-13（窄块 + 德语长译文）本来就不是"截图"，是可以程序构造的一张图——本包默认自己造（Q-008） | `assets/` 实测（CHG-005 事实表第 15 行留档）；`docs/06-test-cases.md` TC-2.2-a / TC-2.7-a；2026-11-04 |
| FACT | **当前测试与门禁基线**：`cargo test --workspace` = **525 passed / 0 failed / 5 ignored**（ice-app 369 · ice-core 58 · ice-providers 65 · ice-vision 33；ignored 是 `ice-providers/tests/live.rs` 那五条要打真实端点的实测），`clippy --workspace --all-targets -- -D warnings` exit 0、`fmt --all --check` 无差异、`tools/check_traceability.py` 45 ↔ 45。本包收口时这 525 项**零回退**是硬约束 | `docs/07-acceptance.md` 2026-11-04 那两行计数；2026-11-04 实测 |
| HYPOTHESIS | **Uniform 档的实心矩形擦除 + Gradient 档的四角双线性 + 1px 模糊，对软件截图足够无痕**：docs/04 §4 第 6 条的形状就是按"界面截图以纯色与线性渐变为主"设计的，但本机从未真跑过一次；照片/纹理背景按 L-1 明知做不到（靠 Complex → 覆盖层兜底）。若真机看下来"补丁感"蔓延到本该是 Gradient 的那一类，触发停止条件 1 | `docs/04` §3/§4、§6 L-1；ADR-003；2026-11-04 前无回填运行记录 |
| HYPOTHESIS | **STHeiti Medium.ttc 的 face 1 覆盖本包要画的全部字形**（中英日 + 拉丁字母的德/法/西译文；`self_metric.rs` 用它画 `en_plain`/`zh_plain`/`paragraph` 三档得到 1.0000 字符正确率，说明这些字形都画得出来）。目标语言超出中英日（`mixed` 与变更 M 那两条非中英用例走的是 AppleGothic）时的缺字形风险，由 `count_missing` 那条纪律在生成时如实报出来，不静默画豆腐块（ADR-003） | `crates/ice-vision/examples/self_metric.rs`；`docs/04-image-pipeline.md` §2.2；2026-11-04 |
| HYPOTHESIS | **一张 1080p 图的合成耗时可接受（目标 <300ms）**：逐像素采样带 + 双线性填充 + 若干百个字形的光栅化都是本地 CPU 活；P6 的自绘识别耗时 32~76ms 只覆盖识别段。慢了要在 I 区出现可感知等待，属需求侧新信息，回来登记 | `docs/04-image-pipeline.md` §2.1 末尾那句耗时观察；TC-P-3 的 1.5s 预算；2026-11-04 |
| HYPOTHESIS | **块级"自动"模式的判据（`variance` 阈值 0.15 / 0.35）在真实截图上分得开**：这两个数来自 docs/04 §3，从没在真图上校准过（P6 的自绘度量不涉及背景估计）。本包按原值实现、把三档判定写成纯函数，调阈值只改数值不改结构（与 CHG-005 REQ-004 同一条纪律） | `docs/04` §3；2026-11-04 |

## 行为与验收

### REQ-001: 逐块翻译，一块失败不牵连其他块（FR-2.3 后半句的起点、FR-2.4 的前半、docs/04 §1 S4）

WHEN 用户在识别结果都改完之后点「翻译全部」
THEN 系统 SHALL 对**每一个 `enabled` 块**单独构造一次 `TranslationRequest`（`text` = 该块 `TextBlock.text`，`pair` = 当前语言对，术语表与上下文尾巴按 P3 既有规则），并把每块的结果分别落到 `TextBlock.translation`；SHALL NOT 把所有块的文本拼成一次请求（拼起来翻就等于把 H 区辛苦分开的手动合并又合回去了，且失败无法归位）。

WHEN 其中一块翻译失败（限流、网络、空译文、配额）
THEN 其余块 SHALL 照常完成并各自落进 `translation`；失败的那一块 SHALL 保持 `translation = None`、在 H 区该行标出失败态，并给一条**只重试这一块的入口**（docs/04 §1 S4 的"失败块标红并可单独重试"）；SHALL NOT 因为一块失败把整批回滚，SHALL NOT 静默改走另一个引擎（D-20 那条纪律：换通道是业务决定，交回用户）。

WHEN 翻译在进行中
THEN 系统 SHALL 一次只让一个图片翻译任务在飞（与 CHG-005 的"同一时刻只允许一次识别"同一条纪律）；界面 SHALL 可切页签、可拖窗；SHALL 在块与块的边界响应停止（REQ-009）。

WHEN 语言对是"自动检测"（`source = auto`）
THEN 派发前 SHALL 用 P3 既有的检测/方向推断决定这一批的实际方向，SHALL NOT 把 `auto` 原样交给机翻端点（P3 的 `pair` 已经是具体语言对，这条纪律在那里，本包只是不许绕过它）。

边界：本 REQ **不改 `ice-providers` 任何签名**（`map_segments` 的整批失败语义与 `TranslationRequest` 的单字符串形状都不动，事实表已给理由）；逐块的串行/并行由 Q-002 定，默认串行（docs/03 I-04 原文）。
并发：翻译任务在 tokio 里跑，通过既有的 `OutcomeReceiver` 回帧循环；`Translator` 句柄仍是 `Arc<Mutex<..>>`（`jobs.rs:3-4` 的注释解释了为什么 await 期间持锁）。
验证层次：无头测"每块一次请求、请求数 = 启用块数、单块 Err 不影响其余 Ok、None 的块不进后续绘制"（用 `StubEngine` 或注入的假引擎，不打真实端点）；"真实端点在 N 块下的耗时与限流"属真机 + 真实额度，走查 W-4.2。

### REQ-002: 每个块先估背景/前景与纹理档位（FR-2.8 的前半、docs/04 §3）

WHEN 一个块的 `PixelRect` 与源图交给背景估计
THEN 系统 SHALL 用**纯函数**产出 `BackgroundEstimate{bg, fg, variance, kind}`：采样带 = 块框向外扩 `padding`（默认 2px）的一圈**边框像素，不含框内**；`bg` = 采样带颜色直方图的众数（量化到 4bit/通道）；`variance` = 采样带内与 `bg` 距离超过阈值的像素占比；`fg` = 框内像素中与 `bg` 距离最大那一簇的众数。

WHEN `variance` 落在三档边界
THEN `variance < 0.15` SHALL 判 `Uniform`（可回填）、`0.15 ≤ variance ≤ 0.35` SHALL 判 `Gradient`（回填 + 边缘过渡）、`> 0.35` SHALL 判 `Complex`（**强制覆盖层，不擦**）；边界上的取值归哪一档 SHALL 由单测钉死（`0.15` 与 `0.35` 各测两侧）。

WHEN 块贴着图像边缘（采样带会跑到图外）
THEN 系统 SHALL 缩小采样带继续估计（docs/04 §1 S5 的失败列），SHALL NOT 因为采样区不足返回错误、也 SHALL NOT 拿框内像素当框外采样（那会把文字本身算进背景，深色模式截图必错）。

WHEN 图片带 alpha（透明或半透明像素）
THEN alpha SHALL 加权参与统计，导出 SHALL 保留 alpha（docs/04 §3 末行）。

边界：本 REQ 不画任何东西、不改像素，只做估计——正因为如此三档判定是纯可测函数。`kind` 的三档与 `CompositeOptions` 的四个阈值 SHALL 有一个唯一出处（SHALL NOT 在绘制侧再抄一份数）。
验证层次：无头测纯色（`Uniform`）、线性渐变（`Gradient`）、噪声/照片（`Complex`）、贴边块、alpha 加权、`fg` 与 `bg` 在深色底上取反方向各一组；"真实截图上这三档分得对不对"属人工，走查 W-4.4（对应 `docs/04` §5 的观感自然度）。

### REQ-003: 擦除原文，且原图文件一个字节都不动（FR-2.4 的哈希判据、docs/04 §4 第 6 条）

WHEN 一批块的版式计划就绪、进入合成
THEN 系统 SHALL 在**源 `RgbaImage` 的一份副本**上作画，全程 SHALL NOT 借用或改写输入那张图（`Renderer::compose(&self, source: &RgbaImage, ...)` 的 `&` 已经是形状保证，本条是把它写成纪律）；`FR-2.4` 那句"原图文件不被修改"的验收（"生成后校验原图文件哈希不变"）SHALL 由一条自动断言承担：同一张 PNG 进 `compose` 前后，磁盘上那个文件的哈希不变 + 传入的 `RgbaImage` 逐像素不变（`Clone` 后比对）。

WHEN 一个块的档位是 `Uniform`
THEN 擦除 SHALL 用 `bg` 的实心矩形填充该块框（含 `padding` 那圈的处理口径要写死：填的是**框内**，采样带本身不许改）。

WHEN 档位是 `Gradient`
THEN 擦除 SHALL 用框四角采样色的双线性插值填充，再加 1px 模糊边缘（docs/04 §4 第 6 条原文）；SHALL NOT 用一条均值色糊过去（那正是"补丁感"的来源）。

WHEN 档位是 `Complex`
THEN 系统 SHALL NOT 擦该块的像素，SHALL 走覆盖层（REQ-006）；这是 ADR-003 的兜底，也是 L-1 的边界。

WHEN 一批块里既有擦除又有绘制
THEN 顺序 SHALL 是**先全部擦除、再全部绘制文字**（docs/04 §4 第 8 条），SHALL NOT 按块逐块"擦完就画"——后画的字会被前一块的擦除盖掉。

边界：非 `enabled` 的块 SHALL NOT 被擦（勾掉 = 这块当它不存在，与 CHG-005 的"不翻译"同一语义，这里兑现"不绘制"，正好补上 TC-2.3-b 那格的 ⏸）；`degraded` 块 SHALL NOT 被擦（它走覆盖层）。
验证层次：无头测三档各自的像素结果（纯色框擦完仍是纯色、渐变框擦完仍是渐变、Complex 档那一片字节 SHALL 与副本外完全一致）、先擦后画的次序、输入未被改写、`enabled=false` 与 `degraded` 两条"不擦"；**"擦得干净不干净"（补丁感）只有真机看**，属 W-4.1/L-1。

### REQ-004: 版式计划——按真实字体度量缩字号，缩不下去就降级并改判覆盖层（FR-2.7、FR-2.8 的对齐半、docs/04 §4 第 1~5 条）

WHEN 一个块的译文要进绘制
THEN 系统 SHALL 先产出块级计划 `BlockPlan{ InPlace | Overlay | Skip, font_size, lines }`：起始字号取 `TextBlock::estimated_font_size()`（= `bbox.h × 0.75`，下限 8.0），**每行宽度用实际字体度量算**，SHALL NOT 用字符数估算（`Language::width_factor` 只在预算不足时做初筛，docs/04 §4 第 2 条原文）。

WHEN N 行放不进框容量
THEN SHALL 按 0.95 步进缩字号，直到 `min_font_scale`（0.55 = FR-2.7 那句"下限为原字号 55%"）；到下限仍放不下时：`fallback_to_overlay` 为真 SHALL 改判 `Overlay` **且把该块标 `degraded = true`**；SHALL NOT 裁掉多出来的行（FR-2.7 的验收"无文字被裁切丢失"就是这一条）。

WHEN 断行
THEN CJK SHALL 按字符断行（`Language::breaks_by_char`）、拉丁 SHALL 按词断行、标点 SHALL NOT 落在行首（docs/04 §4 第 3 条）。

WHEN 决定对齐
THEN SHALL 沿用原块的对齐（依据首行左边界与块框中心的相对位置推断左/中/右，docs/04 §4 第 5 条），并把同一个对齐用于回填与覆盖层。

WHEN 块的档位是 `Complex` 或计划是 `Overlay`
THEN 该块 SHALL NOT 出现在擦除清单里、SHALL 出现在覆盖层绘制清单里；`degraded` 的块 SHALL 在 G 区预览里用橙色描边标出（docs/03 P2 版式第 102 行的原话："降级块用橙色描边标记"）。

边界：单块译文长度沿用 CHG-005 的 1200 字符上限（不新增第二个数）；`Skip` 这个档位本包**只留给"无译文"一种情形**（`translation = None` 且非用户主动勾掉）——SHALL NOT 拿它当"画不下就算了"的出口，那样等于静默丢字。
验证层次：无头测缩字号步进、55% 下限处改判、"放不下必须标 degraded 而不是裁切"、标点不落行首（把 `，。` 顶到行首的断点各造一例）、CJK 与拉丁两条断行路、对齐推断三档、`degraded → Overlay` 与 `Complex → Overlay` 两条独立来路汇成同一个绘制清单；"中→德长译文在真图上到底裁不裁"是 TC-2.7-a 那张构造样图，属 W-4.3。

### REQ-005: 把译文画进图里，缺字形如实报告（FR-2.4 的"中文出现在原区域"、docs/04 §4、ADR-003 R-6）

WHEN 一份 `BlockPlan` 集合交给绘制
THEN 系统 SHALL 用纯 Rust 的字形光栅化（`owned_ttf_parser` + `ab_glyph_rasterizer`，同 `examples/self_metric.rs` 那条已跑通的路）把每行画进副本：前景色取 REQ-002 估出的 `fg`（回填）或覆盖层底色上的高对比色（REQ-006），位置由 `BlockPlan` 的行盒决定，SHALL NOT 依赖 egui（那是 GPU 管线，拿不到位图；`ice-vision/Cargo.toml:69-70` 的注释已经把这件事写明）。

WHEN 目标字体缺某个字形（`face.glyph_index(ch).is_none()`）
THEN 系统 SHALL 如实报告而不是静默画豆腐块：至少一条 `Font` 类错误/警告带**缺字形计数**（SHALL NOT 把缺的那个字符本身写进日志——那等同把用户内容写进日志，NFR-3），界面上给"该块部分字符本机字体没有，已按后备处理"级别的一句话（ADR-003 对 CJK 字体那条纪律）。

WHEN 指定的字体文件不存在或 face 索引读不到
THEN SHALL 返回 `VisionError::Font`，界面走 `state.rs:343` 既有那句"本机处理失败，请重试"，SHALL NOT panic、SHALL NOT 让已完成的识别结果消失（用户的图与列表还在）。

WHEN 绘制完成
THEN SHALL 产出 `CompositeOutput`，其两个计数（`replaced_blocks` / `overlaid_blocks`）SHALL 与实际画的块数一致；`mode_label()` 给出的"回填"/"覆盖层" SHALL 是界面 I 区那句话的同一个词（不再抄第三个字符串字面量）。

边界：字形光栅化是 CPU 活，SHALL 与合成同段跑在 `spawn_blocking`（CHG-005 已把"大图逐帧缩放能把 CPU 顶到听见风扇声"写过一次教训，这次是它更重的那一位）；`ab_glyph_rasterizer`/`owned_ttf_parser` 从 dev 提升为正常依赖**不新增 crate 名**（事实表），但提升后要重跑一次 `cargo tree -p ice-vision -e normal` 的零网络取证。
验证层次：无头测"画进内存的 RGBA 里，指定区域出现了非背景像素且形状与预期字格一致"（自绘一张 → 回填 → 采样回读，走 `self_metric.rs` 同款手法）、缺字形计数、字体缺失那条错误路、两个计数与计划表一致；真机看的是"回填后的字在原来那个位置、颜色对不对"（TC-2.8-a 的 ±8 通道误差）与 W-4.1。

### REQ-006: 覆盖层模式与一键切换（FR-2.5）

WHEN 一个块走覆盖层绘制
THEN 系统 SHALL 在该块位置画一块底色再画译文，**不做任何像素擦除**（FR-2.5 原文"不做像素擦除"）；底色 SHALL 取该块 `bg` 的加深或变亮版本，方向由 `fg` 与 `bg` 的相对亮度决定，SHALL NOT 硬编码"半透明黑底白字"（docs/04 §3 原文），且**译文与底色的对比度 SHALL ≥ 4.5:1**。

WHEN 用户点模式单选「回填」/「覆盖层」
THEN 系统 SHALL 提供 `docs/03` I-02 定的三档（默认**自动** = 块级决定；另两档强制全局），切换 SHALL 只改下一次合成的计划、**不重新识别、不重新翻译**（已有 `translation` 与源图都在内存里）；这条就是需求里那句"回填效果可一键切换到此模式"。

WHEN 同一张图里既有可回填块又有 `Complex` 块
THEN 系统 SHALL 允许**同一张成品图里混合两种画法**（块级决定，docs/04 §6 的措辞是"Complex → 强制覆盖层"，需求没有承诺"一张图只能整图一种模式"）；`CompositeOutput` 因此 SHALL 携带逐块的画法记录，具体形状见 Q-001——SHALL NOT 为了迁就现在那个二选一的 enum 而把整图强行判成一种模式。

WHEN 用户在覆盖层模式下看一张本来全是 `Uniform` 的图
THEN 覆盖层 SHALL 照样画得可读（TC-2.5-b 的判据："全部块用覆盖层绘制，不修改原图像素"——这条有硬判据：成品与源图在**所有块框内**之外逐字节相同）。

边界：覆盖层的圆角、内边距、透明度全部是本包常量，不进配置；一张图里覆盖层块数 SHALL 有上限保护（与 `MAX_EDGE_PX` 同族纪律，防极端密排图把成品画糊）。
验证层次：无头测对比度 ≥4.5:1（按 WCAG 相对亮度公式，可自动算）、底色方向由亮度决定（深底浅字/浅底深字两例反向对照）、"强制覆盖层不动块框外一个像素"、三档单选各自产出的计划表；"同一张渐变背景图覆盖层输出无可读性损失"（FR-2.5 的验收）与"覆盖层可用率 ≥0.9"是人工，属 W-4.1/REQ-010。

### REQ-007: G 区出现成品预览，F 区出现可点选中的识别框（FR-2.4 验收的界面侧、docs/03 I-02/I-05/I-08）

WHEN 合成完成
THEN G 区 SHALL 显示结果图（与 F 区同尺寸等比、同一缩放基准，docs/03 P2 版式那句"结果预览（同尺寸对齐）"），`degraded` 块的框 SHALL 用橙色描边标出；SHALL NOT 把成品静默落盘（只在内存，导出才写文件）。

WHEN 用户在 F 区点某一个识别框
THEN 系统 SHALL 把 H 区滚动/高亮定位到那一块（`docs/03` I-08："叠加识别框（可点击选中）"，CHG-005 把它的状态记为"那一格要画的是'按这个框擦像素'的预览，本包的框只有数据没有画"）。

WHEN 尚无识别结果或尚无译文
THEN E 区那四颗新按钮（翻译全部/生成图片/保存/复制图片）SHALL 按前置条件禁用而不是消失（与 CHG-005 的"入口在原地但禁用"同一条形态纪律），且 SHALL NOT 让用户点了之后什么也不发生却不给原因——缺哪一步就在状态行说哪一步。

WHEN 成品图很大
THEN 预览 SHALL 沿用 `PREVIEW_MAX_EDGE`/`PREVIEW_MAX_HEIGHT` 的等比缩放与"纹理按内容 + 本代序号建 ID"那条既有路（同图不逐帧重算像素），且**纹理换代序号 SHALL 在每次重新生成后变化**（否则同尺寸的新成品被缓存吃掉，用户看到旧图）。

边界：本 REQ 不改窗口形态、不加新的 `Effects` 之外的通道；E 区那一排按钮的版式属 `docs/03` 已定内容，若最小窗口（640×420）下塌了属停止条件 7（CHG-005 已为同类留过一条）。
验证层次：无头测四颗按钮的前置条件矩阵（有图/有块/有译文/有成品 × 四颗，逐格断言"禁用还是可点"）、G 区那一格在成品存在时不再画占位文字、纹理 ID 换代；"框点得准不准、预览看着对不对"属真机走查。

### REQ-008: 导出——保存到用户选的路径，或复制图片到剪贴板（FR-2.6）

WHEN 用户点「保存」
THEN 系统 SHALL 弹出**原生**保存面板（`NSSavePanel`，feature 已在依赖里开着），SHALL 走异步 `beginWithCompletionHandler:` 形态（与 `system/panel.rs::request_open` 同一条协议：回信只带结果、取消与失败分得开、信道满丢回信不阻塞），SHALL NOT 用 `runModal`（P6 已禁，会冻帧）；写入格式按用户选的扩展名在 PNG/JPEG 之间决定，默认 PNG（见 Q-005）。

WHEN 用户点「复制图片」
THEN 系统 SHALL 走 `arboard::set_image`（本仓新增一个 `clipboard::write_image`，与既有 `read_image` 对称，字节序按 `CGImageAlphaInfo::Last` = 直通 RGBA，与 `RgbaImage` 一致）；由于 macOS 实现先 `clipboard.clear()`，**这一步会抹掉剪贴板里原有的文本**，界面 SHALL 说明这一点（见 Q-004），SHALL NOT 擅自把原剪贴板文本"备份后还原"。

WHEN 保存目标已存在同名文件
THEN 系统 SHALL 交给原生面板自己的覆盖确认（那是系统语义，不自己实现一套），SHALL NOT 静默改名（`-1`/`-2` 那种自动改名会让用户以为没保存上）。

WHEN 写入失败（权限、磁盘、路径是目录）
THEN SHALL 给一句说明原因的话、**保留成品图在内存**、给重试入口（docs/04 §1 S8 的失败列原文："写文件失败 → 保留结果图在内存，提示重试"）；SHALL NOT 清空已完成的识别/译文。

边界：导出 SHALL 只写到用户显式选中的路径，SHALL NOT 默认落盘到仓库目录、`features/*/evidence/` 或临时目录留一份副本（图片内容属用户数据，NFR-3）；本包 SHALL NOT 新增配置字段（上次保存目录不进 config）。
验证层次："文件真的写出来了、能被预览打开、剪贴板粘贴到别处得到同一张图"三条**只有真机能定论**（TC-2.6-a → W-4.5）；无头测编码器选择、扩展名推断、面板协议复用（取消 ≠ 失败）、写失败的内存保留、`write_image` 的字节长度 = `宽×高×4` 校验。

### REQ-009: 三阶段进度与"不残留半成品"的停止语义（FR-2.9）

WHEN 用户走完一条图片处理链
THEN I 区 SHALL 显示三段各自的状态与耗时：`识别`（沿用 CHG-005 的 `RecognitionStage` 五态与 `OcrProgress.elapsed_ms`，**不重写它**）、`翻译`（N/M 块 + 耗时）、`合成`（耗时）；FR-2.9 的验收"分三阶段显示状态与耗时"落在这一条。

WHEN 用户在**翻译段**点停止
THEN 系统 SHALL 在块边界真停：不再发下一块请求、已完成块的译文留在原处（那是用户已经拿到的东西，不是半成品）、整图产出 SHALL NOT 发生；文案 SHALL 说清"已停止这次处理，译文还在"，SHALL NOT 写"已撤销"。

WHEN 用户在**识别段或合成段**点停止
THEN 这两段的底层调用都撤不回（识别段因 objc2 `!Send`，见 CHG-005 Q-008；合成段是 `spawn_blocking` 里的一次同步计算），语义 SHALL 与 P6 一致：**丢弃这一次的结果**、界面立刻回空闲、晚到结果按序号丢弃、SHALL NOT 谎称撤回了那次计算。

WHEN 停止发生
THEN SHALL NOT 残留半成品：G 区 SHALL 保持上一次成功的成品或那句占位状态（按 REQ-007 的前置条件），SHALL NOT 出现"画了一半的图"（合成 SHALL 在副本上完成后再一次性换代，不在中间态发布）。

边界：三段共用一条"停止"入口，按当前段决定它是"真停"还是"忽略这次结果"——这一点 SHALL 在 hover 文案里说明白（延续 P6 那颗「忽略这次识别」的诚实口径）；三段序号 SHALL 独立过期（识别段的晚到结果不许作废翻译段，反之亦然；图片侧已有 `image_request_id` 的先例，扩展它而不是再造一套全局锁）。
验证层次：无头测三态各自的状态文案、翻译段在块边界停且已完成的 `translation` 不被清、识别/合成段"忽略这次结果"后界面零变化、三段的过期互不越界、不出现半成品；"三阶段看着顺不顺、耗时对不对"属真机。

### REQ-010: 度量继续分两层，人工评分不许自证（FR-2.4~2.8 的验收方式）

WHEN 本包要给出"回填好不好"的结论
THEN 系统 SHALL 分两层：**可自动算的下限**——`crates/ice-vision/examples/` 下的自绘度量：把已知文字画成干净样图（复用 `self_metric.rs` 那条渲染路）→ 走完整 S4~S7（译文用固定的对照表，不打网络，保证可复现）→ 算「块框内原文是否已被擦净」（采样色差）、「回填文字是否越出框外」、「degraded 是否按判据出现」；这些是**下限测试**，不是达标证据。

WHEN 需要"回填可读性 ≥4.0 / 位置准确性 ≥3.5 / 观感自然度 ≥3.0 / 覆盖层可用率 ≥0.9"这四项结论
THEN 这四项 SHALL 由用户按 `docs/04` §5 与 `docs/06` §4 的评分表人工打分，SHALL NOT 由自动指标替代；`assets/samples/` 仍为空时这四项 SHALL 记为**未闭环**而不是 ✅（与 CHG-005 对 0.92 的处理同一条口径，四个数一个字不动）。

WHEN 度量与取证涉及真实图片
THEN 输出的 CSV/日志 SHALL 只含文件名、块数、计数、耗时、尺寸与错误码，SHALL NOT 含图片字节、识别文本或回填的译文；`--save-dir` 那类另存只存**我自己造的输入图**，不存成品（成品含译文内容，同 NFR-3）。

WHEN TC-2.7-a 那张"中→德长译文窄块"样图不存在
THEN 系统 SHALL 按 Q-008 的默认程序构造它（窄框 + 已知德文长译文，内容与语言都是我给的，不涉及用户数据），并在 `docs/06` 的 IMG 表登记为 IMG-13（构造）与 IMG-1~12（真实）的区别。

边界：本 REQ 不改业务行为，只管"这个包的 ✅ 怎么来的"；`docs/06` §4 那 6 行"待 P7"在实现阶段逐条补结论列，并新增 W-4.1~W-4.6 走查行；`docs/evidence/p7-scoring.md`（docs/04 §5 点名的评分留档处）由用户打分后登记。
验证层次：`VERIFIED_CODE` = 全部无头单测 + 自绘度量 + 依赖图取证；`VERIFIED_RUNTIME` = 真机一次完整链的三段耗时、保存/复制两条出路的实拿、`nettop` 复查零外发；`BUSINESS_CONFIRMED` = 四项人工评分与"看着可用"。

## 未决问题

| id | 问题 | 阻塞 | 结论 |
|---|---|---|---|
| Q-001 | `CompositeOutput` 现在是个**二选一**的 enum（`Replaced{image, replaced_blocks}` \| `Overlay{image, overlaid_blocks}`），而 docs/04 §6 与 REQ-006 要的是"同一张图里 `Complex` 块走覆盖层、`Uniform` 块走回填"**混合**。形状要不要改？ | 否 | **默认改**（事实表已证这个 enum 全仓零引用，改它不牵动任何调用点与测试）：成品只有一份图，另带**逐块画法记录**（块 id → `InPlace`/`Overlay`/`Skip`）与两个计数，`mode_label()` 改成"含 N 块覆盖层"这类如实表达，不再承诺整图一种模式。备选：保留 enum、把 `Complex` 块在自动模式下也硬塞进回填（违背 §3 的判定与 ADR-003 的兜底，我不选）或整图二选一降级（违背 FR-2.7"仍放不下则用覆盖层绘制"）。你要坚持保留原形状就说一句，我按"整图一种模式 + `Complex` 块跳过不画"改 REQ-003/006 并重算哈希 |
| Q-002 | 逐块翻译**串行还是并行**？`docs/03` I-04 定了"行间不并发（避免限流）"，而文本页长文提速那条已经证明并行能快 4 倍（`MAX_IN_FLIGHT=4`） | 否 | **默认按 I-04 串行**（P2 已定的口径不动它，且机翻端点的免费额度是按请求数算的）。代价写清：20 个块 × 一次网络往返，用户可能在电脑前等十几秒——那正是 REQ-009 要把"翻译 N/M 块"显示出来的理由。若你嫌慢，选并行：改一处 `join_all` 加限流，`REQ-001` 那句"一次只让一个任务在飞"不变（在飞的是**这次批量请求**，不是连接）。**不改需求，只改等待时长与限流风险**，所以交回你定 |
| Q-003 | 回填要的字体路径与 face 号，仓里已经有两处字面量（`ice-app/src/fonts.rs:35-36` 给 egui、`self_metric.rs` 给光栅化） | 否 | **默认收一处**（用户 2026-10-19 之后反复强调"代码不能一直堆叠、提取公共和可复用的"）：把 `STHeiti Medium.ttc` 路径 + face 1 那两个常量提升到 `ice-core` 的一个位置，`ice-app::fonts` 与本包的绘制侧都引用它，`examples/self_metric.rs` 也改过去。**代价**：动到 P5 已验收的 UI 字体那一条（一个常量，不是行为改变）。备选：两处各留（更快但重复）；若你选各留我在 `docs/05` §6 记一条"同一个字面量允许出现在两个 crate"的例外 |
| Q-004 | 「复制图片」会抹掉剪贴板里的文本（`arboard` 的 macOS 实现 `osx.rs:343` 先 `clipboard.clear()` 再 `writeObjects`，源码实测） | 否 | **默认接受这条系统行为 + 在按钮 hover 与提示条里明说"复制图片会覆盖剪贴板里的文本"**（这是 macOS 所有 App 的共性行为，不是本包的缺陷）。不做的事：先读原剪贴板文本、复制完再偷偷还原——那会在用户没要求的时候两次改写剪贴板，还可能把几 MB 的旧图留在内存里。备选：把"复制图片"换成"复制成文件（写临时文件再 `writeURLs`）"，能同时保留文本表示，代价是引入临时文件（违背 REQ-008 的"不留副本"），需要你批准才做 |
| Q-005 | FR-2.6 说"保存 PNG/JPEG"——**格式怎么选**？两个按钮、一个面板里选类型、还是按扩展名推断？ | 否 | **默认：一颗「保存」，面板同时允许 png/jpg/jpeg 两种类型，按用户选的扩展名决定编码器，没写扩展名补 `.png`**（与 P6 那条 `ALLOWED_TYPES` 一脉相承）。JPEG 那条要说清一句副作用：**JPEG 编码会丢掉 alpha**（REQ-002 的"导出保留 alpha"只对 PNG 成立），界面在带 alpha 的源图上选 JPEG 时给一句提示。备选：两颗按钮（保存 PNG / 保存 JPEG），多一颗控件但少一次推断 |
| Q-006 | 三阶段的"可取消"里，**翻译段停下来后已经翻好的译文留不留**？FR-2.9 只说"不残留半成品"，没说译文算不算半成品 | 否 | **默认留**（用户已经拿到的东西不当众销毁：H 区那几行译文还在，可以接着翻剩下的、也可以直接改字），并明确"这一次没有成品图"。`docs/06` TC-2.9-b 那句"及时停止、不生成结果文件、可立即重开"三条都满足。备选：全部丢弃（更贴"不残留半成品"的字面，但会销毁用户没要求销毁的东西）——你选丢弃我就把 REQ-009 那句反过来写 |
| Q-007 | 倾斜与竖排块（docs/04 §4 第 7 条：有 `polygon` 且主轴角 >5° 时逆仿射拉正 → 画 → 正仿射贴回）本包**做不做**？ | 否 | **默认不做**，统一走覆盖层并把 L-5 更新为"P7 落地形态 = 覆盖层"（docs/04 §4 第 7 条自己写了退路："P7 若来不及，明确记录为已知限制并统一走覆盖层"）。理由：`TextBlock.polygon` 至今是 `Option` 且 CHG-005 允许只填 `bbox`（REQ-003 边界原文），拿不到可信的倾斜角；仿射回贴还要处理采样带与擦除在旋转下的定义，是一条独立的功能。备选：本包顺带做——那 REQ-003/004/005 都要加"旋转坐标系下"的第二套判定，工时 +4h 起 |
| Q-008 | 真实样图与四项人工评分的原料**仍然缺**（`assets/samples/` 自 2026-10-28 起为空，CHG-005 的 Q-003 未解）。TC-2.7-a 要的 IMG-13（窄块 + 德语长译文）本来就没有，也没人标注 | 否 | **待你决定**，与 P6 同一条口径：**(a)** 你给 3~10 张真实截图进 `assets/samples/`（含一张渐变按钮/照片背景那类，覆盖 IMG-7/8），我出"逐张跑完整链 + 落 CSV（只有文件名/块数/计数/耗时）"的工具，成品**不入库**，打分你或我按表做；**(b)** 本包只出自绘两层指标，四项人工评分记**未闭环**推到 P8；**(c)** 折中：我自造 IMG-13（构造样图，专打 FR-2.7 那条降级链）+ 你随手给 1~2 张真图先跑出观察值。**默认 (c) 起步并邀请 (a)**；无论哪条，四个数（4.0 / 3.5 / 3.0 / 0.9）与 0.92 都一个字不动 |

## 允许动作与停止条件

**批准即允许**：本地实现与验证；在 `ice-vision` 内新建 `bg.rs`（背景/前景估计）、`typeset.rs`（版式计划）、`paint.rs`（字形光栅化与绘制），并实现 `composite.rs` 的回填与覆盖层两条链；**修改 `CompositeOutput`/`CompositeOptions`/`Renderer::compose` 的公开形状**（Q-001，全仓零引用已证）；把 `ab_glyph_rasterizer` 与 `owned_ttf_parser` 从 `[dev-dependencies]` 提升为 `ice-vision` 的正常依赖（不新增 crate 名，`Cargo.lock` 已有）并在提升后重跑一次零网络依赖图取证；在 `ice-core` 收一处字体路径/face 常量（Q-003）；给 `ice-app` 新增 `Job::ImageTranslate`/`Job::ImageCompose`、`Effects` 的四颗意图（翻译全部/生成图片/保存/复制图片）、`system::panel::request_save`（复用 P6 的面板协议）、`system::clipboard::write_image`；把 `views/image.rs` 的 G 区占位与 E 区那一排换成真控件、F 区加叠加框与点击选中，并替换以"G 区是空的一格""没有可点控件"为前提的那几条测试；新建 `examples/` 下的自绘合成度量与 IMG-13 构造小工具；跑 `nettop -p` 与 `cargo tree` 取证；往 `assets/samples/` **新增**文件（只读用户放进去的截图，不改不删它们的原件）；按 REQ-010 回写 `docs/04` §3/§4/§5/§6（调参结论、L-1/L-4/L-5 现状、评分分层）、`docs/06` §4（TC-2.3-a/2.3-b/2.4~2.9 逐条补结论 + IMG-13 + W-4.1~W-4.6）、`docs/02`（`ice-vision` 模块表补 `bg/typeset/paint`）、`docs/03`（E 区第二排、I-02/I-04/I-05/I-06/I-08 由 P7 转实）、`docs/05`（字体常量单点 + 测试计数 + "复制图片覆盖文本"这类系统行为口径）、`docs/traceability.md` 的 FR-2.4/2.5/2.6/2.7/2.8/2.9 六行、`docs/07-acceptance.md` 的 P7 段与变更登记；阶段验收后本地 commit（推送按队列第 01 条你的裁定，今晚默认**不 push**）。

**不允许**：未经登记改需求基线（含把 0.55 / 0.15 / 0.35 / 4.5:1 / ±8 通道 / ≥4.0 / ≥3.5 / ≥3.0 / ≥0.9 / 0.92 这些数自己调松）；引入任何带网络能力的依赖或学习式修补模型（ADR-003）；把图片字节、识别文本、回填译文写进日志、证据、提交或任何第三方（日志只允许块数、计数、耗时、尺寸、错误码、缺字形计数；CSV 只允许文件名与计数）；把成品默认落盘到仓库目录或 `features/*/evidence/`（导出只写用户显式选中的路径）；在用户没要求时备份/还原剪贴板文本；用 `runModal` 卡帧；伪造 `./sdd record` 的运行时或人工评分证据；为了过门禁修改已记录的证据件；未经确认执行 `rm`；改动 `assets/samples/` 里用户文件的原件；在拿到真实样图之前宣布四项评分或 0.92 达标；把 `CompositeOptions` 的四个数或任何导出默认写进 `config.toml`（本包不新增配置字段）；新增 `#[ignore]` 测试（本包运行时取证继续走 `examples/`，与 P6 同一条纪律）。

**停止条件**（触发即记 `SPEC_DEVIATION` 退回评审，不自行改需求或降级 Must）：

1. **擦除在真实截图上普遍留痕**（本该是 `Gradient` 的那一类也补不匀，补丁感蔓延）：可自选的是"调三档阈值（数值）"；如果必须改成"只有 `Uniform` 才回填，其余一律覆盖层"，那是改 ADR-003 的取舍口径，回来对齐。
2. **版式容量模型算不准**（0.75×框高起步 + 0.95 步进 + 55% 下限这一套在真实窄块上频繁溢出或过度缩小）：说明 docs/04 §4 的行距/度量形状本身要改（不只是调数），属设计变更。
3. **系统 CJK 字体这条路拿不到**（`STHeiti Medium.ttc` 不在、face 1 索引变、或许可不允许再分发）：回填整条链画不出字。备选"内置一份开源 CJK 字体"直接违背 NFR-6 的体积前提，要重开 ADR，得你决定。
4. **逐块翻译在真实块数下慢到不可用**（Q-002 选串行、而 15~20 块要等 >30s）：换并行属用户可感知的限流风险与额度消耗，回来按 Q-002 裁定，不自己改。
5. **`NSSavePanel` 的异步协议与帧循环接缝拿不到**（回调不给 URL、沙盒拒写、completion handler 不在主线程）：退回"手输路径保存"改动的是 FR-2.6 那条 Must 的"用户选择路径"，需你批准。
6. **`arboard::set_image` 写进去的图在别处打不开或粘出来尺寸错**（TIFF 表示、位序、alpha 预乘一类）：改成写 PNG 表示或"只做保存、不做复制"——后者改动的是 FR-2.6 的 Must。
7. **三阶段与既有 `RecognitionStage` 冲突到必须重写状态机**（而不是扩两段）：说明我对 P6 那套五态 + 独立序号的复用判断不成立，回来对齐设计。
8. **走查发现图片页在最小窗口（640×420）下版式塌了**（E 区第二排溢出、G 区与 H 区互相压边）：与 P5 的 REQ-012/REQ-013、CHG-005 的停止条件 7 同类，改版式属设计。
9. **长期拿不到真实样图**：本包可以带着"只有自绘度量"的结论收尾，但四项人工评分 SHALL 记为**未闭环**而不是 ✅；这条不算缺陷，算验收口径，写在 `docs/07` 的 P7 段。

## 时间估算

- 开发 12~16 小时。分块：REQ-002 背景估计约 1.5 小时（直方图众数 + 三档判定 + 贴边/alpha，纯函数，测试量大过实现）；REQ-005 字形光栅化与绘制约 2.5~3.5 小时（不确定性最高：把 dev 路提升成生产路、face/度量、缺字形如实报告、行盒落位，停止条件 3 落在这里）；REQ-004 版式计划约 2 小时（步进缩字号、两条断行规则、标点禁则、对齐推断、degraded 改判）；REQ-003 擦除约 1.5 小时（三档 + 先擦后画次序 + 输入不可变那条断言）；REQ-001 逐块翻译约 1.5 小时（新 job 形状、单块 Err 隔离、重试一块）；REQ-006 覆盖层约 1 小时（对比度公式 + 三档单选）；REQ-007 G/F 区约 1.5 小时（控件、前置条件矩阵、纹理换代）；REQ-008 导出约 1.5 小时（`request_save` 复用面板协议 + `write_image`，两处都可能撞停止条件 5/6）；REQ-009 三阶段约 1.5 小时（扩状态机 + 三段独立过期 + 块边界停）。预计新增 **90~140 项测试、约 2,000~2,800 行**（含 `views/image.rs` 那半页重写与两个 example）。
- 文档回写约 2 小时（`docs/04` §3/§4/§5/§6、`docs/06` §4 的 6 行结论 + IMG-13 + W-4.1~W-4.6、`docs/02` 模块表、`docs/03` I-02/04/05/06/08 转实、`docs/05` 单点常量与新计数、`traceability` 六行、`07-acceptance` P7 段与变更登记）。
- 验证 2~3 小时：`cargo test --workspace`（525 项零回退是硬判据）+ `clippy -D warnings` + `fmt --check` + `./sdd check` 一轮；自绘度量一轮；提升依赖后 `cargo tree -e normal` + `nettop -p` 取证一次；真机走查一轮（需你配合约 40~60 分钟：改一处错字看生成图反映、三档模式各生成一次、保存与复制两条出路各走一遍、深色截图看回填色、渐变按钮看覆盖层兜底、翻译中途停止）。
- 外部等待：你的 spec 批准一轮（本包一条 `./sdd approve`）、真实样图提供（Q-008，可能多轮）、四项人工评分（`docs/evidence/p7-scoring.md`）、走查一到两轮、验收签名一轮——不计入开发工时。
- 依据：CHG-005 的 8 个 REQ 实测产出 55 项测试 / 约 4,250 行（含注释与 example），比 spec 预估 90~130 项少，原因是一条判据压多条用例的写法；本包的不确定性比 P6 更集中在**两处**（字形绘制那条从没当生产用过的路、以及"擦得干净"这件事无头测不到），所以工时上限给到 16 小时，并把最贵的两项（REQ-005、REQ-004）排在前面做。
