# CHG-003 · 全量界面：翻译闭环、设置页与密钥边界、系统集成

## 原始意图与来源

- 立项要求（2026-10-19，用户原话）："写个 rust 跨平台的客户端软件。哦，别忘了做两个界面……优先针对 mac。你自己规划和完成项目从立项到结束所要经历的所有阶段即可……每一个阶段都需要有验收阶段。"
- 阶段划分：`docs/00-charter.md` 把 P4 定为"全量界面"；`docs/01-requirements.md` 的 FR-3.1 验收方法写的就是"见 P4 界面走查表"，走查步骤在 `docs/03-ui-design.md` §5，控件对照表在同文 §2 的 F-01~F-11。
- 用户在 P1 走查后追加的三条（已登记进需求基线）：无边框简洁窗必须可拖动、模式切换要形变动画、配色跟随系统但要有手动开关。前两条属 P5（FR-3.2/3.9），第三条 NFR-4 的主题三态开关落在本包。
- 前置状态：CHG-002（引擎层）已 **ACCEPTED**，`spec_hash 88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c`，提交 `fe976ed`；两条真实通道均已实测（机翻 MyMemory、AI DeepSeek）。用户的 DeepSeek Key 已在钥匙串（service `IceTranslator` / account `ai-api-key`，2026-10-19 用户自行写入）。
- 用户明确要求供应商可选："可以自己选择供应商，现在国内几大供应商应该都差不多固定了" → P3 已落地 `AiProvider` 五家预设 + 自定义，本包负责把它做成设置页的下拉。

## 目标与非目标

**目标**：把 `ice-app` 从 240 行的 P1 骨架长成可用的全量界面——文本翻译闭环（输入→提交→加载态→结果/错误）、译文两个动作（复制译文 / 复制对照）、设置页（含 Key 写钥匙串）、历史页、术语页（术语改存 `config.toml`）、应用内快捷键、关窗常驻、单实例互斥、主题三态与字号缩放。领域层与引擎层已就绪，本包只长 UI、系统集成与两处用户本轮决定的存储/交互调整，不改引擎语义。

**非目标**（各自有归属阶段，本包不做也不验收）：

| 不做 | 归属 |
|---|---|
| 简洁模式的窄条布局、边缘吸附、失焦半透明、剪贴板即翻、形变动画（FR-3.2/3.5/3.7/3.8/3.9） | P5 = CHG-004 |
| 图片页的真实流水线：OCR、识别结果编辑、擦除回填、覆盖层、导出（FR-2.x） | P6/P7 = CHG-005/006 |
| 性能采样脚本、跨平台三目标核查、打包与 SDD 目录隔离（NFR-1/5/7） | P8 = CHG-007 |

单实例互斥（需求 §5 第 5 条）**在本包内做**（REQ-009）：理由是 P4 起界面会真实写 `history.jsonl` 与 `config.toml`，两个实例同时写同一份文件是会损坏数据的，等不到 P8。

本包只保证：切到简洁模式的按钮与快捷键存在、切换后源文/译文/语言对不丢（FR-3.3 的内容保留部分），简洁窗内部的版式与动画留给 P5。

## 事实与假设

