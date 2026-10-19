# 阶段验收记录

每阶段一条记录，字段固定：**目标 / 验收方法 / 证据 / 结论 / 遗留**。
结论只有三种：`通过`、`带已知限制通过`、`不通过（回退到本阶段返工）`。

---

## P0 立项与需求 — 2026-10-19

| 项 | 内容 |
|---|---|
| 目标 | 冻结 v1.0 需求基线，明确不做项，定义八个阶段的验收门槛 |
| 验收方法 | 按 `01-requirements.md` 第 6 节自检清单逐条核对：编号唯一性、Must 项可测性、无不可测表述、范围自洽 |
| 证据 | `docs/00-charter.md`、`docs/01-requirements.md`；提交 `docs(P0)` |
| 结论 | **通过** |
| 遗留 | 需求基线锁定；后续变更须改需求文档并在本文件登记原因。机翻端点选择尚未实测（转 P3 的 ADR-004；P3 已实测并落档） |

## P1 工程骨架 — 2026-10-19

| 项 | 内容 |
|---|---|
| 目标 | 工具链就绪；四 crate 分层骨架可编译；空窗口能启动并验证置顶与模式切换 API；开发文档可用 |
| 验收方法 | 命令逐条原样执行 + 真机窗口实测 + 几何日志（`docs/evidence/P1-03-window-geometry.txt`）+ 无头断言 ViewportCommand 序列。**截图不入库**：本机 `screencapture -R 区域` 失效，只能全屏采集，画面必然带用户桌面其他内容，出于隐私删除（见事实清单最后两条） |
| 结论 | **通过** |

### P1 验收项

| # | 检查 | 方法 | 结果 |
|---|---|---|---|
| 1 | Rust 工具链版本满足 edition 2024 | `rustc --version` | ✅ 1.99.1（≥1.85） |
| 2 | 四 crate 依赖方向正确 | `grep ^ice- crates/*/Cargo.toml` + `cargo tree -p ice-core -e normal` | ✅ app→{core,providers,vision}，providers→core，vision→core，core 无内部依赖；core 依赖树中无 egui/eframe/reqwest/image |
| 3 | 全量类型检查通过 | `cargo check --workspace` | ✅ exit 0，0 warning |
| 4 | 单元测试通过 | `cargo test --workspace` | ✅ 53 passed / 0 failed（ice-core 40 · ice-providers 10 · ice-app 3 · ice-vision 0，视觉层是 P6/P7 的桩） |
| 5 | Lint 无警告 | `cargo clippy --workspace --all-targets -- -D warnings` | ✅ exit 0（修掉 5 处：2 个可 derive 的 Default、2 个 field_reassign_with_default、1 个 assert_eq 与字面量 bool 比较） |
| 6 | 窗口能启动并渲染 | 真机启动 + 全屏截图目测（截图不入库） | ✅ 标题栏、heading、两个按钮、状态行均按 `03-ui-design.md` 的位置渲染；几何日志 `P1-03` 记录到实际 inner/outer 尺寸与缩放 |
| 7 | 置顶 API 生效 | 简洁窗（置顶开）与用户其他应用同时运行 | ✅ 用户实测反馈"置顶简洁窗口现在无法拖动有点影响我干活"——即该无边框窗覆盖在其正在使用的应用之上且可点击；对照：普通级全量窗被同一应用遮挡。命令层面由 `topmost_maps_to_always_on_top_window_level` 断言 |
| 8 | 模式切换改变窗口形态 | 几何日志 `P1-03` + 无头单测 `ice-app::tests` | ✅ 简洁态 inner==outer==460×96（无标题栏）；全量态 outer 比 inner 高 32pt（标题栏回来）。点击路径本身未做自动化（本机无法合成鼠标事件），但按钮回调调用的就是被测的 `apply_mode`/`apply_topmost`，其命令序列已断言 |
| 9 | 配置读写与损坏回退 | `config` 模块单测 12 项（含 `corrupt_config_is_rejected_not_panic`、`load_at_falls_back_to_defaults_with_reason`）+ 真机写入畸形 config.toml 启动 | ✅ 畸形配置：WARN 记录原因 → 默认配置启动（几何日志显示回到默认 1080×720 全量窗）→ 界面顶部橙色告警条渲染可见 |
| 10 | 开发文档命令可原样执行 | 逐条复制执行 `05-dev-guide.md` §1/§3/§4 | ✅ 全部 exit 0；`-- --ignored` 当前匹配 0 条（联网测试在 P3 落地）；`cargo fmt --all -- --check` 与 `--check` 两种写法等价可用 |
| 11 | 发布构建可用 | `cargo build --release -p ice-app` | ✅ exit 0，耗时 1m05s，产物 `target/release/ice-translator` 11 MB（strip 后） |

