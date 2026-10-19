Change: CHG-002
Version: v0.1
Spec-Hash: 88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

# CHG-002 · Design

## 实现方案

仓库 `ice-translator`，分支 `main`，基线为 CHG-001 验收后的提交。新增文件全在 `crates/` 与 `docs/`，
不改 `crates/ice-app`（P4）与 `crates/ice-vision`（P6/P7）。

模块划分（延续 ADR-006 单向依赖）：

| 新增 | 位置 | 内容 |
|---|---|---|
| `machine.rs` | `ice-providers` | `MachineTranslateEngine`：MyMemory GET + 450 字符切块 + 响应体状态判定 |
| `ai.rs` | `ice-providers` | `AiEngine`：OpenAI 兼容 POST、提示词构造、分段与段间上下文 |
| `protect.rs` | `ice-providers` | 纯函数占位符保护、还原与占位符安全的切块 `chunk_chars` |
| `error.rs` | `ice-providers` | 追加 `should_retry` / `from_reqwest` / `from_status`，并剥离错误文本里的 URL 查询串 |
| `fake_server.rs` | `ice-providers`（仅 `cfg(test)`） | 本地假 HTTP：按序应答、指定状态码、挂起、确定没人监听的地址 |
| `AiProvider` | `ice-core::config` | 供应商预设（端点 + 默认模型）与 `effective_*` 取值规则，纯数据 |
| `tests/live.rs` | `ice-providers` | 三条 `#[ignore]` 真实端点测试，是本 crate 唯一碰钥匙串的地方（dev-dependency） |

外部调用统一走 `ice-core` 已有的 `Language::code()`（`zh-CN` 形态，MyMemory 直接可用），
不另建语言码映射。源语言判定用 `ice_core::detect()`（P2 已实现的加权字符集检测），
判不出来时按 `en` 请求并把回显留空。`TranslationEngine` trait、`TranslationRequest/Result`、
`ProviderError` 均已存在，本包只实现不改动其签名，避免动到已测通过的调度层。

超时与分段上限都来自配置而不是写死：超时取 `config.translation.timeout_ms`（默认 15 秒），
AI 分段上限取 `config.translation.ai_chunk_chars`（默认 3500），机翻块上限是端点硬约束常量
`MACHINE_QUERY_CHARS = 450`。`reqwest::Client` 每通道建一次并复用。网络类失败最多额外重试 2 次；
重试只针对超时与连接失败，401/429/4xx/5xx 与解析失败不重试（重试不会改变结果，反而放大限流）。

`cargo test --workspace` 全部离线可跑：所有 HTTP 交互用本地 `TcpListener` 起的假服务或纯函数单测覆盖，
真实端点测试标 `#[ignore]`，由 `--ignored` 显式触发（见"验证层次"）。日志只写长度与哈希前缀（NFR-3）。

## REQ-001 实现与验证

`MachineTranslateEngine::translate`：

1. `protect` 后用 `chunk_chars(template, MACHINE_QUERY_CHARS)` 按**字符数**切块（不是字节数），
   优先在 `\n`、句末标点处断开，且不劈开占位符；
2. 每块 `GET https://api.mymemory.translated.net/get?q=<urlencoded>&langpair=<src>|<tgt>`。
   源语言：用户显式指定则照用（此时 `detected` 留 `None`，界面不假装"检测到"）；
   "自动检测"用 `ice_core::detect()`，置信度 < 0.3 或判不出来时按 `en` 请求且回显留空；
3. 解析 `responseData.translatedText` 与 `responseStatus`。`responseStatus` 实测既可能是数字 `200`
   也可能是字符串 `"403"`（错误写在 body、HTTP 仍是 200），因此用 `serde_json::Value` 取值后归一化为
   `u16`，非 200 即 `ProviderError::Status(code, responseDetails)`；状态字段形态不认识 → `Decode`，
   **不猜成功**；