| 标签 | 结论 | 来源 / commit / 环境 / 时间 |
|---|---|---|
| FACT | egui 0.37 的 `Options::theme_preference` 默认即 `ThemePreference::System`，native 侧由 `event_loop.system_theme()` 填充，且 `WindowEvent::ThemeChanged` 会实时更新 `egui_input.system_theme` → NFR-4 的"跟随系统 + 下一次重绘跟随"是框架既有能力，不需要自己监听系统外观 | `~/.cargo/registry` 源码：`egui-0.37.0/src/memory/theme.rs:142`、`egui-0.37.0/src/memory/mod.rs:206-207`、`egui-winit-0.37.0/src/lib.rs:506-507`；查阅时间 2026-10-20 |
| FACT | arboard 3.7 提供 `Clipboard::new()` / `get_text()` / `set_text()` / `set_image()` / `clear()` → FR-1.5 的两个按钮与 P7 的图片复制共用一个依赖，不新增 crate | `arboard-3.7.0/src/lib.rs:81-171`；根 `Cargo.toml:28` 已声明 |
| FACT | keyring 4.2 的 `Entry::new(service, user)` + `set_password` / `get_password` / `delete_credential` 齐备，FR-4.2 的写入、读取、清除三动作都能做；`get_password` 已在 P3 的 live 测试里实测读到用户真实 Key | `keyring-4.2.0/src/v1.rs:74-102`；`crates/ice-providers/tests/live.rs`（CHG-002 V-004 通过） |
| FACT | 领域层已提供本包需要的大部分能力：`Config::{load,save,sanitized}`、`UiConfig::{theme,font_scale,dim_when_unfocused}`、`TranslationConfig::{lang_pair,remember_target,visible_languages}`、`History::{append,recent,search,clear}`、`Glossary::{upsert,remove,post_substitute,prompt_fragment}`。**唯一的例外是术语持久化**：`Glossary::{path,load,save,load_at,save_at}` 现在指向独立的 `glossary.toml`，按本轮决定要改成走 `config.toml` 的 `[[term]]` | `crates/ice-core/src/{config,history,glossary}.rs`；45 项单测，CHG-002 V-001 通过；改动范围见 REQ-006 |
| FACT | UI 层可以完全离线验证：引擎抽象是 `Arc<dyn TranslationEngine>`，P3 已提供 `engine::mock::MockEngine`（可设回复、延迟、调用计数），HTTP 形状另有本地假服务 | `crates/ice-providers/src/{engine.rs,fake_server.rs}` |
| FACT | **本机无法合成鼠标点击**（P1 实测：AppleScript 取窗口信息超时），截图只能整屏采集后用 `sips --cropOffset` 裁剪 → 交互类验收必须是"无头断言 + 用户真人走查"的组合，不存在全自动 UI 测试这条路 | `docs/07-acceptance.md` P1 事实清单第 6~8 条；2026-10-19 |
| FACT | 机翻真实延迟 731–2133ms（单句）、1060 字符 3 段 1.9–3.2s，抖动大 → 加载态不是可选装饰，且请求期间输入必须不被阻塞 | CHG-002 证据 `features/CHG-002/evidence/*-V-003.txt`；2026-10-19/20 |
| FACT | eframe 0.37 与常见教程的 API 差异已在 P1 逐条纠正（`App::ui(&mut Ui, &mut Frame)`、`Panel::top/bottom`、`Decorations`、`OuterPosition`、`AppCreator` 返回 `Ok(Box<dyn App>)`），本包直接沿用已验证的写法 | `crates/ice-app/src/main.rs`（3 项无头断言通过）；`docs/07-acceptance.md` P1 事实清单 |
| HYPOTHESIS | egui 0.37 的 `TextArea` 承载 20,000 字符时输入不卡死（FR-1.1 的验收门槛）。**实现第一天就先做这次实测**，不达标则改实现方案（见 Q-004），不默认改需求 | 待实测；本仓库尚无 20k 字符的输入实测记录 |
| HYPOTHESIS | `ViewportCommand::Visible(false)` 在 macOS 上能让窗口隐藏而进程常驻，点 Dock 图标可恢复（FR-5.2）。若 eframe 0.37 在关窗时直接结束 run loop，则改走 `CloseRequested` 拦截；这是实现选择，不改验收标准 | 待实测；P1 只用过 `InnerSize`/`Decorations`/`WindowLevel`，未验证隐藏与恢复 |
| 决定 | 术语表改存 `config.toml` 内的 `[[term]]`，与需求 §4.5 的字面描述一致；旧的 `glossary.toml` 做一次性导入且**不删除用户文件**。原先"按 P2 现状接线、文字更正留到 P8"的方案被用户否掉 | 用户 2026-10-20 批准前问答；现状见 `crates/ice-core/src/glossary.rs:51-82` |
| HYPOTHESIS | `toml::to_string_pretty` 渲染带 array-of-tables 的 `Config` 时，只要术语字段是结构体的**最后一个字段**就能产出合法且可读的 `config.toml`（后续不会有键被吞进 `[[term]]`）。实现第一步先写一个往返单测证实；若不成立，改为在 `config.rs` 里手写渲染，不动需求 | 待实测；P2 只在独立的 `Glossary` 结构上验证过 `[[terms]]` 往返（`glossary.rs:226-233`），没验证过嵌在 `Config` 里的情况 |

## 行为与验收

### REQ-001: 全量界面骨架与页签（FR-3.1、NFR-8）