### P1 过程中确定/修正的事实

- rustup 首次安装出现"部分安装"状态，重新执行安装脚本自动恢复，最终 stable 1.99.1。
- `reqwest` 0.13 的 TLS feature 名已从 `rustls-tls` 改为 `rustls`；`objc2-vision` 无 `VNImageRequestHandler` feature（实际为 `VNRequestHandler`）。因此 OCR 与 AppKit 的 objc 绑定推迟到真正需要它们的 P5/P6 阶段再接，骨架不背未验证的依赖。
- 本机无法连通 `translate.googleapis.com`（连接失败），`api.mymemory.translated.net` 可用（实测 en→zh、zh→en 正常，ja→zh 命中翻译记忆给出劣质结果）。机翻通道据此选型，详见 ADR-004。
- eframe/egui 0.37 与常见教程差异（已按源码修正，不是猜测）：`App::update` → `App::ui(&mut Ui, &mut Frame)`；`on_exit` 不再带 glow 参数（默认渲染器为 wgpu）；`TopBottomPanel` → `Panel::top/bottom(id)`；`ViewportCommand::WithDecorations` → `Decorations`；`Position` → `OuterPosition`；`WindowLevel(Some(..))` → `WindowLevel(..)`；`AppCreator` 需返回 `Ok(Box<dyn App>)`。
- 启动形态此前被忽略：骨架最初无论配置里存的是什么模式都以全量尺寸开窗，已改为按 `config.ui.mode` 决定初始尺寸/边框/窗级（避免"先开大窗再缩"的闪烁）。
- 本会话的 macOS 权限现状：`screencapture` 需要屏幕录制授权（用户重新授权后可用）；`screencapture -R x,y,w,h` 在本机一律返回 "could not create image from rect"，只能全屏采集后用 `sips --cropOffset` 裁剪；AppleScript 取窗口信息超时，因此**无法合成鼠标点击**，UI 交互类走查在 P4/P5 需要用户配合或改用无头断言。
- **全屏截图不能作为入库证据**：唯一可用的采集方式是整屏，画面必然包含用户 IDE 中的项目代码与其他会话窗口。P1 已拍的两张（旧 `P1-01`/`P1-02`）在推送前删除，视觉结论改由几何日志 + 无头命令断言 + 用户实测反馈支撑。P4/P5 需要图像证据时改用"仅含本应用窗口区域"的裁剪，并在提交前逐张检查。
- **屏幕锁定期间截到的是锁屏壁纸**：`caffeinate -u` 只能唤醒显示、不能解锁会话，此时 `screencapture` 返回不含任何应用窗口的壁纸图（表现为文件正常、内容全黑或全壁纸）。补拍视觉证据必须在用户解锁屏幕后进行。
- **SIGTERM 不触发 `on_exit`**：实测 `kill -TERM` 后 config.toml 未被改写，即异常退出/强杀不会保存窗口几何与模式。正常退出（Cmd+Q、关窗）走 eframe 的 `on_exit` 会保存。→ 记为 P5 待办：**几何回写时机**（拖动/缩放结束与形态切换时落盘），否则简洁窗位置在崩溃后丢失。
- 远端推送凭据此前缺失，2026-10-19 用户已配置完成，P1 起恢复"每阶段验收后 commit + push"。


---

## P2 架构与设计 — 2026-10-19