4. `translatedText` 为空或全空白 → `ProviderError::Empty`（不是"翻译成功但结果为空"）；
5. 块间直接相接（切块本身无损，有测试保证拼接等于原文），`segments` = 块数。

单测：`chunk_chars` 的长度上限与边界（恰好上限、含换行、含中文标点、碎块防护）；
假服务返回 200/`"403"`/空译文/畸形 JSON 的映射；800 字符文本被拆成 2 块、每块解码后 ≤ 450 字符且拼接等于原文；
超时错误带重试次数且假服务收到 3 次请求；连接被拒时说"不可达"而不是"被拒绝"。
实时验证：`#[ignore]` 的 `machine_live_short` 与 `machine_live_long_text_is_chunked`
（后者 1060 个字符，实测切成 3 段、2.4 秒返回译文，而不是收到 body 里的超限错误）。
MyMemory 是公开免费只读服务，请求里只有测试自带的示例句。

## REQ-002 实现与验证

`ice-core::config` 增加：

```rust
pub enum AiProvider { Bailian, DeepSeek, Zhipu, Kimi, VolcengineArk, Custom }   // #[default] = Custom
impl AiProvider {
    pub const fn preset_base_url(self) -> &'static str;   // Custom 返回 ""，表示必须手填
    pub const fn preset_model(self) -> &'static str;      // 方舟返回 ""：它要填推理接入点 ID
    pub const fn needs_manual_model(self) -> bool;
}
```

预设值取各家公开的 OpenAI 兼容基址（到 `/v1` 或 `/v4`、`/api/v3` 版本段为止），全部是 https。
`AiConfig` 增加 `provider` 字段（结构上已有 `#[serde(default)]`，旧 TOML 缺该字段读为 `Custom`，
行为与升级前完全一致）。取值规则是"手填优先、预设填空"：

```rust
pub fn effective_base_url(&self) -> String  // base_url 为空则取 preset_base_url()
pub fn effective_model(&self) -> String     // model 为空则取 preset_model()
pub fn is_ready(&self) -> bool              // 两个 effective_* 都非空 且 key_configured
pub fn endpoint(&self) -> String            // effective_base_url 归一化到 /chat/completions；地址为空时返回空串
```

所以选了 DeepSeek 不必再手打地址就能用，而配置文件手改仍然生效（FR-4.3 的可手动编辑性不被削弱）。

`AiEngine::new(cfg, api_key, timeout, segment_chars)` 与 `AiEngine::from_config(&Config, api_key)`。
`is_available()` 只作为 trait 方法实现（不在 `impl AiEngine` 上另开同名 inherent，避免两条调用路径分叉）
= `cfg.is_ready() && !api_key.trim().is_empty()`。不可用时调度层已在发请求前拦下（`dispatcher.rs` 现有逻辑），
引擎内部再断言一次以防直接调用，返回 `NotConfigured`。
请求体：`{"model":..., "messages":[{"role":"system","content":<提示词>},{"role":"user","content":<段原文>}], "temperature":...}`，
`temperature` 为 `None` 时字段整个不发（部分供应商拒绝该参数为空值）；Key 走 `Authorization: Bearer`。
提示词按顺序含：源语言（显式时写明，自动时交给模型判断）→ 目标语言 → 只输出译文 →
`[[n]]` 必须原样保留且各出现一次 → 保留换行 → 术语偏好（有则加）→ 承接上文（有则加）。
响应读 `choices[0].message.content`；缺失或空数组 → `Decode`，但只报结构问题、**不回显响应体**
（响应体里是用户的译文）。401/429/其他状态按 REQ-005 映射。

单测：每个预设的 `endpoint()` 都无重复 `/chat/completions` 且为 https；旧 TOML（无 `provider` 字段）
解析为 Custom；手填值覆盖预设；Custom 缺地址时 `is_ready()` 为 false；请求体形状（model、messages 的
role/content、缺省不发 temperature、非缺省发 temperature）、Bearer 头、提示词含目标语言与术语；
`is_available()` 在缺 Key、缺 base_url、缺模型三种情况下均为 false，且此时 `translate` 一个请求都不发。
实时验证：`ai_live_deepseek`（需用户先把 Key 写进钥匙串）。