WHEN 应用以全量模式启动
THEN 系统 SHALL 渲染 `03-ui-design.md` §2 的四个区域：A 顶部工具条（页签 文本/图片/历史/设置、源语言下拉、目标语言下拉、引擎分段选择、⌗悬浮按钮）、B 源文区、C 译文区、D 状态条，且控件的命名与相对位置符合 §2 的控件清单 F-01~F-11。

WHEN 用户切换页签
THEN 系统 SHALL 保留各页自己的状态：离开文本页再回来，源文、译文、语言对、引擎选择都不变；页签本身不写入配置（重启回到文本页）。

WHEN 用户打开图片页（本阶段）
THEN 系统 SHALL 显示"图片翻译将在后续阶段落地"的占位说明，而不是空白页、报错或未实现的按钮。

WHEN 窗口被缩到最小允许尺寸（640×420）
THEN 系统 SHALL 保证无控件被裁掉或互相重叠，源文/译文两栏仍可用。

WHEN 用户用 Tab 遍历界面
THEN 系统 SHALL 让焦点顺序与视觉顺序一致、所有主要操作键盘可达、焦点环可见（1px 描边）。

WHEN `config.ui.font_scale` 改变（0.8~2.0）
THEN 系统 SHALL 即时按该系数缩放全局字号，不需要重启。

边界：全量模式下 Esc 不绑定任何动作（简洁模式的"Esc 清空输入"属 P5）。
并发：切页签只改 UI 状态，不发起任何请求。
验证层次：控件存在性与页签状态保留用无头断言（`egui::Context::default()` + 直接调渲染函数，沿用 P1 的做法）；视觉位置、最小尺寸、Tab 顺序、字号观感必须用户走查（本机无法合成点击）。

### REQ-002: 文本翻译闭环与竞态（FR-1.1、FR-1.2、FR-1.3、FR-1.8、NFR-1 的 UI 部分）

WHEN 用户在源文区按 Enter（焦点在源文区）或 Cmd+Enter（任意焦点），或点"翻译"按钮
THEN 系统 SHALL 提交一次翻译；Shift+Enter SHALL 只换行不提交；粘贴文本 SHALL NOT 自动提交。

WHEN 源文为空或只有空白
THEN 系统 SHALL 不发请求，译文区显示空态提示"输入后按 Enter 翻译"。

WHEN 请求正在进行
THEN 系统 SHALL 把主按钮变为 spinner + "翻译中…"并禁用重复提交；若已有旧译文，则在右上角显示细条"更新中"且不清空正文；源文区在此期间保持可编辑。

WHEN 结果返回
THEN 系统 SHALL 展示译文、耗时，以及缓存命中标记（命中时显示"缓存"、不显示耗时）；在源语言为"自动检测"时回显检测到的语言，显式指定源语言时不回显。

WHEN 已有更新的请求被提交，而旧请求的结果此时才返回
THEN 系统 SHALL 按单调递增的 `request_id` 丢弃过期结果：不回写译文、不闪一下旧内容、不改耗时标记。

WHEN 用户切换引擎（机翻↔AI）或改目标语言
THEN 系统 SHALL 对当前源文自动重译（视为一次新提交，不复用旧结果），并在 1 秒内给出可见反馈（加载态）。

WHEN 源文长度达到 20,000 字符
THEN 系统 SHALL 保持输入可用、不卡死，并能出结果（AI 通道的分段由引擎层负责，UI 只透传与展示段数）。

WHEN 相同源文 + 相同引擎 + 相同语言对再次提交
THEN 系统 SHALL 命中本地缓存、不发网络请求，并在界面上标出这是缓存结果。

并发：UI 线程 SHALL NOT await 网络，也 SHALL NOT 做像素运算；任务用 tokio `spawn`，结果经 `mpsc::unbounded` 回投并调 `ctx.request_repaint()`（架构 §4）。
回归：引擎层与领域层行为不变，P3 的 96 项单测（core 45 + providers 51）必须仍全绿。
验证层次：状态机、竞态丢弃、加载态、缓存标记、段数展示全部用 `MockEngine`（可设延迟与调用计数）在无头断言里跑；"20,000 字符不卡死"必须真机实测并记录输入响应观感与耗时。

### REQ-003: 译文操作与剪贴板（FR-1.5，已按用户决定合并为两个按钮）

