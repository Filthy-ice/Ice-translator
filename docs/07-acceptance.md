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

## P4 全量界面 — 2026-10-20 实现完成，2026-10-21 验收通过

变更包：`features/CHG-003`（`CHG-003@v0.1`，Spec SHA-256 `84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7`，用户 2026-10-20 批准，含 Q-001/Q-002/Q-003 三项决定）。

| 项 | 内容 |
|---|---|
| 目标 | 全量界面可用：四页签 + 工具条 + 状态条；翻译闭环含加载态与 `request_id` 竞态丢弃；译文两个复制按钮含空译文保护；错误可读且带动作按钮；设置页持久化与密钥边界；历史页与术语分区；快捷键与关窗常驻；单实例互斥 |
| 验收方法 | ① 界面层用无头 `ctx.run_ui` + `Effects` 队列断言，引擎用 `StubEngine`，**不碰网络/剪贴板/钥匙串**；② `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`；③ `tools/check_traceability.py` 保证编号集合仍一致；④ `./sdd scan` + `tools/check_staged.py` 敏感信息门禁；⑤ 真机走查 W-1~W-6 与 `./sdd record` 导入运行时证据 |
| 代码态结果 | **全绿**：`cargo test --workspace` 246 passed / 0 failed / 3 ignored（ice-app 144 · ice-core 49 · ice-providers 53）；clippy exit 0；fmt 无差异。20k 字符单帧布局实测 **15.6ms**（预算 100ms，CHG-003 的 HYPOTHESIS 消解） |
| 结论 | **通过（签发）** — 12 项验证全部有证据：V-001~V-008 由 `./sdd check` 在最终代码态重跑（246 passed / 0 failed / 3 ignored，clippy exit 0，fmt 无差异，追踪 44↔44，`./sdd scan` 通过，`sk-` 形态 grep 0 命中）；V-009 四轮真人走查（23/24 项通过，5 个缺陷 D-1~D-5 全部修复并复验）；V-010 五个真机子项全齐；V-011 两条真实通道；V-012 钥匙串三件套（写入/清除为用户口述 + 指纹变化印证，读取有两份直接日志）。**唯一的证据缺口**是 W-1.8 的"红字 + 去设置按钮"视觉同框（假 Key 会覆盖用户真 Key，用户选择不走），已由 429 真机走通同一渲染路径 + 两条无头测试覆盖动作链，转 P8 用废 Key 补 |
| 遗留 | ① **简洁窗四条转 CHG-004**（P4 第四轮用户预报）：切过去尺寸没变小（→ W-2.1）、不能拖动（→ W-2.13）、窗内不能切机翻/AI（→ 新增 W-2.15）、窗内译文不能滚动（→ 新增 W-2.16）；CHG-003 规格第 19/25 行已把"简洁窗内部版式、吸附、半透明、剪贴板即翻、形变动画"划给 P5，本包只保证切换按钮/快捷键存在与切换后内容不丢。② **自动检测撞到目标语言的"自动反向"转 CHG-004**（用户 2026-10-21 决定）：现状是本地拦下给中文可读错误 `SameLanguage`，用户要的是目标语言自动翻转（英语 ↔ 简体中文）并持久写回、提示条说明、本次直接出译文；这是对 FR-1.2/FR-1.4 行为的改动，已登记为 TC-1.2-c，本包不改。③ **W-1.8 假 Key 分支**未在真机复现，转 P8。④ AI 五家预设里只有 DeepSeek 经本机联调（其余四家本机 TLS 建连被重置），设置页切换后需各跑一次 live。⑤ NFR-1 的 P50<800ms 只有单次样本，判定转 P8 的 20 次采样脚本 |

### P4 验收项

