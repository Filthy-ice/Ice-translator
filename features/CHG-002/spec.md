# CHG-002 · 翻译引擎层：机翻通道、AI 通道与不可译内容保护

## 原始意图与来源

2026-10-19 用户立项诉求（原话见 `docs/01-requirements.md` §1 与项目记录）：自己用的翻译软件，
机翻与 AI 双通道，可切换；AI 供应商由用户自己选（国内几家已收敛到 OpenAI 兼容接口），
"到了开发 AI 翻译阶段，告诉我在哪配置即可，我配上后你测试"。

需求基线：FR-1.3（双引擎可切换）、FR-1.6（不可译内容保护）、FR-1.7（失败必须可读且不丢源文）、
FR-1.8（缓存，P2 已实现并有测试，本包不重做）、FR-1.9（AI 超长自动分段）、
FR-4.2 + ADR-005（密钥只进钥匙串）、NFR-1（机翻 P50 < 800ms）、NFR-3（日志不记源文全文）。
阶段安排：`docs/02-architecture.md` §8 落地表把 `machine`、`ai`、`protect` 与 ADR-004 归本阶段（P3）。

代码事实源：`crates/ice-providers/src/{engine,request,cache,dispatcher,error}.rs` 已完成并通过 10 项测试；
`machine.rs`、`ai.rs`、`protect.rs` 尚不存在，本包新建。`crates/ice-core/src/config.rs` 的
`AiConfig`（base_url / model / temperature / key_configured）与 `endpoint()` 归一化已存在。

## 目标与非目标

目标：

- 机翻通道零配置可用：不要求用户注册、不要求 Key，发出请求并拿到按序拼回的译文；
- AI 通道走 OpenAI 兼容 `chat/completions`，供应商以预设可选、允许自定义 `base_url`，缺 Key 时明确不可用；
- 不可译内容（URL、邮箱、代码块、标识符、带单位数字）在两通道输出中逐字节不变；
- 所有失败路径给出可读错误与"可否重试"的判断，且绝不丢源文、不 panic；
- 补齐 ADR-004（机翻端点选型），把 P1/P3 的实测结论与淘汰理由写进 `docs/adr/`。

非目标：

- 不做界面、不做设置页、不动 `crates/ice-app`（P4）；不做剪贴板与快捷键（P4/P5）；
- 不做图片 OCR 与回填（P6/P7）；不改缓存策略与术语后处理逻辑（P2 已完成）；
- 不为非 OpenAI 兼容格式的供应商写适配器；不做流式输出、函数调用、多轮对话；
- 不在本包做打包隔离（P8），只保证新增文件不含密钥与本机绝对路径；
- 不改动 `docs/01-requirements.md` 的需求语义（本包只实现，不改需求）。

仓库范围：`ice-translator`（本仓库），目标分支 `main`。

## 事实与假设

| 标签 | 结论 | 来源 / commit / 环境 / 时间 |
|---|---|---|
| VERIFIED_RUNTIME | MyMemory 可用：`q=hello world&langpair=en\|zh-CN` 返回"你好世界"，`responseStatus:200` | 本机 `curl` 直连，2026-10-19 |
| VERIFIED_RUNTIME | MyMemory 单次查询硬上限 500 字符；660 字节实测返回 `responseStatus:"403"`、正文 `QUERY LENGTH LIMIT EXCEEDED. MAX ALLOWED QUERY : 500 CHARS`，且 HTTP 层仍是 200 | 本机 `curl`，2026-10-19；错误在响应体里，不在状态码 |
| VERIFIED_RUNTIME | `api.deepseek.com/v1/chat/completions` 可达：不带 Key 返回 HTTP 401 | 本机 `curl` POST，2026-10-19 |
| VERIFIED_RUNTIME | 本机网络对 `dashscope.aliyuncs.com`、`open.bigmodel.cn`、`api.moonshot.cn`、`ark.cn-beijing.volces.com` 在 TLS 建连阶段 `Recv failure: Connection reset by peer`；环境变量中无代理设置 | 本机 `curl -w %{errormsg}`，2026-10-19 |
| VERIFIED_RUNTIME | `translate.googleapis.com` 连接失败（15s 超时），与 P1 记录一致 | 本机 `curl`，2026-10-19；`docs/07-acceptance.md` P1 事实段 |
| VERIFIED_CODE | `AiConfig::endpoint()` 已把 base_url 归一化为 `.../chat/completions`，`is_ready()` 要求 base_url、model 与 Key 标记齐备 | `crates/ice-core/src/config.rs:191-213` |
| VERIFIED_CODE | `ProviderError` 已具备 `Timeout/Status/Unauthorized/RateLimited/NotConfigured/Decode/Empty/Http` 与 `retryable()` | `crates/ice-providers/src/error.rs:3-46` |
| VERIFIED_CODE | 调度层已实现"选引擎→查缓存→调用→术语后处理→回写"，AI 通道注入提示词，机翻通道做译后替换 | `crates/ice-providers/src/dispatcher.rs`，7 项测试通过（`cargo test -p ice-providers`，2026-10-19） |
| VERIFIED_CODE | `reqwest`、`urlencoding`、`regex`、`async-trait` 已在 `ice-providers` 依赖中，无需新增依赖 | `crates/ice-providers/Cargo.toml` |
| BUSINESS_CONFIRMED | 密钥只存 macOS 钥匙串（service `IceTranslator`，account `ai-api-key`），不支持环境变量注入 | ADR-005 决策 1、5 与"影响"段，用户已接受 |
| HYPOTHESIS | 被重置的四家供应商在用户家用网络或其他网络下可达 | 只在当前网络证伪了可达性，未证明端点本身有问题；预设只是默认值，不作为本包验收前提 |