## REQ-003 实现与验证

`protect.rs` 提供：

```rust
pub struct ProtectedText { pub template: String, pub originals: Vec<String> }
pub struct Restored { pub text: String, pub lost: Vec<usize> }
pub fn protect(text: &str) -> ProtectedText;   // 受保护片段 -> [[n]]
pub fn restore(translated: &str, originals: &[String]) -> Restored;
pub fn chunk_chars(text: &str, max: usize) -> Vec<String>;
```

规则按优先级排列，靠前规则先占位，其后规则不得切开已保护区间（重叠即跳过），
所以代码块内的 URL 只整体保护一次：``` 围栏代码块 → 反引号内联代码 → URL → 邮箱 →
日期（`2026-10-19`、带时刻）→ 带单位数字（`12 MB`、`5ms`、`3 元`…）→
驼峰标识符（`MyTranslateEngine`、`getUserInfo`）→ 下划线标识符（`user_id`）→
原文里本就存在的 `[[3]]`（转义成占位符，避免与我们的编号碰撞）。
**单个首字母大写的普通英文词（Kubernetes）刻意不保护**：它既会误伤句首单词，
机翻也几乎不会改写这类专名；保护清单以"看得出是标识符"为界。

占位符格式 `[[n]]`：纯 ASCII、模型极少改写；还原时按字面匹配而不是按位置，
因此模型调整句序也能还原（有测试覆盖）。`restore` 对缺失占位符不做猜测：
`[[n]]` 在译文中消失时，把对应原文追加到末尾并登记进 `lost`，
上层按 REQ-003 的"无法逐字节还原即报失败"处理：`lost` 非空返回 `ProviderError::Decode`。

`chunk_chars` 与保护同在纯函数层，两通道共用（机翻 450、AI 用配置值）：
按字符数而非字节数切；优先 `\n` 与句末标点；断点若落在占位符内部则移到占位符之外；
断点太靠前（不足上限一半）时改为硬切，否则会切出几乎空的碎块，白花一次请求。
切点必须落在字符边界——中文句号占 3 字节，早期版本 `i + 1` 直接把 `&str` 切崩过（有测试锁住）。

`protect` 是纯函数，AI 与机翻两条通道都用它（机翻同样会改写数字与代码）。
还原发生在引擎内部、返回给调度层之前，因此 `dispatcher` 的术语后处理顺序不变。

单测：清单里每类各构造一例，断言 `protect` → `restore(identity)` 逐字节等于原文；
同一标识符出现 2 次各自占位、全部还原；代码块内含 URL 时只整体保护一次；
超长代码块整体被一个占位符替掉，模板因此远小于切块上限；
不含受保护内容时 `template == 原文` 且 `originals` 为空；普通英文句子不被占位符污染；
切块不劈开占位符、不产生碎块、拼接等于原文；占位符丢失被报进 `lost`。

## REQ-004 实现与验证

分段上限取 `config.translation.ai_chunk_chars`（默认 3500），切块复用 `chunk_chars`，
重试策略与机翻各自独立：

- 每段请求把上一段译文末尾 160 字符（`CONTEXT_TAIL_CHARS`）放进 system 提示词的"承接上文"槽位，
  第一段沿用调用方传入的 `context_tail`（跨次翻译的上下文）；
- 段序严格递增，译文按序相接；源段以换行结尾而模型丢了它时补一个 `\n`，避免两段黏成一行；
- `TranslationResult.segments` 取实际段数；
- 任一段失败立即返回该段错误，不返回半截译文（`?` 传播，无 catch 继续）。

单测：小上限逼出两段，断言第二段提示词含"承接上文"且含第一段译文尾巴；
两段按顺序相接并保住换行；第 2 段返回畸形 body 时整体 `Err`，`Ok` 路径不存在；
段数与 `segments` 一致。

## REQ-005 实现与验证

错误映射集中在 `ice-providers::error`（两通道共用）：

| 观察到的情况 | 返回 |
|---|---|
| `reqwest::Error::is_timeout()` | `Timeout(配置的超时毫秒, 已重试次数)` |
| 连接失败/被重置/DNS | `Http("无法连接 <host>：网络不可达或被中间网络设备拦截，不是服务端拒绝")` |
| 401 | `Unauthorized` |
| 429 | `RateLimited` |
| 其他非 2xx | `Status(code, 截断到 200 字符的原因文本)` |
| 响应体里的业务错误码（机翻） | `Status(code, responseDetails)` |
| JSON 解析失败/缺字段 | `Decode` |

重试判据 `should_retry()` 与界面判据 `retryable()` **有意不同**：自动重试只针对超时与连接失败，
429 与 4xx/5xx 不自动重试（立刻重试只会放大限流，且改不了结果）；上限 `EXTRA_ATTEMPTS = 2`，
之间不 sleep 退避（自用工具、单用户，避免叠加延迟；NFR-1 要求 P50 < 800ms）。
源文安全：`translate()` 只借用 `&TranslationRequest`，无写路径；
`reqwest` 的错误 Display 会把完整 URL（含 `?q=<用户原文>`）带出来，因此 `from_reqwest`
连接分支只取主机名、兜底分支用 `scrub_queries` 剥掉 `?` 之后的查询串；
`Decode` 只报结构问题、`Status` 只截断原因文本，都不拼响应体。

单测：假服务分别制造超时（挂起不响应）、连接被拒（端口未监听）、401、429、500、畸形 JSON、
响应体业务错误码；断言错误种类、`retryable()`/`should_retry()` 的差异，
断言超时重试到上限（假服务计数 3）而 401/429/500 只发 1 次；
断言错误消息里不出现源文子串、也不出现 `q=`。

## REQ-006 实现与验证

`AiEngine` 只从构造参数拿 Key（`api_key: String`），自身不读钥匙串、不读 `config.toml`——
"密钥来自何处"留在 P4 的 `ice-app::system::Secrets`（ADR-005 归属层），
生产代码里 `ice-providers` 不依赖 `keyring`；只有 `tests/live.rs` 通过 dev-dependency 读真实条目，
它默认 `#[ignore]`，不进 `cargo test --workspace` 的常规路径，也不接受环境变量注入（ADR-005）。