| # | 检查 | 方法 | 结果 |
|---|---|---|---|
| 1 | 规格先于代码 | `./sdd status CHG-003` | ✅ APPROVED，批准哈希与当前 spec 一致；实施期的偏差全部登记在下方 §P4 偏差清单，未改任何 REQ |
| 2 | 全量单测 | `cargo test --workspace` | ✅ 246 passed / 0 failed / 3 ignored（V-001，第四轮走查后重跑，日志 `/tmp/ice-test-full.log`） |
| 3 | 领域层回归 | `cargo test -p ice-core` | ✅ 49 项（V-002）：术语入 `config.toml` 的 TOML 往返、旧 `glossary.toml` 导入的四种输入、历史裁剪与搜索 |
| 4 | 界面层无头断言 | `cargo test -p ice-app` | ✅ 144 项（V-003）：页签状态保留、竞态丢弃、错误映射与动作按钮、快捷键分类、`ViewportCommand` 编排、PID 锁与陈旧锁接管、20k 字符布局预算；D-4 后新增两栏滚动容器的实测断言（内容高于视口、视口不超栏高、两栏滚轮偏移真的增长） |
| 5 | Lint | `cargo clippy --workspace --all-targets -- -D warnings` | ✅ exit 0（V-004） |
| 6 | 格式 | `cargo fmt --all -- --check` | ✅ 无差异（V-005） |
| 7 | 需求编号一致 | `python3 -B tools/check_traceability.py` | ✅ 44 ↔ 44（V-006），文档回写后重跑仍成立 |
| 8 | 敏感信息门禁 | `./sdd scan` + `python3 -B tools/check_staged.py` + `grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates` | ✅ `./sdd scan` 通过、Key 形态 grep 0 命中（V-007/V-008）；`check_staged.py` 在 `git add` 之后、提交之前再跑一次 |
| 9 | 真人走查 W-1~W-6 | 用户按 `06-test-cases.md` 走查表逐项验证并截图（裁剪到仅含本应用窗口） | ✅ **四轮走查完成**（V-009，`BUSINESS_CONFIRMED`，证据件 `*-V-009.txt` 含走查表快照）：W-1.1~W-1.7、W-1.9~W-1.24 共 23 项通过（第四轮用户原话"都可以了"）。四轮共跑出 5 个缺陷并全部修复+复验：D-1 动作行被裁出面板、D-2 Dock 恢复回来一张拉伸贴图（修了三轮）、D-3 两栏底边不齐、D-4 18k 粘贴把源文框撑出窗口且滚不动、D-5 装饰字形显示为豆腐块。**W-1.8 的假 Key 分支未真机复现**（会覆盖用户真 Key，用户选择不走）：错误态渲染路径已由真机 429 走通（状态条转红 + 可读中文原因），`Unauthorized`→「去设置」→切设置页并聚焦 Key 输入框由 `failure_keeps_source_and_maps_action`、`failure_action_go_settings_switches_tab_and_focuses_the_key_field` 覆盖，缺口只有"红字 + 按钮"视觉同框，转 P8 |
| 10 | 真机运行时证据 | 计划里这一项含五个子项：18k 字符粘贴手感、单帧耗时、Cmd+W 隐藏后 30s CPU 取样、Dock 恢复、双开被拒且 config 不变 | ✅ **五个子项全齐**（V-010，证据件 `*-V-010.txt`，首采于 2026-10-20、其后覆盖补齐）：① 窗口 `09:24:49` 建好，钥匙串在后台线程 `09:25:07` 才返回（用户在系统授权框上花了 18s），期间窗口持续渲染；② 双开时第二个实例退出码 **1**、中文提示含持锁者 PID、`config.toml` 与 `history.jsonl` 的 mtime+size 前后一致、锁文件未被第二个实例删掉；③ 空闲 **30.2s 消耗 CPU 恰好 0.000s（0.0000%）**，RSS 93.3MB；④ **18,000 字符粘贴**（2026-10-21 第四轮）：D-4 修复后的构建上机翻 6 段 11.85s 完整出结果（`chars=9181` 为译文长度），左框不变形不贴底、边框完整、能滚到末尾、字数行 18,000，译文栏同样可滚；修复前另有 71,211 字符 / 6 段 / 53.4s 的成功记录；同轮 AI 通道被供应商限流（429×2、Timeout×1），界面给可读中文原因、进程不崩、源文不丢，失败日志只记 `error`+`status`；单帧布局 15.6ms（TC-2.1-c）；⑤ **Cmd+W 隐藏后点 Dock 恢复**（2026-10-21 第三轮）：用户原话"可以了，收起叫回后依然能够使用"，日志取证收起后 248ms 进入 `revealed=false focused=false`、2084ms 两者同时回来并触发"窗口恢复可见，重发几何"——`CancelClose`+`Minimized` 这条偏差真机成立，FR-5.2 成立 |
| 11 | 真实端点两条 | 机翻 MyMemory（无需 Key）；AI 用用户钥匙串里的 Key | ✅（V-011，证据件 `*-V-011.txt`）：**由用户在真实界面上操作**，机翻 3 次 695–1512ms、AI DeepSeek 3 次 708–988ms，其中一次 `cache_hit=true elapsed_ms=0`（FR-1.8 真机成立）；引擎来回切换均出译文（FR-1.3）。同语言对由新增的 `SameLanguage` 本地拦下（`status=0`，未发请求） |
| 12 | 钥匙串三件套 | 写入 / 读取 / 清除，敏感值只留长度与哈希前缀 | ✅（V-012，证据件 `*-V-012.txt`）：**读取**有两份直接日志（`key_len=35 key_hash8="d5477b63"` 与 `"4b507e46"`，含一次授权框阻塞 18s 的情形）；**写入与清除**由用户在界面上自行完成（2026-10-21，原话"钥匙串我已经试过了"），证据等级为**用户口述 + 指纹变化间接印证**——两次启动指纹不同只能来自条目被重写（指纹是 SipHash 摘要前 8 位，单测断言不含 Key 任何片段）。没有当刻日志行的原因已写明：那次实例用 `RUST_LOG=ice_app=debug` 启动而 crate 名是 `ice_translator`，日志被全过滤；清除会真删用户凭据，代理不得代跑。边界四项核对通过：`Config` 结构无 Key 字段、`env::var` 取 Key 0 处、设置页只显示"已配置（35 位，存在系统钥匙串）"连指纹都不显示、`describe_error` 对 `BadEncoding`/`BadDataFormat` 不转 Display |

### P4 证据清单（已全部取到，2026-10-21 收口）

证据件都在 `features/CHG-003/evidence/`，命名 `NNNN-V-0NN.txt`，**每次 `./sdd check` / `./sdd record` 都追加一个新编号**，
所以本文只写验证号、不写死编号：取同号里编号最大的那份即最后一次覆盖。写死编号会立刻过期——`./sdd` 的"证据过期"判定
比的是整棵树的哈希（`tools/sdd.py:181-188` 只排除 `features/`、`.sdd/` 等账本路径，`docs/` 在树里），
改一个数字就会让全部证据失效、陷入"改文档→证据过期→重记→编号变→再改文档"的循环。