## 行为与验收

### REQ-001: 机翻通道零配置可用且不受长度限制

WHEN 用户选择机翻并提交非空源文，且未配置任何 Key
THEN 系统 SHALL 直接向机翻端点请求，返回译文并把源语言检测值带回；空译文 SHALL 报为可读错误而不是返回空串。

WHEN 源文长度超过单次查询上限
THEN 系统 SHALL 按不超过 450 字符切块，逐块请求并按原顺序拼回，分隔与原文空白保持一致，不因超长而失败或截断。

WHEN 服务端把错误写在响应体里而 HTTP 状态仍是 200（如 `responseStatus:"403"`、`QUERY LENGTH LIMIT EXCEEDED`）
THEN 系统 SHALL 依据响应体状态判定失败并给出该原因，SHALL NOT 把错误文案当译文返回。

边界：本 REQ 无权限与幂等维度（单次只读 GET，不写任何本地状态）；并发由 `TranslationEngine` 的
`Send + Sync` 约束保证，实例可被 UI 跨线程共享。回归：不改动 `dispatcher`/`cache` 的既有行为，
其原 7 项测试须继续通过。

### REQ-002: AI 通道走 OpenAI 兼容接口且供应商可选

WHEN 用户在配置中选择内置供应商之一
THEN 系统 SHALL 使用该供应商的请求端点与默认模型名，内置清单为：阿里云百炼、DeepSeek、智谱 GLM、
Kimi（Moonshot）、火山方舟，以及"自定义"（由用户填 `base_url`）。

WHEN 用户填写的 `base_url` 已含 `/chat/completions`、带结尾斜杠、或只到版本段
THEN 系统 SHALL 归一化成唯一正确的请求地址（沿用 `AiConfig::endpoint()` 现有行为，不重复实现）。

WHEN API Key 未保存或 `base_url`/模型名为空
THEN 通道 SHALL 报告"未配置"并令 `is_available()` 为 false，界面可据此提示去设置页；SHALL NOT 发出请求。

异常路径：HTTP 401 → "Key 无效或被拒绝"；429 → "被限流，请稍后再试"；其余非 2xx → 带状态码与原因的失败。
权限与边界：请求体只含用户提交的源文与提示词，不附带机器名、路径或其他应用数据。

### REQ-003: 不可译内容在两通道中逐字节保持