| 项 | 内容 |
|---|---|
| 目标 | 架构/UI/图像流水线设计成文；每条需求映射到具体模块，无孤儿需求；关键取舍有 ADR |
| 验收方法 | ① 用脚本把 `01-requirements.md` 的需求编号集合与 `traceability.md` 的行集合做双向差集；② 检查矩阵中每条 Must 的"实现"列非空；③ 逐 crate 把 `pub mod` 清单与 `02-architecture.md` §2 的模块表对照；④ 核对 §8 自检 7 项；⑤ ADR 是否覆盖 GUI/OCR/擦除/密钥/分层五项取舍 |
| 证据 | 差集结果 44 ↔ 44 双向为空（无孤儿需求，也无凭空多出的行）；Must 行"实现"列 0 空；`02-architecture.md` §8；`docs/adr/ADR-001/002/003/005/006` |
| 结论 | **通过（签发）** |
| 遗留 | ADR-004（机翻端点选型）依计划待 P3 实测后补（P3 已落档，见下）；`ice-vision` 四个模块与 `ice-app` 六个模块按 §8 的落地表在 P4~P7 填充 |

### P2 过程中确定/修正的事实

- **模块表漏登记**：`ice-providers::request`（`TranslationRequest`/`TranslationResult`）在骨架里已存在并被 `dispatcher` 使用，但 §2 的表没有这一行。已补登记。这是逐 `pub mod` 对照才暴露的偏差，说明"设计文档 vs 实际代码"必须机器核对，不能凭印象。
- **需求统计此前是错的**：矩阵与需求文档写的 Must 27 / Should 16 / Could 2，逐行重算实为 **Must 30 / Should 12 / Could 2**（共 44 条）。原因是 P0 手工计数即有误，FR-3.9 加入时又只把 26 改成 27 而没有重算。两处统计已按脚本结果更正。
- **矩阵漏一行 Could**：`NFR-8 可访问性`（字号缩放、键盘可达）在需求里有、矩阵里没有，已补，并挂到 P4 走查（`03-ui-design.md` §5 第 5 步的 Tab 顺序检查正是它的验证方法）。
- NFR-6（分层依赖）在矩阵里原记"待 P8"，实际 P1 已用 `cargo tree` 核过，改为"部分 ✅，P8 三目标复核"。
- ADR-001 的成立不再只是纸面判断：单窗口双形态所需的 `Decorations/InnerSize/OuterPosition/Resizable/WindowLevel` 组合在 macOS 真机按预期改变窗口形态（`evidence/P1-03-window-geometry.txt`），这是 P2 能签发的直接依据。

---

## P3 引擎层 — 2026-10-19

变更包：`features/CHG-002`（`CHANGE-ID@v0.1`，Spec SHA-256 `88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c`，用户批准）。

| 项 | 内容 |
|---|---|
| 目标 | 机翻与 AI 两条真实通道可用且行为可证；不可译内容逐字节保持；超长输入自动分段；错误可诊断且不泄源文与 Key；机翻端点选型落 ADR |
| 验收方法 | ① 本地假服务覆盖全部 HTTP 行为（端点、状态码、超时、畸形 JSON 都由测试注入）；② `cargo test -p ice-core -p ice-providers` 与 `cargo clippy --workspace --all-targets -- -D warnings`；③ 真实服务端点由 3 条 `#[ignore]` 用例实测，输出用 `./sdd record` 导入为证据；④ `./sdd check` 逐条登记验证；⑤ `tools/check_traceability.py` 保证编号集合仍一致；⑥ `./sdd scan` + `tools/check_staged.py` 做敏感信息门禁 |
| 证据 | `features/CHG-002/`（spec/design/tasks/approvals/evidence + `project/ledger` 里的证据件）；`docs/adr/ADR-004-machine-translate-endpoint.md`；测试计数 ice-core 45 · ice-providers 51 · ice-app 3（共 99 通过、3 项 ignore 需真实网络） |
| 结论 | **通过（签发）** — 7 项必需验证全部有证据：V-001/002/005/006/007 由 `./sdd check` 本地跑，V-003/V-004 由 `./sdd record` 导入真实服务端点的实测输出 |
| 遗留 | FR-1.7 的"源文仍在输入框"、FR-1.2 的回显、401 的"去设置"动作属界面行为，转 P4 走查；NFR-1 的 P50<800ms 目标本阶段只留下单次样本，判定转 P8 的 20 次采样脚本；`AiProvider` 五家预设里只有 DeepSeek 经本机关联调（其余四家本机 TLS 建连被重置，未验），P4 设置页若切换需再各跑一次 live |

### P3 验收项