WHEN 用户点"复制译文"
THEN 系统 SHALL 把译文文本写入系统剪贴板（这同时就是需求里"一键替换系统剪贴板为译文"的效果——剪贴板被译文整体覆盖），并给出可见的成功反馈。

WHEN 用户点"复制对照"
THEN 系统 SHALL 把"原文 + 译文"的对照文本写入剪贴板，格式固定（原文段、空行、译文段），且原文里的换行原样保留。

WHEN 译文为空（尚未翻译或翻译失败）
THEN 两个动作 SHALL NOT 触碰剪贴板，并提示"没有可复制的译文"——不静默清空用户剪贴板里原有的东西。

WHEN 剪贴板不可用（被其他进程占用、系统拒绝）
THEN 系统 SHALL 显示可读错误、不 panic、不清空已展示的译文。

边界与幂等：重复点同一按钮结果一致；两个按钮互不串台（"复制对照"绝不只写译文）。
变更来源：用户在 2026-10-20 的批准前问答中明确"合并成两个按钮"。这是需求文字调整，实现阶段 SHALL 同步改 `docs/01-requirements.md` FR-1.5（三个动作 → 两个按钮，说明"复制译文"即替换剪贴板）、`docs/03-ui-design.md` F-09（按钮数 3 → 2）、`docs/06-test-cases.md` 的 TC-1.5-a/b/c，并在 `docs/07-acceptance.md` 的变更登记新增一行（依据：本 spec 问答，非实现者自行决定）。
验证层次：写入内容的组装（对照格式、空译文保护）是纯函数，直接单测；真实剪贴板读写用 arboard 在真机点一次并粘出来验证（本机可读写剪贴板，无需授权弹窗）。

### REQ-004: 错误可读、带动作、且不丢源文（FR-1.7）

WHEN 引擎返回 `Unauthorized`（401）
THEN 系统 SHALL 在译文区用红字显示"API Key 无效，请重新保存"，并提供可点击的"去设置"动作，点击后跳到设置页并定位到 Key 输入。

WHEN 引擎返回 `NotConfigured`
THEN 系统 SHALL 显示"AI 服务未配置"并提供同一个"去设置"动作。

WHEN 引擎返回 `RateLimited`（429）
THEN 系统 SHALL 显示"被限流，稍后重试"，且 SHALL NOT 禁用翻译按钮。

WHEN 引擎返回 `Timeout(ms, retries)`
THEN 系统 SHALL 显示"超时（已重试 n 次）"，并提供"改用机翻"动作，点击后切到机翻引擎并立即重译。

WHEN 引擎返回 `Empty` / `Decode` / `Http` / `Status`
THEN 系统 SHALL 分别显示对应的人话文本（来自 `ProviderError` 的 `Display`，UI 不重新拼语义），并保留原文。

WHEN 任何一次翻译失败
THEN 系统 SHALL 保证：源文完整留在输入区、进程不 panic、源文不出现在日志与窗口标题里、状态条从"就绪"变为错误态。

回归：错误文案的唯一来源是 `ice-providers::error`，UI 层只在"需要给出动作按钮"时 match 具体变体（架构 §5 的规则），不得复制一份文案。
验证层次：每种错误变体用 `MockEngine` 或本地假服务注入，无头断言检查渲染出的文本与动作按钮；"源文仍在输入框"由同一个断言覆盖（读回 `AppState.source`）。真实断网/错 Key 的人工验证在走查里做一次。

### REQ-005: 设置页持久化与密钥边界（FR-4.1、FR-4.2、NFR-4）

WHEN 用户打开设置页
THEN 系统 SHALL 提供这些可配项：AI 供应商预设（百炼 / DeepSeek / 智谱 / Kimi / 火山方舟 / 自定义）、`base_url`、`model`、API Key、默认引擎、默认语言对、界面字号、主题（跟随系统 / 强制浅色 / 强制深色）、启用的额外语言。

WHEN 用户修改任一项
THEN 系统 SHALL 立即生效（不需重启）并持久化；重启后 SHALL 仍是修改后的值。

WHEN 用户选择供应商预设
THEN 系统 SHALL 自动填入该家的 `base_url` 与常用 `model`；用户手填的值优先于预设；选"自定义"且地址为空时 SHALL 提示并把 AI 通道标记为不可用（状态条显示"AI 未配置"）。