WHEN 源文包含 URL、邮箱、``` 包裹的代码块、`XxxYyy` 形式标识符、带单位数字（如 `12 MB`、`2026-10-19`）
THEN 系统 SHALL 在请求前把这些片段替换为占位符、在收到译文后原样还原，使它们在最终译文中的字节与源文完全一致。

WHEN 模型无视占位符格式并改写了占位符本身
THEN 系统 SHALL 仍能还原出原始片段（按占位符匹配而非按位置回填），并在无法还原时报失败而不是返回残缺文本。

WHEN 源文不含任何受保护片段
THEN 系统 SHALL 不做替换、不改写空白，直接提交原文（无额外加工）。

边界：代码块内部的换行与缩进属于受保护内容，须整体保留；同一标识符多次出现 SHALL 全部还原。
并发：保护与还原是纯字符串计算，无共享状态。

### REQ-004: AI 超长输入自动分段并保持段间一致

WHEN AI 通道收到的源文超过 3,500 字符
THEN 系统 SHALL 切为若干段（每段不超过 3,500 字符）按序请求，并把上一段译文尾部作为下一段的上下文提示，
最终按段顺序拼回完整译文，`segments` 字段与实际段数一致。

WHEN 某一段请求失败
THEN 系统 SHALL 立即返回该段的可读错误，SHALL NOT 返回只拼到一半的译文冒充成功。

WHEN 分段边界落在句子中间
THEN 系统 SHALL 优先在换行或句号处断开，找不到时才按长度硬切，保证不丢字符、不重字符。

不适用说明：机翻通道的切块上限（REQ-001）与本 REQ 的 3,500 字符是两套独立参数，不合并成一个常量。

### REQ-005: 失败语义可读、可判断可否重试、且不丢源文

WHEN 请求超时、连接被重置、DNS 失败、返回非 2xx、或响应结构无法解析
THEN 系统 SHALL 分别返回对应的可读错误（超时含已重试次数；连接类失败说明是网络不可达而非服务拒绝），
并给出该错误"可否重试"的判断；任何情况下 SHALL NOT panic，SHALL NOT 清空或覆盖用户源文。

WHEN 网络类失败发生（超时或连接失败）
THEN 系统 SHALL 按既定策略最多重试 2 次、单次超时不超过 8 秒，重试全部失败后以 `Timeout`/`Http` 形式上报，
不做无限重试。

边界：本 REQ 的"源文不丢"在引擎层的含义是——失败只返回 `Err`，不经由任何写路径改动调用方持有的请求；
界面上的源文保留属 P4 验收范围。

### REQ-006: 密钥与日志边界

WHEN 需要 AI 通道的 Key
THEN 系统 SHALL 只从钥匙串（service `IceTranslator`，account `ai-api-key`）读取，
`config.toml` 的 `ai` 段 SHALL NOT 出现密钥字段，只有 `key_configured` 标记。

WHEN 记录日志或抛出错误
THEN 系统 SHALL NOT 输出 Key、请求头 Authorization 值，也 SHALL NOT 输出源文全文；
源文只记长度与哈希前缀。

WHEN 钥匙串读取失败或没有条目
THEN 通道 SHALL 报"未配置/无法读取"并继续可用（机翻通道不受影响），SHALL NOT 把密钥回退写入任何文件。

权限：钥匙串条目属当前登录用户；本 REQ 不请求任何跨用户或系统级权限。
验证层次：钥匙串真实读写属 P4（`ice-app::system::Secrets`）；本包以"引擎实例只接收传入的 Key 字符串、
自身不读磁盘、不读配置"来证明密钥不进日志，实时钥匙串验证在 P4 补做并记录。

### REQ-007: 机翻端点选型决策落档（ADR-004）

WHEN 本包完成机翻通道实现
THEN 系统 SHALL 在 `docs/adr/` 下有 ADR-004 文件，记录选定端点、实测证据（REQ-001 事实行的来源）、
被淘汰端点及淘汰理由（`translate.googleapis.com` 本机不可达）、以及该决策的失效条件与替代方案。

WHEN ADR-004 与 `docs/02-architecture.md` §8、`docs/07-acceptance.md` 变更登记中的"待 P3 补"说法冲突
THEN 系统 SHALL 以 ADR-004 与需求基线为准并回写这两处引用，不留两处并存的事实。

边界：本 REQ 是文档产出与一致性，不含运行时代码；幂等（重复执行 `check` 不改变文件内容）。

## 未决问题

见 `change.json.questions`：Q-001（AI 实时契约验证是否在本阶段完成）需用户决定，未关闭前不得批准。

## 允许动作与停止条件

批准范围仅 local-delivery：在本仓库内设计与实现 `ice-core`/`ice-providers` 代码与文档，
本地跑 `cargo test` 与 Mock/本地服务验证，向可达的公共只读端点（MyMemory、DeepSeek 端点连通性探测）
发少量不含个人信息的测试文本，commit 并 push 到 `origin/main`。

明确不包含：读取或写入用户钥匙串条目（P4 由用户自己配置后进行）、用用户 Key 发起计费请求前的告知、
部署、发布、写入其他仓库、改动 `~/Desktop/project-ssd`。

停止条件：

- 需要真实 Key 才能完成的验证 → 先完成本地与 Mock 层，把实时验证作为待补证据登记，不冒充通过；
  若用户决定本阶段不做实时验证，本包在 `VERIFIED_CODE` + `Mock/local` 证据下进入 VERIFIED，
  并把该前提写进验收记录，供以后在可访问网络下补测。
- 若发现机翻端点无法稳定满足 FR-1.7（如随机 403、限流严重）或需要 Key 才能用 → 记录 SPEC_DEVIATION
  退回 SPEC_REVIEW，按需求基线第 2 条的降级方案（离线词表 / 用户自备 Key）重新评审，不自行换成
  "看起来能用"的端点。
- 若发现需求 44 条之外要新增行为（如流式输出）→ 走 `revise` 递增版本重新评审。

## 时间估算

开发与实现：0.8~1.2 人日（三个新模块 + 预设清单 + ADR-004；调度/缓存/错误已就绪，不用重做）。
验证：`cargo test --workspace` 现有 53 项约 20 秒内，新增单测与 Mock 服务测试预计 15~25 项；
实时探测单条约 1 秒（本机实测 MyMemory 往返 < 1s）。
外部等待：REQ-002/REQ-004 的真实供应商契约验证需用户配置 Key 并授权，等待时间不计入开发工时。
依据：REQ-001/REQ-002 的网络探测耗时、P2 阶段同类模块（`request`/`cache`/`dispatcher`）的实际测试数量。