| # | 检查 | 方法 | 结果 |
|---|---|---|---|
| 1 | 规格先于代码 | `./sdd status CHG-002` | ✅ APPROVED（批准哈希与当前 spec 一致；实施期只 `replan` 设计，未改 spec，故批准仍然有效） |
| 2 | 机翻通道契约 | `cargo test -p ice-providers machine` | ✅ 10 项：显式源语言不外传检测、自动检测走本地、判不出按 `en` 且不回显、450 字符切块后拼接与原文逐字符守恒、body 级错误不当成译文、空译文算失败、状态形态不认识算 `Decode`、超时带重试次数、连接失败说"不可达"而非"被拒"、被服务端改写数字后仍能还原 |
| 3 | AI 通道契约 | `cargo test -p ice-providers ai` | ✅ 14 项：请求体符合 OpenAI chat 形状（含 `authorization: bearer …`，未配置 temperature 时不发该键）、术语与目标语言进 system 提示词、分段带上一段尾巴、按序拼回并补换行、第 2 段失败不返回半截、401/429/500 映射不同且不重试非网络类、占位符丢失报失败、无 Key 时 `is_available()==false` |
| 4 | 内容保护 | `cargo test -p ice-providers protect` | ✅ 14 项（V-002） |
| 5 | 错误与隐私边界 | `cargo test -p ice-providers error` + `debug_output_hides_the_key`、`query_string_is_scrubbed_from_error_text`、`error_text_never_carries_the_source_text` | ✅ 错误文本剔除 URL 查询串；`Debug` 只打 `key_len`；`toml::to_string(&config)` 不含 `api_key` |
| 6 | 全量单测 | `cargo test --workspace` | ✅ 99 passed / 0 failed / 3 ignored（V-001） |
| 7 | Lint | `cargo clippy --workspace --all-targets -- -D warnings` | ✅ exit 0（V-007） |
| 8 | 机翻真实端点实测 | `cargo test -p ice-providers --test live -- --ignored --nocapture machine` | ✅（V-003 已用 `./sdd record` 导入，最终代码态重跑一次）`machine_live_short`："你好世界"，1 段；`machine_live_long_text_is_chunked`：1060 字符 → 3 段、译文 260 字符，且响应不含 `QUERY LENGTH`。**延迟抖动大**：两轮单句 731 / 2133ms，3 段 1.9 / 3.2s |
| 9 | AI 真实通道实测 | `cargo test -p ice-providers --test live -- --ignored --nocapture ai`（Key 从钥匙串读，不落文件） | ✅（V-004 已用 `./sdd record` 导入，最终代码态重跑一次）`ai_live_deepseek`：英文单句 → 中文译文（1 段，1142 / 1226ms）；混合句 `https://a.example.com` / `12 MB` / `getUserInfo` 经 protect→模型→restore 后三个片段**逐字节回来**。Key 全程未进入终端回显、证据件与仓库（两轮证据件里 `sk-` 出现 0 次）。本机对国内其他供应商 TLS 建连被重置，故 AI 的真实契约以 DeepSeek 为准（Q-001 的落地口径），其余四家预设未联调，已记入遗留 |
| 10 | 供应商可选 | `preset_provider_supplies_url_and_model`、`manual_values_beat_the_preset`、`custom_without_url_is_not_ready`、`every_preset_except_custom_has_an_address` | ✅ 百炼/DeepSeek/智谱/Kimi/方舟五家预设 + 自定义；手填优先、预设填空；地址已带 `/chat/completions` 时不重复拼接 |
| 11 | ADR-004 落档 | `grep -q "状态：已接受" docs/adr/ADR-004-*.md` | ✅（V-005）字段齐全，含实测到的 body 级错误、450 而非 500 的余量理由、以及三条失效/降级条件 |
| 12 | 需求编号一致 | `python3 -B tools/check_traceability.py` | ✅ 44 ↔ 44（V-006） |
| 13 | 追踪矩阵回填 | 人工核对 `traceability.md` 的 FR-1.2/1.3/1.6/1.7/1.9、NFR-1/2/3 行 | ✅ 实现列补上本包模块，结果列写清"引擎层已测 / 界面待 P4"的边界 |

### P3 过程中确定/修正的事实