WHEN 用户保存 API Key
THEN 系统 SHALL 把它写入 macOS 钥匙串（service `IceTranslator`、account `ai-api-key`，条目名固定），并且：`config.toml` 里只出现"是否已配置"的布尔量、日志里只出现长度与哈希前 8 位、界面上只显示"已配置"与"清除"按钮——**不回显 Key 的任何片段**（包括后 4 位）。

WHEN 用户点"清除 Key"
THEN 系统 SHALL 二次确认后调 `delete_credential`，之后 AI 通道不可用、机翻通道不受影响。

WHEN 钥匙串不可用（被锁、权限被拒、条目损坏）
THEN 系统 SHALL 显示可读错误、应用继续可用（机翻照常）、不 panic、不把错误详情里的任何凭据信息写进日志。

WHEN 主题选"跟随系统"
THEN 系统切换深浅色后 SHALL 在 1 秒内跟随（下一次重绘）；WHEN 选"强制浅色"或"强制深色" THEN 系统外观变化 SHALL NOT 影响应用配色，且两主题下所有文本可读。

WHEN 界面语言
THEN 全部文案 SHALL 为简体中文，中英混排不破版（NFR-4）。

边界与幂等：重复保存同一个 Key 不产生重复钥匙串条目（keyring 的 `set_password` 是覆盖语义）；设置页的"保存"对未改动项是幂等的。
隐私硬约束：Key 的输入框内容 SHALL NOT 进入 tracing 日志、窗口标题、历史文件、错误文本、截图证据或提交内容。
验证层次：主题三态到 `theme_preference` 的映射、"手填优先于预设"的取值、Key 掩码文案是纯逻辑，单测覆盖；钥匙串真实写入/读取/清除在真机做一次，用"条目存在性退出码 + `config.toml` 与日志全文 grep 无 `sk-` + 界面不回显"三件套取证，绝不在证据里出现 Key。

### REQ-006: 历史页与术语表（FR-4.4、FR-4.5）

WHEN 一次翻译成功
THEN 系统 SHALL 追加一条历史记录（源文、译文、引擎、语言对、时间）到 JSON Lines 文件，追加写、不重写整文件。

WHEN 用户打开历史页
THEN 系统 SHALL 显示最近 500 条，支持关键词搜索、点击一条回显到源文/译文区、清空全部。

WHEN 用户点"清空历史"
THEN 系统 SHALL 先二次确认，确认后清空文件；应用不重启、当前源文与译文不受影响。

WHEN 历史文件损坏或缺失
THEN 系统 SHALL 在历史页显示"历史不可用"与原因，翻译功能照常，不 panic。

WHEN 记录数超过 500
THEN 系统 SHALL 只保留最近 500 条（丢弃最旧）。

WHEN 用户在术语页新增、修改或删除一条术语
THEN 系统 SHALL 立即生效且持久化到 `config.toml`（按需求 §4.5 的格式写成 `[[term]] source="..." target="..."`）：AI 通道把它注入提示词、机翻通道做译后替换，已缓存的结果也走同一后处理（不重新发请求）。

WHEN 应用启动且 `config.toml` 里没有术语、但配置目录下存在旧的 `glossary.toml`
THEN 系统 SHALL 把旧文件里的术语一次性导入内存与 `config.toml`，并在设置页顶部显示一条说明（"已从 glossary.toml 导入 N 条术语"）；**SHALL NOT 删除或改写用户原有的 `glossary.toml` 文件**（那是用户自己的数据，删除需用户显式授权），导入是幂等的——第二次启动不重复导入、不产生重复条目。

WHEN 术语数据损坏（`config.toml` 的 `[[term]]` 解析失败，或旧 `glossary.toml` 解析失败）
THEN 系统 SHALL 以空表启动并提示，不阻塞翻译；`config.toml` 整体解析失败时沿用 FR-4.3 的降级语义（默认配置 + 启动提示条），不因术语段损坏而丢掉其余配置。

WHEN 术语条目很多
THEN `config.toml` 的 `[[term]]` 数组 SHALL 排在文件末尾（TOML 的 array-of-tables 之后的键都属于该表，`Config` 结构里术语字段必须是最后一个字段），提示词注入仍受 `max_prompt_terms()=60` 的截断保护。