| 验证 | 方式 | 证据件 |
|---|---|---|
| V-001~V-008 | `./sdd check CHG-003 V-00N` 在最终代码态重跑（全量单测 246 / ice-core 49 / ice-app 144 / clippy / fmt / 追踪 44↔44 / `./sdd scan` / 暂存区门禁） | `*-V-001.txt` ~ `*-V-008.txt` |
| V-009 真人走查 | 用户本人四轮操作 + 截图 + 口述，代理不参与判定 | `*-V-009.txt`（含 `06-test-cases.md` §2 走查表快照） |
| V-010 真机运行时 | 五个子项：启动时序、双开、空闲 CPU、18k 粘贴、Dock 恢复 | `*-V-010.txt`（首采于 2026-10-20，其后覆盖补齐） |
| V-011 真实端点 | 机翻 MyMemory + AI DeepSeek，用户在真实界面上操作 | `*-V-011.txt` |
| V-012 钥匙串 | 读取有直接日志；写入/清除为用户口述 + 指纹变化印证 | `*-V-012.txt` |

**唯一的证据缺口**：W-1.8 的"红色原因 + 「去设置」按钮"视觉同框未在真机复现（填假 Key 会覆盖用户真 Key，用户选择不走）。
替代证据是同一路径上的真机 429（状态条转红 + 可读中文原因）与两条覆盖 `GoSettings` 动作链的无头测试，缺口转 P8 用一把废 Key 补。

截图规则仍然有效：裁剪到**仅含本应用窗口**（仓库公开，规则见 `03-ui-design.md` §5）。

### P4 过程中确定/修正的事实

- **macOS 钥匙串授权框会阻塞 `get_password()`，而建窗在它之后 → 表现为"启动卡死"**。真机日志：进程停在 `keyring_core: get password from entry …`，12s 后被 `timeout` 杀掉（exit 124），`config.toml` 一次都没写出来。根因是条目由 `/usr/bin/security add-generic-password` 写入、其 ACL 不信任未签名的本程序，于是系统弹授权框并同步等待。修法：读取挪到建窗**之后**的 `runtime.spawn_blocking`，结果经容量 1 的 `mpsc` 回投帧循环，并在那里 `ctx.request_repaint()`。修好后日志时序自证：`winit::create_window` 在 `08:58:56.638`（其后约 5400 行渲染活动），`get password` 在 `08:58:56.657` 才开始、且阻塞期间窗口照常渲染。
- **`mpsc::Sender::send` 是 async 的**，在 `spawn_blocking` 里 `let _ = tx.send(…)` 只会造出一个没人 await 的 future——探测结果**永远到不了**帧循环，且编译不报错。clippy 的 `let_underscore_future` 抓到了它，改用 `blocking_send`，并补 `the_probe_result_actually_arrives_over_the_channel` 作为回归锁。
- **`config.toml` 里的术语从未进过引擎**：`build_translator` 不装术语，`IceApp::new` 也没调 `runner.set_glossary`，于是要等用户在设置页改一次术语才生效（违背 FR-4.5 的"即时生效"）。更隐蔽的是 `replace_translator` 每次重建都会把术语一起丢掉。修法：启动即推一次，并把重建收进 `install_translator()`（换引擎后立刻重灌），由 `glossary_from_config_reaches_the_engine_on_the_first_job` 与 `rebuilding_the_translator_keeps_the_glossary` 双向锁住。
- **`capture_rect` 曾把标题栏高度算进窗口位置**：外框矩形还没到位时退回用内框原点，于是每启动一次窗口就往下挪一截。改为"位置取外框原点、尺寸取内框"，任一缺失（第一帧）就返回 `None`、不覆盖已保存的配置。
- **egui/eframe 0.37 没有 `ctx.style(closure)`**：设计文档里写的这个 API 不存在，真实入口是 `ctx.style_of(egui::Theme) -> Arc<Style>` 与 `ctx.all_styles_mut`。后者亮暗两套都改，所以断言任一套即可。同理设计写的 `ctx.set_theme_preference(...)` 实为 `ctx.set_theme(...)`。
- **`Visible(false)` 在 macOS 上叫不回来**：eframe 0.37 不投递 `Reopen`，隐藏后点 Dock 图标没有反应，FR-5.2 的"再次点击图标恢复窗口"落空。改用 `CancelClose` + `Minimized(true)`：进程不退、内容原样、Dock 里有图标可点。这是对封存设计的偏差，不改 REQ，见下方清单。
- **同语言对会被机翻端点硬拒，且错误是英文**：真机复现——中文源文 + 目标简体中文，"自动检测"解析出 `zh-CN|zh-CN`，端点回 HTTP 200 但体内 `responseStatus: 403`、`responseDetails: PLEASE SELECT TWO DISTINCT LANGUAGES`，界面直接显示这句英文。用 curl 逐一试出触发条件后，改为在 `machine.rs` 发请求**之前**就地判定，新增 `ProviderError::SameLanguage(lang)` 给中文文案。判定放在引擎层而不是 UI 层，是因为"发过去必然失败"，本地拦下结果完全一样、还省一次往返；AI 通道不加这道闸，LLM 对同语言输入不会失败。
- **失败日志只记变体名不足以排障**：上面那个 403 在日志里只有 `error="Status"`，看不出是 400 还是 503，只能靠 curl 反推。补 `status=` 字段（`jobs::status_code`）。状态码不是秘密：响应体已在 `from_status` 里截断到 200 字符，查询串已在 `from_reqwest` 里洗掉，源文与译文仍然一律不进日志（NFR-3）。补上后一次就定位到 403。
- **单实例锁的"认得自己 PID"分支无法在同进程内测**：`acquire` 会把记着自己 PID 的锁当成"PID 被回收过的陈旧锁"接管（这是正确行为，`kill -9` 后 PID 复用是真实场景）。原测试在同进程里写锁再取，前提本身不成立。改为 `spawn /bin/sleep 30` 拿一个活的外来 PID 做拒绝测试，另用一条测试锁住"自己的 PID 也算陈旧"的接管行为。真机也复现了接管：`kill` 掉旧实例后重启，陈旧锁被自动清除。
- **`HistoryStore::read_all()` 静默跳过损坏行**（P3 已封存的行为：`Err(e) => tracing::debug!(…)`），只有 I/O 级失败才返回 `Err`。原界面层测试按"一行坏 → 整页报错"写，前提与领域层相反。拆成两条诚实的测试：坏行跳过、整页仍可用；用目录冒充历史文件制造"能打开但读不出"，此时才给可读原因。两种情况下翻译都不受影响。
- **跨平台的 `time_t` 宽度差异**：`libc::time_t` 与 `tm_gmtoff` 在 64 位平台是 `i64`、32 位平台是 `i32`。历史页的本地时区格式化保留 `try_into`/`from` 转换（按 32 位写），并对本平台的 `clippy::useless_conversion` 就地放行——去掉转换在 32 位目标上编译不过，而 NFR-7 要求跨平台。