- `impl Debug for AiEngine` 手写：只打印 provider、model、endpoint、`key_len` 与超时，
  Key 值一律不出现（`{:?}` 是最常见的泄露途径）；
- 发请求前只记 `model`、字符数、段数与文本指纹：`ice-providers` 内私有
  `fn text_fingerprint(&str) -> u64`，基于 `std::hash::{Hash, DefaultHasher}`（不新增第三方依赖；
  该指纹只用于日志关联，不承担完整性或安全校验），日志写 `len=… fp=<十六进制前 8 位>`；
- `AiConfig` 不含 Key 字段，`Config::save` 的原子写写不出 Key；
- 401 文案固定为"API Key 无效或被拒绝（HTTP 401）"，不拼接响应体里的任何回显。

单测：`format!("{:?}", engine)` 不含传入的 Key 子串且含 `key_len`；
`toml::to_string(&config)` 结果不含 `api_key`；假服务返回 401 时错误 Display 不含 Key；
指纹稳定、随内容变化、格式化后 8 位；`ice-providers` 的 `[dependencies]` 里没有 `keyring`
（由 `Cargo.toml` 与"引擎不碰钥匙串与磁盘"共同证明边界）。

## REQ-007 实现与验证

新增 `docs/adr/ADR-004-machine-translate-endpoint.md`，字段与其他 ADR 一致（状态、日期、关联、背景、决策、理由、影响）。
内容：选定 MyMemory 的理由（无 Key 可用 + 本机实测可达 + 单次 500 字符上限已被 REQ-001 的切块覆盖）、
被淘汰的 `translate.googleapis.com`（本机 15 秒连不上，与 P1 记录一致）、
失效条件（免费额度或限流不可用时的降级路径，回指需求基线第 2 条）与替代方案（离线词表 / 用户自备 Key）。
同时回写 `docs/02-architecture.md` §8 与 `docs/07-acceptance.md` 中"ADR-004 待 P3"的三处引用为"已落档"，
并在变更登记新增一行；`docs/traceability.md` 的 FR-1.3/1.6/1.7/1.9/NFR-1/NFR-3 行的实现列补上本包模块名。