边界：搜索是子串匹配（不做分词、不做模糊）；回显一条历史 SHALL NOT 自动触发新请求（只填内容，等用户按 Enter）。
变更来源：术语存储从 `glossary.toml` 改为 `config.toml` 内的 `[[term]]`，是用户 2026-10-20 的明确决定（"改成写进 config.toml"），改完即与需求 §4.5 的字面描述一致，需求文字**无需**修改。代价：`ice-core::glossary` 的 `path/load/save/load_at/save_at` 要改成围绕 `Config` 的读写（或保留 `Glossary` 类型但把持久化职责移到 `config.rs`），P2 的 `roundtrip_through_toml` 等 6 项单测随之调整；实现阶段 SHALL 在 `docs/02-architecture.md` §2 的模块表与 §8 落地表、`docs/05-dev-guide.md` 的测试计数、变更登记里同步回写。
验证层次：搜索、500 条裁剪、回显不触发请求、术语即时生效都用 mock 引擎 + `tempfile` 单测（core 层已有 history 5 项、glossary 6 项，术语存取改造后项数会变）；文件损坏路径注入畸形 JSON Lines/TOML 断言降级；旧文件导入用 `tempfile` 造一个 `glossary.toml` 断言"导入 N 条 + 原文件哈希不变 + 二次启动不重复"；真实的历史与配置文件位置（`~/Library/Application Support/IceTranslator/`）在真机确认一次。

### REQ-007: 应用内快捷键与关窗常驻（FR-5.1、FR-5.2、FR-3.3 的内容保留部分）

WHEN 用户按 Enter（焦点在源文区）或 Cmd+Enter（任意焦点）
THEN 系统 SHALL 触发翻译。

WHEN 用户按 Cmd+Shift+M
THEN 系统 SHALL 在全量与简洁模式之间切换，且切换后源文、译文、语言对完整保留、不重复发请求。（简洁窗内部的版式、吸附与形变动画属 P5；本包只保证切换与内容不丢，切换是即时的、允许"硬切"。）

WHEN 用户按 Cmd+Shift+C
THEN 系统 SHALL 复制译文（等价于点"复制译文"，含空译文保护）。

WHEN 用户按 Cmd+Shift+P
THEN 系统 SHALL 切换置顶状态并持久化（简洁模式下才看得出来，快捷键本阶段就生效）。

WHEN 用户按 Cmd+W 或点窗口关闭按钮
THEN 系统 SHALL 隐藏窗口而进程常驻（空闲 CPU < 1%），点 Dock 图标 SHALL 恢复窗口且状态完好（源文、译文、页签、语言对都在）。

WHEN 用户按 Cmd+Q
THEN 系统 SHALL 先落盘配置（模式、窗口几何、置顶、主题、字号、默认引擎与语言对）再退出。

WHEN 请求进行中用户重复按 Enter
THEN 系统 SHALL NOT 叠加第二个请求（沿用 REQ-002 的禁用与 `request_id` 语义）。

边界：快捷键 SHALL NOT 与系统或输入法常用组合冲突；逐个按键在走查里验证（走查表 W-4）。
验证层次：按键到动作的映射、内容保留、关窗落盘用无头断言（检查排队的 `ViewportCommand` 与 `AppState`，沿用 P1 的 `queued(ctx)` 手法）；"进程常驻 + Dock 恢复 + CPU < 1%"必须真机实测（`ps`/Activity Monitor 取样）。

### REQ-008: 验收方式与证据规范

WHEN 本包验收
THEN 系统 SHALL 按 `03-ui-design.md` §5 的五步走查执行并把逐条结论写进 `docs/07-acceptance.md` 的 P4 段：① 控件清单 F-01~F-11 逐条对照（存在 / 位置符合 / 行为符合三列）；② 空态、加载态、错误态各一张图（错误态用故意填错的 Key 制造）；③ 缩到最小尺寸确认无裁切重叠；④ 亮/暗主题各一张；⑤ Tab 顺序走一遍。

WHEN 截图要入库
THEN 每张 SHALL 裁剪到只含本应用窗口所在矩形，提交前逐张打开确认画面里没有用户其他应用的内容（代码、聊天、文档都算）；裁不干净就不提交，改记文本或日志证据。屏幕锁定时拍到的是壁纸，采集前先确认会话已解锁。（P1 立的硬规则，本包继续适用。）

WHEN 需要真实网络
THEN 只跑两条：机翻真实端点（不需 Key）与 AI 真实通道（用户钥匙串里的 Key）。输出经脱敏后进证据；其余全部用 mock 引擎与本地假服务。