- **切块按字符索引会切开多字节汉字**：`chunk_chars` 最初按字节偏移切片，实测 panic 于 "end byte index 1348 is not a char boundary … inside '。'"。修正为自然断点必须加上断点串自身的 UTF-8 长度（`natural_cut`），并由 `chunks_respect_char_limit_and_order` 固化。这是 NFR-2 在本阶段唯一真实踩到的一次崩溃。
- **占位符还原失败的处理与 P2 设计相反**：`02-architecture.md` §3.1 与 `TC-1.6-c` 原写"降级为不保护、返回原始译文并提示"。静默返回一段被改写的文本等于把破坏当成成功，CHG-002 spec 的 REQ-003 明确"无法还原时报失败"。代码按 spec 实现（`restore` 返回 `lost`，非空即 `Decode`），两处文档按 spec 更正并在此登记。
- **还原按占位符编号匹配而非按位置**：模型常调整语序，按位置回填会错位。乱序由 `reordered_placeholders_still_restore` 覆盖；源文本身就带 `[[0]]` 的情况在 `protect` 阶段先转义，避免与占位符命名空间相撞（`existing_bracket_placeholder_in_source_is_escaped`）。
- **免费机翻端点的错误写在响应体里，HTTP 仍是 200**：只看状态码会把 `"responseStatus":"403"` + `responseDetails` 当成成功；且该字段有时是数字有时是字符串。归一化为 `u16` 后比较，形态不认识一律失败（`unrecognized_status_shape_is_decode_failure`）。
- **错误文本可能带出源文**：`reqwest` 的错误串包含完整 URL，查询参数里就是用户原文。新增 `scrub_queries` 剔除 `?` 之后的部分，连接失败只报主机名。
- **标识符保护只认"看得出是标识符"的写法**（多段驼峰、下划线连接）。单个首字母大写的普通英文词（Kubernetes）不保护——它按规则也是驼峰，保护它会让正常译文里出现无法还原的原文。这是范围决定，不是遗漏，写在 `protect.rs` 的模块注释里并由 `ordinary_capitalized_words_are_not_protected` 锁定。
- **引擎不读钥匙串、不读磁盘**：`AiEngine` 只从构造参数拿 Key，钥匙串归 P4 的 `ice-app::system::Secrets`（ADR-005）。于是 `ice-providers` 生产依赖里没有 `keyring`，只有 `tests/live.rs` 用 dev-dependency 读一次。
- **reqwest 0.13 把 `RequestBuilder::query` 关在 `query` feature 后面**：编译报错后读 vendored 源码确认，根 `Cargo.toml` 补开该特性。
- **超时与分段上限改由配置驱动**：设计期写死的常量（8000ms / 3500 字符）改为读 `config.translation.{timeout_ms, ai_chunk_chars}`，机翻的单次查询上限 450 是端点硬约束，保留为常量。经 `replan` → `seal-plan` 重新封板，spec 未动。

---

## P4 全量界面 — 待记录
## P5 简洁悬浮模式 — 待记录
## P6 OCR — 待记录
## P7 译文回填 — 待记录
## P8 收口 — 待记录

## 变更登记