### P4 与封存设计的偏差清单（均不改任何 REQ）

| # | 偏差 | 原因 |
|---|---|---|
| 1 | `failure_of` 多收一个 `current: EngineKind` 参数 | 机翻通道上给"改用机翻"按钮没有意义，需要知道当前引擎 |
| 2 | `hide()` 用 `CancelClose` + `Minimized(true)`，设计写 `Visible(false)` | eframe 0.37 在 macOS 上不投递 `Reopen`，隐藏后 Dock 叫不回来 |
| 3 | 工具条与状态条落在 `views/mod.rs`，REQ-001 把四个区域都归给 `views::full` | 简洁模式要复用同一条状态条 |
| 4 | 按钮行多出一个"翻译"按钮 | 封存的 F-01..F-11 控件清单里没有它，但键盘之外必须有一条鼠标路径。已回写 `03-ui-design.md` 为 **F-12**，同时补 **F-13**（失败动作按钮）与 **F-14**（术语分区） |
| 5 | F-09 由三个按钮改为两个 | Q-002 用户决定，已回写 `01-requirements.md` FR-1.5、`03-ui-design.md`、`06-test-cases.md` |
| 6 | `MockEngine` / `HistoryStore` 的若干 FACT 更正 | 实现第一步实测后写回 `design.md` |
| 7 | `ctx.set_theme_preference(...)` → `ctx.set_theme(...)`；`ctx.style(closure)` → `ctx.style_of` / `all_styles_mut` | 0.37 的真实 API |
| 8 | `Effects` 增 `sync_glossary`、去 `probe_key` | 术语要同帧推给派发器；Key 探测改为启动时后台一次，不是界面动作 |
| 9 | 设置页多一个"窗口置顶"复选框 | NFR-4 同源决定：跟随系统之外必须留手动项，置顶同理 |
| 10 | 配置落盘是脏标记 + 退避 500ms，不是"当帧结束" | 字号滑块每帧标脏，逐帧写盘等于每帧一次原子 rename |
| 11 | P4 的简洁窗体编排是 `app.rs` 里的私有函数，不是第五个视图文件 | `views::compact` 留到 P5 与拖动/吸附/形变一起做 |
| 12 | `views/history.rs` 内含一个小的本地时间格式化（unix 走 `localtime_r`，其他平台退 UTC） | 不想为一个时间戳引入 `chrono`/`time` 依赖 |
| 13 | `main.rs` 长出可测的 `prepare()`，并用钥匙串事实对齐 `key_configured` | 启动逻辑原先全在 `main()` 里，无法测；config 里的标记只是缓存，钥匙串才是事实来源 |
| 14 | **启动时读钥匙串挪到建窗之后的后台线程**（`app::spawn_key_probe` / `apply_key_probe`），更正 `design.md` 的启动时序 | macOS 授权框会同步阻塞 `get_password()`，放在建窗前表现为"启动卡死"（真机踩到） |
| 15 | **术语在启动时推给派发器，且每次引擎重建后重灌**（`install_translator`） | 否则 `config.toml` 里的术语要等用户改一次术语才生效，违背 FR-4.5 |
| 16 | **新增 `ProviderError::SameLanguage`，机翻在发请求前拦下同语言对**；失败日志补记 HTTP 状态码 | 真机踩到：端点回英文 `PLEASE SELECT TWO DISTINCT LANGUAGES` 直接显示给用户，且日志里只有 `error="Status"` 无从定位。这是 `ice-providers` 的对外错误枚举多了一个变体，属 P3 封存面的扩张，故在此显式登记 |

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
| 2026-10-20 | **需求基线变更（Q-002，用户决定"合并成两个按钮"）**：FR-1.5 由"复制译文 / 复制对照 / 一键替换剪贴板"三个按钮改为**两个**——「复制译文」（其效果即整体替换剪贴板）与「复制对照」；空译文时两者都不触碰剪贴板，只提示。同步回写 `01-requirements.md` FR-1.5、`03-ui-design.md` F-09、`06-test-cases.md` TC-1.5-*、W-1.9 | "复制译文"与"替换剪贴板"在实现上是同一次 `set_text`，两个按钮做同一件事只会让用户怀疑区别在哪 |
| 2026-10-20 | **存储位置变更（Q-003，用户决定"改成写进 config.toml"）**：术语表由独立的 `glossary.toml`（`[[terms]]`）迁到 `config.toml` 的 `[[term]]`，`Glossary` 的持久化职责移入 `ice-core::config`；启动时若 config 内无术语而存在旧文件，做**一次性只读导入**并在设置页提示条数，**绝不删除或改写用户的 `glossary.toml`**；损坏时以空表降级、不阻塞翻译 | 需求 §4.5 原文就写的是 config 内 `[[term]]`，是实现走偏了；按需求原文改回，需求文字无需修改。不删用户文件是硬约束（用户自己的东西不由程序处置） |
| 2026-10-20 | **阶段归属变更（Q-001，用户答复"哪个更符合产品规范就用哪个"）**：需求 §5 第 5 条"单实例运行"由无归属改为**归 P4**（`CHG-003` REQ-009），实现为 `ice-app::system::single_instance`；`02-architecture.md` §2 的 `ice-app` 模块表补该行，`traceability.md` 单列一行追踪 | 判定依据是产品规范本身：P4 起 `history.jsonl` 与 `config.toml` 第一次被真实界面写入，两个实例并发写会损坏用户数据，等不到 P8 打包阶段 |
| 2026-10-20 | **启动时序更正**：读钥匙串从"建窗之前、同步"改为"建窗之后、`runtime.spawn_blocking` 后台"，结果经容量 1 的 `mpsc` 回投帧循环并 `request_repaint`；`main::prepare()` 接受 `Err(KeyState::Unknown)` 表示"探测未回"，此时不动 config 里的缓存标记，等 `apply_key_probe` 拿到真值再对齐。`02-architecture.md` §4/§6、`05-dev-guide.md` §8/§9 同步回写 | 真机踩到：macOS 对不由本程序写入的钥匙串条目会弹授权框，`get_password()` 同步阻塞，窗口根本建不出来，表现为"启动卡死"（日志停在 `get password from entry`，12s 后被杀、`config.toml` 一次未写）。不改任何 REQ |
| 2026-10-20 | **缺陷修复**：`config.toml` 里的术语从未推给派发器（要等用户改一次术语才生效），且 `replace_translator` 每次重建都会丢掉术语。改为启动即推一次，并把重建收进 `install_translator()`（换引擎后立刻重灌） | 违背 FR-4.5"即时生效"。由 `glossary_from_config_reaches_the_engine_on_the_first_job`、`rebuilding_the_translator_keeps_the_glossary` 双向锁住 |
| 2026-10-20 | **缺陷修复**：`capture_rect` 在外框矩形缺失时退回用内框原点，把标题栏高度算进保存的位置，每启动一次窗口往下挪一截。改为位置取外框原点、尺寸取内框，任一缺失（第一帧）返回 `None` 不覆盖配置 | 真机可见的窗口漂移。由 `capture_takes_position_from_outer_and_size_from_inner`、`capture_ignores_the_first_frame_where_geometry_is_unknown` 锁住 |
| 2026-10-20 | **`ice-providers` 对外错误枚举新增一个变体 `SameLanguage(&'static str)`**（10 → 11 个），机翻在发请求前拦下源/目标同语言的请求；失败日志在变体名之外补记 `status=` HTTP 状态码。`02-architecture.md` §5、`05-dev-guide.md` §6/§9 同步回写，`StubError::all()` 与 `variant_name`/`variant_label` 一并扩到 11 | 真机踩到：中文源文 + 目标简体中文时端点回 HTTP 200 但体内 `responseStatus: 403` + 英文 `PLEASE SELECT TWO DISTINCT LANGUAGES`，界面直接显示这句英文（违背 NFR-4 中文界面），且日志里只有 `error="Status"` 无从定位——补上状态码后一次就查到 403。状态码不是秘密（响应体已截断、查询串已洗掉），源文与译文仍不进日志。**这是对 P3 封存面的扩张，故显式登记** |
| 2026-10-20 | **控件清单补三项**：`03-ui-design.md` 新增 **F-12** 翻译按钮（在飞时禁用 + "翻译中…" + spinner）、**F-13** 失败动作按钮（401/未配置 → "去设置页填 Key"；AI 超时 → "改用机翻"）、**F-14** 设置页术语分区（即时生效并写进 `config.toml` 的 `[[term]]`）；ASCII 布局图同步 | 封存的 F-01..F-11 清单里没有"翻译"按钮，但键盘之外必须有一条鼠标路径；F-13/F-14 是实现中已落地却漏登记的控件。属文档补齐实现，不改 REQ |
| 2026-10-20 | **恢复判定改用 `inner_rect.is_some()`**，原实现看 `viewport().minimized` | macOS 上该字段运行时永不刷新（egui-winit 为避开 emilk/egui#3494 死锁只在 `is_init` 时填），原判据恒不成立 |
| 2026-10-20 | **`hide()` 改用 `CancelClose` + `Minimized(true)`**，设计原文写的是 `Visible(false)` | eframe 0.37 在 macOS 上不投递 `Reopen`，`Visible(false)` 之后点 Dock 图标叫不回窗口，FR-5.2 的"再次点击图标恢复"落空。Dock 恢复待真机走查确认 |
| 2026-10-20 | **设置页 AI 服务分区文案修正 + 控件清单补 F-15**：供应商下拉旁的提示补上"选完即生效，这一栏没有保存按钮"，未配置提示由"填好地址与模型、保存 Key"改为"选好供应商、保存 Key"；`03-ui-design.md` 新增 **F-15**（AI 服务分区）并写明"只有 Key 需要显式提交" | 真机走查时用户反馈："虽然配置好了，但是你说的保存按钮我没有找到。" 根因是我的指引与界面文案都暗示存在一个整体"保存"按钮，而 `select_provider()` 实际是选中即写缓冲、失焦即落盘，只有 Key 需要按钮（它要写钥匙串）。界面不说清楚，用户就会去找一个不存在的按钮——属可用性问题，不是代码缺陷，不改 REQ |
| 2026-10-20 | **缺陷 D-1（V-009 走查发现）**：文本页底部整条动作行被裁出 `CentralPanel`，翻译、复制译文、复制对照、失败动作四个按钮**既看不见也点不到**，用户只能用 Enter 触发翻译。根因是 `ui.columns` 用 `top_down_justified` 布局、两栏把可用高度整个撑满，排在其后的 `button_row` 落到了面板外。改为先用 `allocate_ui` 把两栏关进扣掉按钮行的高度里（`BUTTON_ROW_HEIGHT = 54`，按行间距 8 + 上间距 8 + 按钮实高 38 实测算出） | 用户走查反馈"W-1.9 我没有找到复制按钮"。**所有断言 `Effects` 的测试都照样通过**——无头测试只看"做了什么"，从不看"画到哪"，这是界面层测试方法的盲区。补 2 条布局测试（`the_action_row_stays_inside_the_space_it_is_given`、`the_action_row_fits_even_with_a_failure_button`），按最小窗口 640×420 扣掉工具条 56、状态条 28、面板内边距 16 之后的 320px 断言 `ui.min_rect()` 不溢出；修复前实测溢出到 y=374，修复后正好 320。另查 settings/history/image 三页都在 `ScrollArea` 里，不受此影响 |
| 2026-10-20 | **缺陷 D-2（V-009 走查发现）**：Cmd+W 收进后台之后点 Dock 图标恢复，窗口回来的是**一张被非等比拉伸的旧画面**——边框拖不动、字变窄变扁、按钮和输入框点上去毫无反应，用户只能关窗。两处根因：① `app.rs` 用 `viewport().minimized == Some(false)` 判恢复，而 `egui_winit::update_viewport_info` 在 macOS 上**只在 `is_init` 时填这个字段**（运行时问 `window.is_minimized()` 会死锁，emilk/egui#3494），我们自己发的 `Minimized(true)` 之后它永远停在 `Some(true)`，`reveal()` 一次都不会执行，`state.hidden` 也永远清不掉；② 最小化期间 `inner_rect`/`outer_rect` 是 `None`，恢复时若那次 `Resized` 没被处理，GL surface 会停在旧尺寸上。改为按"有没有真实几何"判恢复（同一个函数正是按 `is_minimized()` 决定要不要填 `inner_rect`，且该字段运行时刷新），`reveal()` 里把当前形态的几何命令整套重发一遍逼 winit 同步 surface，并在隐藏期间加 250ms 重绘心跳（eframe 在无待重绘窗口时把事件循环停在 `ControlFlow::Wait`，macOS 取消最小化不保证再给 `RedrawRequested`） | 用户走查反馈第三步。①是从 egui-winit 源码确证的（`if is_init \|\| !cfg!(target_os = "macos")` 那段带 #3494 注释），补 2 条测试锁住判据与"恢复必重发几何"；②与心跳属**针对性修复，尚待真机复验**——若仍冻结，需按 `RUST_LOG=eframe=trace` 的日志判断是主线程阻塞还是 surface 尺寸未同步。**FR-5.2 在复验通过前不成立**（本轮修复不足：真机第二轮仍是贴图，根因与最终解法见下方 2026-10-21 的「D-2 第二轮修复」行，第三轮复验通过后 FR-5.2 成立） |
| 2026-10-20 | 测试计数 233 → **237**（ice-app 131 → 135：2 条布局溢出测试 + 2 条窗口恢复测试） | 走查缺陷 D-1/D-2 的回归锁 |
| 2026-10-21 | **缺陷 D-3 + 动作条右下角化（用户第二轮走查提出）**：① 截图里"两边窗口大小不一样"——译文栏的 `ScrollArea(auto_shrink=false)` 吃光整栏可用高度、源文栏却为字数行让位矮一截，两栏底边差约 30px；改为两栏共用**定高页脚行**（`PANE_FOOTER_HEIGHT`）+ 正文/页脚都画进丢弃的子 Ui 并按给定矩形精确推进光标（`TextEdit` 外框、徽标内边距、空页脚的实际高度都不可控，行高定死底边才对齐）。② 用户要求底部按钮行改到**译文框内右下角的自带外框动作条**：第一版按原话做成"默认隐藏、鼠标碰译文框右下边框 0.15s 浮出"，用户看到实物后改为"**干脆不要收起了，就放在右下角不动**"，并指出 popup 外框的阴影"有点遮挡"译文框边框——最终版常驻右下角、外框改填充＋描边**无阴影**、四边离译文框边框 6px。按钮经丢弃子 Ui 放置（`Ui::put` 会推进光标并把矩形并进父 Ui，把译文框撑高） | 用户截图 + 两轮原话。新增 4 条测试（两栏对齐 ×2、动作条角落不压边 ×1、窄栏换行 ×1），删除被取代的 2 条按钮行溢出测试与 3 条悬浮隐藏/浮出测试 |
| 2026-10-21 | 测试计数 237 → **244**（ice-app 135 → 142） | D-3、右下角动作条与 D-2 第二轮恢复判据的回归锁（含 1 条走 `logic` 钩子的接线测试） |
| 2026-10-21 | **缺陷 D-2 第二轮修复**：第一轮（按 `inner_rect` 判恢复 + 重发几何 + `ui()` 里 250ms 心跳）真机复验仍失败，用户报"最小化重新叫出来，还是贴图"。读 eframe 0.37 源码定位到真正的根因：**隐藏期间 eframe 走 `update_logic_only`，只调 `App::logic`，`App::ui` 一帧都不执行**（`wgpu_integration.rs:716-767`、`epi_integration.rs:384`），而恢复判定与心跳全写在 `ui()` 里，窗口一收起来就双双断掉——没人再续 `request_repaint_after`，事件循环停在 `ControlFlow::Wait`，从 Dock 叫回来时没有任何一帧去发现"回来了"。改为把看护挂到 `App::logic`（`Context::run_logic` 会刷新 `raw.viewports`/`raw.focused`、会把 `logic` 里发的 viewport 命令带出去，`begin_pass_repaint_logic` 还专门保证这里发的重绘请求能到集成层）；判据加宽为"几何回来 **或** 窗口拿到焦点"（`focused` 每帧被 `update_viewport_info` 无条件刷新，`inner_rect` 却可能卡在 `None`），并加 800ms 静默期 `REVEAL_GRACE` 防 Cmd+W 那一帧把自己"恢复"回来；`reveal()` 补发 `Minimized(false)` | 用户第二轮反馈"还是贴图"。上一轮的日志只能证明 `reveal` 没执行、证明不了"没有帧"，这一轮从 eframe 源码确证隐藏期间 `ui` 根本不跑。新增 5 条测试（4 条 `RevealGuard` 判据：静默期内不判、焦点单独成立、几何单独成立、可见时不成立；1 条 `restoring_from_the_dock_is_noticed_by_the_logic_hook` 锁住"判定挂在 `logic` 上"这条接线——第二轮就是代码写对了却挂错了钩子），并在隐藏期间按状态变化打 debug 日志供复验取证。**第三轮真机复验通过**（2026-10-21，用户原话"可以了，收起叫回后依然能够使用"）：`/tmp/ice-retest.log` 记录收起 248ms 后进入 `revealed=false focused=false`、2084ms 时几何与焦点同时回来并触发"窗口恢复可见，重发几何"。FR-5.2 成立 |
| 2026-10-20 | 测试计数 99 → **233**（ice-app 3 → 131、ice-core 45 → 49、ice-providers 51 → 53；`--ignored` 仍 3 项）；`05-dev-guide.md` §4 计数、§2 目录结构、§6 界面层测试写法、§8 平台差异三行、§9 常见故障五行同步更新 | P4 落地。界面层测试固定为无头 `ctx.run_ui` + `Effects` 队列断言 + `StubEngine`，不碰网络/剪贴板/钥匙串 |
| 2026-10-21 | **缺陷 D-4（W-1.16 走查发现）**：粘贴 18,000 字符后源文框"变形、贴底、无法下拉"，译文栏"没有下拉"。左栏根因是 D-3 的布局手法留下的坑：两栏正文画进丢弃的子 Ui 再手工推进光标，而 `Ui::new_child` **不继承裁剪**（只克隆父 painter 的 clip_rect，egui ui.rs:210-235），`TextEdit::multiline` 又按内容全高分配矩形——18k 字符把源文框撑到窗口外，既裁不掉也没有滚动容器；译文栏有 `ScrollArea` 所以不变形。修复：两栏正文都进定高 `ScrollArea::vertical().auto_shrink([false,false])`；源文外框从 TextEdit 自带框改到定高 `Frame::group`（自带框跟着内容高度走，溢出时底边会被裁掉，两栏边框从此同源），焦点态用上一帧的 `focus_in_source` 决定描边颜色 | 用户原话 + egui 源码。新增 2 条测试：`a_huge_paste_keeps_the_source_editor_inside_its_pane`（滚动容器可视高度不超过栏目、内容远高于一屏）、`both_panes_scroll_when_the_content_does_not_fit`（8 帧滚轮位移后两栏偏移都增长）。**旧测试为何放走它**：栏目的 `min_rect` 只反映手工推进的光标，丢弃子 Ui 里溢出多少都看不见——D-1/D-3 那批布局断言因此全部通过。译文栏真机"不能下拉"无头复现不了（滚轮测试通过）；**第四轮真机复验通过**（2026-10-21，用户原话"都可以了"）：左框不变形、不贴底、四边边框完整、两指/滚轮能滚到末尾、字数行 18,000，译文栏同样能滚到底——D-4 关闭 |
| 2026-10-21 | 测试计数 244 → **246**（ice-app 142 → 144） | D-4 的回归锁 |
| 2026-10-21 | **缺陷 D-5（走查发现）**：提示条的关闭按钮、工具条的"⌗ 悬浮"、简洁窗的"▣ 全量 / ⧉ 复制"在真机上显示成豆腐块。根因：这四个装饰字形（U+2317 / U+2715 / U+25A3 / U+29C9）既不在 egui 内置字体里，eframe 0.37 的 macOS 系统字体回退（`egui_system_font_provider`，只在内置字体缺字时按 locale 匹配系统字体）也找不到它们。改为纯文字标签与 Latin-1 的 `×`（`悬浮窗` / `全量` / `复制` / `×`），不引入字体依赖——Windows/Linux 的系统回退更不可控，纯文字跨平台最稳。同一轮用户又指出提示条的关闭符排在文字**下方**（竖排布局所致），改为与文字同一行、跟在文字后面 | 用户截图 + 原话"点击后有奇怪的符号，不光是这一个按钮""叉号怎么在文字下面"。无新增测试：无头环境没有系统字体回退，断言不了真机的字形覆盖。**第四轮真机复验通过**（2026-10-21，用户原话"可以了"）：工具条「悬浮窗」、简洁窗「全量 / 复制」、提示条均无豆腐块，× 排在文字右侧同一行、可点掉、四秒自动消失——D-5 关闭 |
| 2026-10-22 | **需求基线变更（CHG-004 T-001 三轮实测后用户决定）**：FR-3.9 由"形态切换是同一个窗口的形变（180~220ms 尺寸/位置补间动画）"改为"**同一个窗口的瞬时切换，不做补间动画**"；Must 级别与编号不变（需求仍 44 条 · Must 30 · Should 12 · Could 2）。连带改动：`03-ui-design.md` §4 重写为一次性下发几何、`02-architecture.md` §3.3 的 `Morph` 段落改为瞬时下发 + 切换后回读落盘、`06-test-cases.md` W-2.14 判据改为"一步到位、不重建窗口、无黑屏闪烁"、`traceability.md` FR-3.9 行；`ice-app::mode::Morph` 不再实现 | 真机探针三轮实测（`features/CHG-004/evidence/T-001-hypothesis-verdicts.md` + `T-001-probe{,-v2,-v3}.log`）：① 逐帧 `InnerSize` 补间每 200ms 只有 4~5 次请求落到窗口，三次复现 ~1.0s 事件循环停摆，用户判"一卡一卡的，ppt 了"；② 设计里的第一级降级（30fps）同样不成立；③ 改"窗内补间"（动画期窗口不 resize、只插值画在窗口内的内容矩形）单向实测丝滑（204ms/14 帧真 60fps），但要做成双向就必须在起点把窗口撑成两端的外接矩形，v3 实测这样每次必撞 1027/1028/1042ms 停摆；同一轮里单发几何命令的「瞬移」对照 **20ms 落定**。停摆的真因是 **窗口 surface resize 本身**（v3 那次外接矩形恰好等于当前 rect、窗口尺寸未变的跑次 13 帧满速跑完），与命令频率无关 → 平滑的双向形变在 eframe/winit 0.30 + macOS 上拿不到，用户据此决定取消动画。原 FR-3.9 里可测的部分（同一窗口、不黑屏不闪烁、内容不丢、端点 ±2px、切换后可交互）全部保留 |
| 2026-10-23 | **需求基线变更（CHG-004 v0.4 / Q-009，用户决定"四边都藏 6px"）**：FR-3.5 的贴边停靠几何明确为**四边对称——窗口藏到那条边之外，可视区内只留 `dock_sliver_px`（6）的可见热区**。`features/CHG-004/spec.md` REQ-003 第一条的右/下公式由 `x = area.right - w + sliver`、`y = area.bottom - h + sliver` 改为 `x = area.right - sliver`、`y = area.bottom - sliver`（左/上原样不动）；第三条"SHALL NOT 产生负坐标或把窗口推到屏幕外"改为"只禁止非预期越界"的边界澄清（左/上停靠必然产生负坐标、右/下必然让 `rect.max` 超过 `area.max`，那正是上一条要藏的机制本身，两句并存时任何字面实现都无法同时满足）；夹取规则拆清为"交叉轴全量夹回可视区，停靠轴只在窗口长度超过该轴可用区时把起点夹到 `area.min`（`area.max - size` 此时是负数，照抄会把窗口甩到屏幕另一侧之外），且夹取 SHALL NOT 把 6px 热区一起夹没"。连带：`01-requirements.md` FR-3.5 行、`03-ui-design.md` §3 的吸附几何伪代码与说明、`06-test-cases.md` TC-3.5-a/b/d/e。CHG-004 由 v0.3 升到 **v0.4**，v0.3 的批准作废，需重走 `revise → approve → plan-stamp → seal-plan → transition`。Must 级别与编号不变（需求仍 44 条 · Must 30 · Should 12 · Could 2）。**连带改动**：拖离判定由"窗口到边线的几何距离"改为"松手原点相对**停靠锚点**沿停靠轴向屏幕内的位移 ≥ 阈值"（新增纯函数 `mode::released` + `IceApp::dock_anchor`，锚点不能取 `compact_rect`——`mode::capture` 每帧写它，拖动途中会被冲掉），因为窗口藏到边外之后那个几何距离在四条边上恒为约 `-454`，按原判据谁都拖不离；v0.3 的左/上本就已有此缺陷，只是 TC-3.5-d 当时只测右边没暴露。测试计数 300 → **305**（ice-app 196 → 201：`mode.rs` 加"四边各自判得出拖离"与"只跑几何判据会被吸回"两例、`app.rs` 加"向内拖 200px 就地解除并停在松手处""锚点不被每帧 capture 冲掉影响""沿交叉轴滑动仍保持停靠"三例），clippy/fmt 全绿；真机走查（四边各吸一次、各拖离一次、指示循环）仍属 T-011 | 实施 T-005（吸附几何）时发现同一 REQ 内部自相矛盾：左/上写的是"藏进去只剩 6px 可见"，右/下按字面算出来却是"窗口基本全可见、只往外多探 6px"，与本条自己声明的"保留 6px 可见热区供拖回"、`03-ui-design.md` §3 的同一句说明都相反，判定为抄写不对称而非有意设计。实现侧不能静默挑一个，故向用户给出四选项（按字面即已实现的行为 / 四边都藏 6px / 四边都齐边不藏 / 先真机试用再定），用户选"四边都藏 6px"。代码侧 `ice_app::mode::dock` 的右/下两支随之改公式，纯函数单测 TC-3.5-a/b 的期望值同步更新 |