WHEN 涉及密钥的验证
THEN 证据里 SHALL 只出现"条目存在/不存在"的退出码、长度、哈希前 8 位；SHALL NOT 出现 Key 明文或任何可还原片段。

WHEN 本包完成
THEN 系统 SHALL 回写 `docs/traceability.md`（FR-1.1/1.2/1.4/1.5、FR-3.1、FR-4.1/4.2/4.4/4.5、FR-5.1/5.2、需求 §5 的单实例条、NFR-4/8 的实现列与结果列）、`docs/02-architecture.md` §2 的 `ice-app` 模块表（补 `system::{secrets,clipboard,hotkeys,single_instance}` 与 `views::*`）与 §8 的落地表、`docs/05-dev-guide.md` 的测试计数与目录约定，并在 `docs/07-acceptance.md` 的变更登记新增相应行（含 FR-1.5 合并按钮、术语存储迁移两条）；文档里不留"待 P4"与"已完成"两种并存说法。

### REQ-009: 单实例互斥（需求 §5 第 5 条）

WHEN 应用启动且配置目录下的锁文件不存在，或存在但持有进程已消失
THEN 系统 SHALL 取得锁（在 `~/Library/Application Support/IceTranslator/` 下建锁文件并写入本进程 PID），然后正常启动。

WHEN 应用启动且锁被一个仍然活着的进程持有
THEN 系统 SHALL NOT 打开窗口、SHALL NOT 写 `config.toml` 或 `history.jsonl`，而是给出可读提示（"Ice Translator 已在运行"）后以非零退出码结束；提示里 SHALL NOT 泄露锁文件之外的路径细节。

WHEN 持锁进程崩溃或被强杀（留下陈旧锁文件）
THEN 下一个实例 SHALL 能识别"锁里记录的 PID 已不存在"并接管，不要求用户手动删文件。

WHEN 应用退出（含 Cmd+Q 与窗口销毁）
THEN 系统 SHALL 释放并删除自己创建的锁文件；删除只针对本次启动创建的那一份（比对 PID），SHALL NOT 删掉别的实例刚建的锁。

边界：REQ-009 是"防同一份用户数据被两个进程并发写坏"的保险，不是授权/许可机制；不做跨机器互斥、不做 IPC 把第二个实例的意图转给第一个（例如"点 Dock 图标恢复已运行窗口"仍走 macOS 自己的应用激活，FR-5.2）。
判定依据（用户 2026-10-20 的答复是"哪个更符合产品规范就用哪个"）：产品规范这边有两处硬约束——需求 §5 第 5 条明写单实例，FR-4.4/FR-4.3 又要求 `history.jsonl` 追加写与 `config.toml` 原子替换；从 P4 起这两份文件第一次被真实界面写入，并发写会直接损坏用户数据，所以放本包，不留到 P8。
验证层次：锁的获取/陈旧接管/释放是纯文件系统逻辑，用 `tempfile` 造锁文件（写入不存在的 PID、写入自己的 PID、写入损坏内容三种）单测；"第二个实例真的退出且不写文件"用一次真机双开验证（第二个进程退出码 + 配置文件 mtime 不变）。

## 未决问题

| id | 问题 | 阻塞 | 结论 |
|---|---|---|---|
| Q-001 | 需求 §5 第 5 条的"单实例运行"（第二个实例提示并退出）没有归属阶段，架构模块表里也没有对应模块 | 否 | **已决（用户答复"哪个更符合产品规范就用哪个"）**：放本包，见 REQ-009。理由不是"打包时才需要"，而是 P4 起 `history.jsonl` 与 `config.toml` 第一次被真实界面写入，两个实例并发写会损坏用户数据。实现阶段在 `docs/02-architecture.md` §2 的 `ice-app` 模块表里补 `system::single_instance` 一行 |
| Q-002 | FR-1.5 的"复制译文"与"一键替换系统剪贴板为译文"在实现上是同一次写入 | 否 | **已决（用户答复"合并成两个按钮"）**：REQ-003 只留"复制译文"与"复制对照"两个按钮，空译文保护照旧。实现阶段同步改 `docs/01-requirements.md` FR-1.5、`docs/03-ui-design.md` F-09、`docs/06-test-cases.md` TC-1.5-*，并登记变更行 |
| Q-003 | 术语表实际存 `glossary.toml` 的 `[[terms]]`，需求 §4.5 写的是"config 内 `[[term]]`" | 否 | **已决（用户答复"改成写进 config.toml"）**：按需求原文实现，见 REQ-006；旧文件一次性导入且不删除。需求 §4.5 无需修改。剩下一个技术不确定性（serde/toml 的 array-of-tables 渲染顺序）已降级为事实表里的 HYPOTHESIS，实现第一步用往返单测打掉 |
| Q-004 | 20,000 字符输入的卡顿风险未知（HYPOTHESIS） | 否 | **保持未决，不阻塞批准**。实现第一天先实测。若不达标：先改实现（懒渲染/限制可见行数/提交时才全文进引擎），仍不达标再回来找你，由你决定是调需求门槛还是接受带限制通过。默认按此推进 |