| 日期 | 变更 | 原因 |
|---|---|---|
| 2026-10-19 | 无 | 基线建立 |
| 2026-10-19 | P1 通过；`ice-app` 启动形态改为读取 `config.ui.mode`（原实现固定按全量尺寸开窗） | 骨架自测发现保存的模式不生效，属 FR-3.1 范围内的修正，不改需求 |
| 2026-10-19 | 新增 P5 待办：窗口几何与形态切换的**即时落盘**（不依赖 `on_exit`） | 实测 SIGTERM/强杀不触发 `on_exit`，异常退出会丢失简洁窗位置 |
| 2026-10-19 | 提交节奏恢复为"每阶段验收后 commit + push" | 用户已配置 GitHub 推送凭据 |
| 2026-10-19 | 证据规范变更：`docs/evidence/` 只允许"裁剪到仅含本应用窗口"的截图，P1 两张整屏截图删除；视觉结论改由几何日志 + 无头断言支撑 | 整屏截图含用户 IDE 代码与其他会话窗口，不应进入远端仓库。规则写入 `03-ui-design.md` §5 |
| 2026-10-19 | P2 签发；需求统计更正为 Must 30 / Should 12 / Could 2（共 44 条），矩阵补登记 `ice-providers::request` 模块与 `NFR-8` 行 | 逐行脚本重算发现 P0 的手工统计有误、矩阵漏一行 Could；模块表漏登记已存在的 `request` 模块。均为文档与代码对齐修正，不改需求内容 |
| 2026-10-19 | 需求基线变更 3 项：FR-3.2 增补"无边框不得导致窗口不可移动"；新增 **FR-3.9 形态切换形变动画**（M）；NFR-4 由"提供暗色主题"改为"**默认跟随系统深浅色 + 三态开关**" | 用户实测 P1 骨架窗口后反馈：置顶简洁窗拖不动影响干活、切换太硬、要求配色跟随系统。三项均已确认 egui/eframe 0.37 有对应能力（`StartDrag`、逐帧 `InnerSize`、`ThemePreference::System`），落在 P4/P5 实现 |
| 2026-10-19 | 引入 SDD 工作流引擎（P2.5，变更包 `CHG-001`）：根目录新增 `sdd`/`tools/`/`templates/`/`skills/`/`features/`/`project/`/`codebase/`/`tests/`，P3 起的阶段变更走 `features/CHG-0NN/` 的批准与证据门禁；`docs/00~07` 基线继续作为需求事实源 | 用户要求把改动纳入结构化记录。来源为其本地工作区的通用模板层，公司项目内容一律不取；本仓库公开，提交前跑 `./sdd scan` 与 `tools/check_staged.py` 两道门禁 |
| 2026-10-19 | `CHG-001` 实施中修实一处引擎缺陷：同仓布局下 `check` 写入的账本改变被测树哈希，证据写完即过期，`transition VERIFIED` 被阻塞；`fingerprint` 增加账本路径排除，并补 4 项同仓回归（引擎测试 32 → 36） | 真跑才暴露的问题：来源工作区规格与代码分仓，本仓库同仓后语义不成立。属实现选择变化、不改已批准业务行为，按流程走 `replan` 重新封存计划，不重开规格评审；`docs/verification.md` 记为与来源的第 4 处差异 |
| 2026-10-19 | **`TC-1.6-c` 预期更正**：由"占位符被改写→退回不保护、返回原始译文并提示"改为"还原失败→报 `ProviderError::Decode`，不返回任何文本"；`02-architecture.md` §3.1 同步改写 | CHG-002 spec 的 REQ-003 已批准为"无法还原时报失败"。旧写法会静默返回被改写的译文，与 FR-1.6"不得被篡改"的验收意图相反。需求本身（FR-1.6）未改 |
| 2026-10-19 | 联网测试命名规范由 `net_` 前缀改为 `tests/live.rs` + `<通道>_live_<场景>`；`05-dev-guide.md` §4/§6 补真实端点命令与钥匙串配置一行；测试计数 53 → 99（+3 项 `--ignored`） | 一条通道往往要验多个场景（短文本/超长切块/真实模型），通道名在前才能用一条过滤命令选中整组。属文档规范对齐实现，不改需求 |
| 2026-10-19 | ADR-004 落档；`traceability.md` 的 FR-1.2/1.3/1.6/1.7/1.9 与 NFR-1/2/3 回填实现列与结果列；`02-architecture.md` §2 模块表补 `error` 行、更正 `machine`/`ai`/`protect` 的关键类型与 §8 落地表 | P3 实测完成，"待 P3"的三处引用按 CHG-002 REQ-007 回写。⚠ NFR-1 的机翻 P50<800ms 本阶段只有单次样本（731–1422ms），**未确认达标**，判定推给 P8 的 20 次采样脚本 |
| 2026-10-19 | `CHG-002` 走 `replan` 重新封存计划（spec 哈希不变，用户批准仍然有效）：超时与 AI 分段上限改读 `config.translation.{timeout_ms, ai_chunk_chars}`，设计期两个切块函数合并为 `chunk_chars` + `choose_cut`，`restore` 返回 `Restored{lost}`，`AiConfig` 取值规则改名 `effective_*`，机翻引擎新增错误文本剔除查询串 | 都是实施中浮现的实现选择变化（可配置性、避免重复实现、错误语义要能表达"丢了哪个占位符"、源文不得进错误文本），未改动已批准的业务行为，按流程不重开规格评审 |