验证：`grep -q` 断言 ADR-004 文件存在且状态行已填写；`tools/check_traceability.py` 保证编号集合仍一致；
文档一致性由人工评审（追踪验证只证明映射存在，不证明语义正确）。

## 影响与回滚

影响文件：新增 `crates/ice-providers/src/{machine,ai,protect,fake_server}.rs`、
`crates/ice-providers/tests/live.rs`、`docs/adr/ADR-004-*.md`；
修改 `crates/ice-core/src/config.rs`（`AiProvider` 与 `AiConfig.provider`、`effective_*` 取值规则）、
`crates/ice-providers/src/{lib,error}.rs`、根 `Cargo.toml`（reqwest 增加 `query` feature——0.13 起
`RequestBuilder::query` 被关在特性后面，不开就编译不过）、
`crates/ice-providers/Cargo.toml`（`urlencoding` 依赖 + 仅测试用的 `keyring` dev-dependency）、
`docs/{02-architecture,traceability,05-dev-guide,07-acceptance}.md`。
不新增第三方大依赖，不改 `ice-app`（P4）与 `ice-vision`（P6/P7）。
不影响：UI 层、OCR、配置磁盘格式的向后兼容（旧 TOML 缺 `provider` 字段可读，读为 `Custom`）。
风险：真实端点的响应格式与限流行为可能变化（免费服务），失效条件写进 ADR-004，
触发时按 spec 的停止条件走 SPEC_DEVIATION 而不是静默换端点。
回滚：`git revert` 本阶段提交即移除四个模块、live 测试与 ADR；`config.rs` 的新增字段有
`#[serde(default)]`，回滚后旧配置文件仍可被回滚后的代码读取。

## 验证层次

| 层 | 命令 | 能证明什么 |
|---|---|---|
| 本地/`VERIFIED_CODE` | `cargo test -p ice-core -p ice-providers` | 切块、解析、错误映射、保护还原、预设与配置兼容（HTTP 走本地假服务） |
| 本地/`VERIFIED_CODE` | `cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check` | 静态门槛，不含行为语义 |
| Mock/`VERIFIED_CODE` | `cargo test -p ice-providers protect` | FR-1.6 的逐字节不变（纯函数，无网络） |
| 真实端点/`VERIFIED_RUNTIME` + `environment=sandbox` | `cargo test -p ice-providers -- --ignored machine_live`、`... ai_live` | 真实第三方契约：MyMemory 与 DeepSeek 各一条最小请求。这里的 "sandbox" 指被测的是外部真实服务，而非本系统的部署环境；本包不部署任何东西 |
| 未覆盖 | 百炼/智谱/Kimi/火山方舟的真实契约 | 本机网络 TLS 建连被重置（Q-001 决议），只保留预设与 Mock 证据，验收记录标注未验证，待可用网络补做 |

`ai_live` 需用户先把 Key 写入钥匙串（`security add-generic-password -a ai-api-key -s IceTranslator -w`）并授权，
它经 `keyring` 的 dev-dependency 读取真实条目——不进生产代码，也不接受环境变量注入（ADR-005）。
`cargo test --release` 与打包属 P8。