## 允许动作与停止条件

**批准即允许**：本地实现与验证；跑真实端点的只读翻译请求（机翻免费端点 + 用户自己的 DeepSeek Key，额度消耗极小）；在用户于设置页点"保存"时写钥匙串条目；写用户配置目录下的 `config.toml` / `history.jsonl` / 单实例锁文件；**读**旧的 `glossary.toml` 做一次性导入；按 REQ-003/REQ-006 的"变更来源"改 `docs/01-requirements.md`、`docs/03-ui-design.md`、`docs/06-test-cases.md` 中与这两个决定直接冲突的文字并登记变更行；`docs/` 与 `features/CHG-003/` 的其余文档回写；阶段验收后 commit + push。

**不允许**：未经登记就改需求基线（FR-1.5 与术语存储这两处是用户本轮明确授权的，仍要写变更登记行）；删除或改写用户的 `glossary.toml`（只读导入，删不删由用户自己决定）；把源文、截图或任何内容上传到第三方；在证据、日志、提交里出现 Key；删除用户已有文件；未经确认执行 `rm`。

**停止条件**（触发即记 `SPEC_DEVIATION` 退回评审，不自行改需求或换方案）：

1. egui 0.37 无法表达某个必需控件（例如状态条的可点击动作、译文区可选中复制）；
2. 20,000 字符实测导致输入不可用，且改实现方案后仍不达标；
3. 钥匙串写入被系统拒绝且无替代路径（FR-4.2 是 Must，不能降级成明文存配置）；
4. `Visible(false)` 与 Dock 恢复走不通，导致 FR-5.2 无法在不改架构的前提下满足；
5. 走查发现控件位置与 `03-ui-design.md` §2 不符而我判断改设计比改实现更合理（设计变更需要你确认）；
6. 术语写进 `config.toml` 后出现无法通过实现手段规避的问题（例如术语量大到配置文件解析变慢、或 `[[term]]` 与既有字段的渲染顺序无法稳定），需要退回"独立文件"这个与你本轮决定相反的方案时。

## 时间估算

- 开发 8~11 小时：9 个 REQ，UI 代码量比 P3 大（预计新增 1,400~2,000 行 + 50~70 项单测），领域层与引擎层已就绪、无网络不确定性；`ice-app` 要拆成 `state` / `jobs` / `system::{secrets,clipboard,hotkeys,single_instance}` / `views::{full,settings,history,glossary}` / `mode`（部分）。
  - 其中本轮三个决定新增的工时：单实例互斥约 +1 小时（REQ-009，含陈旧锁接管与真机双开验证）；术语存储迁到 `config.toml` 约 +1~2 小时（改 `ice-core::glossary`/`config.rs` 的存取、调整 P2 的 6 项单测、写导入逻辑并断言不碰用户旧文件）；FR-1.5 合并按钮约 −0.5 小时（少一个按钮与相应文案，但要改三处文档）。
- 验证 2~3 小时：无头断言为主，真机走查一轮（需要你配合，约 20~30 分钟），两条真实端点各跑一次，另加钥匙串三件套取证与双开验证。
- 外部等待：你的 spec 批准一轮、走查一轮、验收签名一轮——不计入开发工时。
- 依据：P3 的 7 个 REQ 实际用一个工作日完成（含 replan 与两轮 live 实测）；P4 的 REQ 数与代码量都更大，风险集中在三处 HYPOTHESIS（20k 字符输入、`Visible(false)` + Dock 恢复、`Config` 内嵌 `[[term]]` 的渲染），第一天就会逐个打掉。
