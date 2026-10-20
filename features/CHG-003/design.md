Change: CHG-003
Version: v0.1
Spec-Hash: 84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

# CHG-003 · Design

仓库 `ice-translator`，分支 `main`，基线为 CHG-002 验收后的提交 `2c64e93`。
改动集中在 `crates/ice-app`（从 240 行的 P1 骨架长成完整界面）、`crates/ice-core`
（术语持久化改到 `config.toml`）与 `docs/`（回写与变更登记）。**`crates/ice-providers` 的语义不改**，
`crates/ice-vision` 不动（P6/P7）。

## 实现方案

### 模块划分

| 新增 / 改动 | 位置 | 内容 |
|---|---|---|
| `app.rs` | `ice-app` | `IceApp`：`eframe::App` 实现，帧循环、页签分发、快捷键、`ViewportCommand` 收口 |
| `state.rs` | `ice-app` | `AppState`（源文/译文/加载态/`request_id`/页签/提示条）与纯状态转移函数，无头可测 |
| `jobs.rs` | `ice-app` | 翻译任务的派发与回收：tokio `spawn` + `mpsc::unbounded` + 过期结果丢弃 |
| `theme.rs` | `ice-app` | `Theme → ThemePreference` 映射、`font_scale → ` 全局字号、主色/错误色/降级色 |
| `system/secrets.rs` | `ice-app` | keyring 读写删，**应用侧唯一碰钥匙串的地方**；只回 `Option<已配置>` 与长度/哈希前缀 |
| `system/clipboard.rs` | `ice-app` | arboard 封装 + 两个按钮的文本组装（纯函数）+ 空译文保护 |
| `system/hotkeys.rs` | `ice-app` | `egui::InputState → Action` 的纯函数映射，便于无头断言 |
| `system/single_instance.rs` | `ice-app` | PID 锁：`acquire` / 陈旧锁接管 / `release`（只删自己那份） |
| `views/{full,settings,history,image}.rs` | `ice-app` | 四个页签的绘制函数，签名统一为 `fn show(ui, &mut AppState, &mut Effects)` |
| `mode.rs` | `ice-app` | 全量↔简洁切换（P4 只做即时切换与内容保留，形变动画属 P5） |
| `Config.terms` | `ice-core::config` | 新增最后一个字段 `#[serde(rename = "term")] pub terms: Vec<Term>` |
| `Glossary` 存取 | `ice-core::glossary` | 删掉 `path/load/save/load_at/save_at`，改为 `from_config` / `write_to_config` / `import_legacy` |

`ice-app` 是 binary crate，单元测试写在各自文件的 `#[cfg(test)] mod tests` 里（P1 已用这个手法做过
三条 `ViewportCommand` 断言），不额外拆 lib target。

### 依赖变更

`crates/ice-app/Cargo.toml` 增加：`keyring.workspace = true`（正式依赖，不再是 dev-only）、
`async-trait`（测试里的假引擎要实现 `TranslationEngine`）、`[target.'cfg(unix)'.dependencies] libc = "0.2"`
（REQ-009 判定 PID 存活用 `kill(pid, 0)`；macOS 没有 `/proc`，而 `libc` 本就是 tokio/reqwest 的
传递依赖，显式声明不增加实际编译量），以及 dev-dependency `tempfile`（锁文件与历史文件的临时目录测试）。
不引入其他第三方 crate。

### 启动时序

```
main()
 ├─ tracing 初始化（EnvFilter，默认 info）
 ├─ single_instance::acquire()  ── 已有活实例 → eprintln 提示 + exit(1)，不建窗口不写文件
 ├─ Config::load()              ── 损坏 → 默认值 + reason（黄色提示条，FR-4.3）
 ├─ glossary::import_legacy()   ── config 内无术语且存在旧 glossary.toml → 只读导入，返回条数
 ├─ secrets::read()             ── 只取"是否存在 + 长度 + 哈希前 8 位"，Key 明文只进 AiEngine
 ├─ Translator 装配             ── register(machine)；ai.is_ready() 时才 register(ai)
 ├─ tokio runtime（多线程）     ── 由 IceApp 持有，UI 线程绝不 await
 └─ run_native(ViewportBuilder) ── 形态/几何/置顶直接取配置，避免闪烁（FR-3.1/3.6）
on_exit / Drop ── 落盘 config（含术语）→ 释放锁
```

### 并发模型（沿用架构 §4，不新增机制）

翻译任务：`state.submit()` 递增 `request_id` → `jobs::spawn(translator, kind, req, id, tx)` →
tokio `spawn` 里 `await translator.translate(...)` → `tx.send(JobOutcome { id, result })` →
`ctx.request_repaint()`。帧首 `jobs::drain(&mut state)` 只接受 `outcome.id == state.request_id`
的结果，其余静默丢弃（不回写、不闪旧内容、不改耗时标记）。加载态由 `state.in_flight: Option<u64>` 表达。
`Translator` 内部是 `Arc` 化的引擎与 `Mutex<LruCache>`，跨任务共享一个实例。

### 三处 HYPOTHESIS 的处置

| 假设 | 处置 | 结论 |
|---|---|---|
| `Config` 内嵌 `[[term]]` 能否被 `toml` 稳定渲染 | 设计前用 `/tmp` 独立探针（`toml 1.1` + `serde`）实测：字段在末尾与在中间都能渲染，往返 `==` 为真 | **已证实**，不需要手写渲染。仍保留 `terms` 为最后一个字段（可读性），并写一条往返单测作为将来 `toml` 升版的护栏 |
| `TextArea` 承载 20,000 字符不卡死 | T-003 的第一件事：无头 `ctx.run()` 跑 20k 字符的 `TextArea` 布局，测单帧耗时；再由用户在真机粘贴 18k 字符确认手感 | 实现第一天出结论；不达标先改实现（限制可见行数/提交时才全文进引擎），仍不达标记 `SPEC_DEVIATION` |
| `Visible(false)` 隐藏 + Dock 恢复 | T-008 先做真机验证；若 eframe 0.37 在关窗时结束 run loop，改走 `CloseRequested` 拦截 | 属实现选择，验收标准不变（FR-5.2 的"进程常驻 + 可恢复 + 空闲 CPU < 1%"） |

### 一处 Spec 事实更正（不改变任何 REQ）

Spec 的 FACT 行称"P3 已提供 `engine::mock::MockEngine`，HTTP 形状另有本地假服务"可供 UI 层离线验证。
实测两者都是 `#[cfg(test)]` 私有的（`crates/ice-providers/src/engine.rs:29`、`lib.rs:11`），
`ice-app` 引用不到；而且 `MockEngine` 只能回固定文本，**无法注入 `ProviderError`**，
REQ-004 的错误映射本来就需要更强的假引擎。因此 `ice-app` 自带一个 `#[cfg(test)]` 的
`StubEngine`（可设回复文本、延迟、错误、调用计数），验证能力是 spec 描述的超集，
`ice-providers` 不改（P3 已 ACCEPTED，不动它的指纹）。实现阶段在变更登记里记这一条。

同一处还有个小笔误：spec 写的 `History::{append,…}` 实际类型名是 `HistoryStore`
（`crates/ice-core/src/history.rs:33`），方法名一致，不影响任何 REQ。

### 一处 Spec 内部张力的裁定

REQ-001 明确"四个页签：文本/图片/历史/设置"（与已封存的 `03-ui-design.md` §2 F-01 一致），
REQ-006 又提到"术语页"。裁定：**术语表作为设置页内的一个分区**，不新增第五个页签；
在 `03-ui-design.md` §2 补一行 F-12（术语分区：列表 + 新增/删除 + 即时生效）并登记变更。
若你更希望它是独立页签，说一声，那要改 REQ-001 的页签枚举，走 `revise` 重批。

## REQ-001 实现与验证

`views/full.rs` 按 `03-ui-design.md` §2 的四区绘制：`Panel::top("toolbar")` 56px 放
F-01~F-05，`CentralPanel` 内用 `SidePanel`/`egui::Grid` 分出 B（源文）与 C（译文）两栏，
`Panel::bottom("status")` 28px 放 F-11。间距用 `ui.spacing_mut().item_spacing = 8.0`、
容器内边距 16px 对齐 §1 的栅格规则；主色/错误红/降级黄在 `theme.rs` 里集中定义，不散落在各 view。

页签状态保留：`AppState.tab` 只决定绘制哪个 view，**各页的数据都常驻 `AppState`**
（源文、译文、语言对、引擎、历史搜索词、术语编辑缓冲），切走再切回不重建、不触发请求。
页签本身不写配置（`Config` 里没有 tab 字段，重启回到文本页）。

图片页（`views/image.rs`）只渲染一段占位说明与 `03-ui-design.md` §2 图片页版式的静态框，
不放任何"未实现"的按钮（避免走查时点到没反应的东西）。

最小尺寸 640×420 由 `ViewportBuilder::with_min_inner_size` 保证；两栏在窄宽下用
`ui.available_width()` 分配，`egui` 的 `ScrollArea` 兜住溢出，不裁切控件。
Tab 顺序即绘制顺序（egui 的焦点遍历按 widget 提交顺序），所以绘制顺序必须与视觉顺序一致：
工具条 → 源文 → 译文 → 按钮行 → 状态条。焦点环用 `ui.visuals_mut().widgets.focused` 的 1px 描边。
字号缩放：每帧把 `config.ui.font_scale` 乘进 `egui::Style::text_style` 的五个字号，改完即时生效。

验证层次：控件存在性、页签切换后 `AppState` 各字段不变、切页签不发起请求（`StubEngine.calls()` 为 0），
用无头 `ctx.run()` + 直接调 view 函数断言；20k/最小尺寸/Tab 顺序/字号观感由用户走查（走查表 W-1、W-3、W-5）。

## REQ-002 实现与验证

提交入口三处（Enter 于源文区、Cmd+Enter 于任意焦点、"翻译"按钮）都汇到 `AppState::submit()`：
`in_flight.is_some()` 时直接返回（不叠加请求）；源文 `trim().is_empty()` 时不发请求、
译文区显示空态文案。`Shift+Enter` 由 `hotkeys.rs` 判定为"换行"而非提交（读 `modifiers.shift`）；
粘贴不提交（egui 的 paste 只改 `TextEdit` 的内容，我们不监听剪贴板变化，FR-3.8 的复制即翻属 P5）。

加载态：`in_flight = Some(id)` 时主按钮画 `ui.spinner()` + "翻译中…" 且 `ui.disable()`；
若 `translation.text` 非空则右上角画一条细 Badge"更新中"，正文不清空。源文区始终可编辑。

结果展示：译文、`elapsed_ms`、`segments`（>1 时显示"分 N 段"）、`cache_hit` 为真时显示"缓存"
且不显示耗时（`elapsed_ms` 此时恒为 0，显示它会误导）。源语言为"自动检测"（`pair.source.is_none()`）
时才回显 `detected`，显式指定源语言则不回显（FR-1.2）。

竞态：`request_id: u64` 单调递增，`drain` 只接受相等的 id。切换引擎或改目标语言走同一条
`submit()` 路径（视为新提交，`remember_target` 同步更新最近 5 种），旧结果自然作废。

20,000 字符：`TextArea` 挂 `ScrollArea`，字数统计用 `chars().count()` 且只在内容变化时算一次
（缓存进 `AppState.char_count`，避免每帧 O(n)）；提交时全文进引擎，分段由引擎层负责（FR-1.9）。

回归：`cargo test --workspace` 里 P3 的 96 项（core 45 + providers 51）必须仍全绿；
本包只加不删（术语改造会调整 core 的 6 项 glossary 测试，项数变化在文档里如实记）。

验证层次：状态机与竞态用 `StubEngine`（设 50ms 延迟 + 可编程回复）在无头断言里跑——
提交两次只保留后一次的结果、`calls()` 计数正确、缓存命中时不打网络；
20k 字符的布局耗时用 `ctx.run()` 实测并记录单帧毫秒数；真机粘贴 18k 字符的手感由用户走查（W-2）。

## REQ-003 实现与验证

`system/clipboard.rs` 里两个纯函数 + 一个副作用函数：

```rust
pub enum CopyKind { Translation, Bilingual }
pub fn compose(kind: CopyKind, source: &str, translation: &str) -> Option<String>  // 空译文 → None
pub fn write(text: &str) -> Result<(), ClipboardError>                              // arboard set_text
```

对照格式固定为"原文段 + 空行 + 译文段"，原文里的换行原样保留（不做 trim、不折行）。
"复制译文"就是 `set_text(译文)`，整体覆盖剪贴板——这正是需求里"一键替换系统剪贴板为译文"的效果，
按用户 2026-10-20 的决定合并为一个按钮，`03-ui-design.md` F-09 的按钮数由 3 改 2。
空译文时两个按钮都不构造文本、不调 arboard，改用 `state.toast("没有可复制的译文")`，
绝不静默清空用户剪贴板里原有的东西。arboard 失败（被占用/系统拒绝）映射成可读文本进提示条，
不 panic、不清空已展示的译文；错误里不含剪贴板原有内容。

验证层次：`compose` 的两种格式、空译文返回 `None`、换行保留都是纯函数单测；
真实剪贴板在真机点一次并粘出来核对（W-2），证据只记"粘贴回来的文本等于译文"这一事实。

## REQ-004 实现与验证

错误文案的唯一来源是 `ice-providers::error` 的 `Display`，UI 层不复制文案、不重新拼语义。
`state.rs` 里一个纯函数把 `ProviderError` 映射成界面呈现：

```rust
pub struct Failure { pub message: String, pub action: Option<FailureAction> }
pub enum FailureAction { GoSettings, UseMachine }
pub fn failure_of(err: &ProviderError) -> Failure
```

只有需要给出动作按钮的变体才 match：`Unauthorized`/`NotConfigured` → `GoSettings`（跳设置页并把
焦点定位到 Key 输入），`Timeout(_, retries)` → `UseMachine`（切机翻并立即重译，文案带上重试次数），
`RateLimited` → 无动作且**不禁用翻译按钮**，其余（`Empty`/`Decode`/`Http`/`Status`/`EmptyInput`/
`Cancelled`）→ 无动作。失败时 `state.translation` 保留上一次成功的译文还是清空？——保留源文是硬要求，
译文区改为红字显示原因（F-08 的行为列），旧的成功译文被错误信息替换但不丢源文；
状态条从"就绪"切到错误态。

隐私：错误文本不进日志全文（只记变体名与长度），源文不进日志、不进窗口标题（P3 的 `scrub_queries`
与"只记长度与哈希前缀"规则继续沿用）。

验证层次：九个变体逐个用 `StubEngine` 注入，无头断言检查渲染出的文本、动作按钮的存在与点击后的
状态变化（`GoSettings` → `tab == Settings`，`UseMachine` → `engine == Machine` 且触发一次新提交），
同一条断言里读回 `state.source` 确认源文完好；真实断网与错 Key 的人工验证在走查里各做一次（W-2）。

## REQ-005 实现与验证

`views/settings.rs` 分四组：AI 服务（供应商下拉 / base_url / model / Key / 清除）、
翻译默认值（默认引擎 / 默认语言对 / 额外语言）、外观（主题三态 / 字号滑杆）、数据（术语分区见 REQ-006、
历史入口见 REQ-006）。任何一项改动即 `state.mark_config_dirty()` → 当帧结束时 `Config::save_at`
（原子替换已有实现），无需"保存"按钮，重启后仍是改后的值。

供应商预设：选 `AiProvider` 时把 `preset_base_url()` / `preset_model()` 填进 `cfg.ai.base_url/model`
的**编辑缓冲**；`effective_base_url()` 的"手填优先"规则由 core 保证，UI 不重复实现。
选 `Custom` 且地址为空 → `is_ready()` 为 false → 状态条显示"AI 未配置"，AI 分段选择在
`EngineKind::Ai` 上禁用并给出原因（`Translator::availability()` 已有这个能力）。
方舟预设 `preset_model()` 为空 → 用 `needs_manual_model()` 显示"请填写模型名/接入点 ID"。

Key 边界（FR-4.2 的硬约束）：输入框用 `TextEdit::password`，**不回显任何片段**（含后 4 位）；
点"保存 Key"→ `secrets::write()` → `keyring::Entry::new("IceTranslator","ai-api-key").set_password`，
成功后只把 `cfg.ai.key_configured = true` 写进 `config.toml`；界面显示"已配置"与"清除"按钮；
日志只出现 `key_len` 与 `sha256` 前 8 位（哈希在 `secrets.rs` 内算，Key 明文不进 tracing 的任何字段）。
"清除 Key"→ 二次确认 → `delete_credential` → `key_configured = false`，机翻通道不受影响。
钥匙串不可用（被锁/权限拒绝/条目损坏）→ 可读错误进提示条，应用继续可用，不 panic，
错误详情里的凭据信息不写日志。Key 明文的生命周期只在内存里：从 `secrets::read()` 出来直接进
`AiEngine::new(cfg, key, timeout, segment_chars)`，`AppState` 不持有它。

主题三态：`theme.rs` 把 `Theme::{System,Light,Dark}` 映射到 `eframe::NativeOptions`/
`egui::Options` 的 `ThemePreference`，运行中改动通过 `ctx.set_theme_preference(...)` 即时生效；
"跟随系统"依赖 `egui-winit` 的 `WindowEvent::ThemeChanged`（框架既有能力，见 spec FACT 行）。
`font_scale` 见 REQ-001。全部文案简体中文，中英混排靠 `Language::breaks_by_char()` 与
`width_factor()`（core 已有）决定换行与宽度估算。

验证层次：三态映射、"手填优先于预设"的取值、`Custom` 空地址时的可用性判定、Key 掩码文案
（断言渲染输出里不含 Key 的任何子串）都是纯逻辑单测；钥匙串真实写/读/删在真机做一次，
取证三件套：条目存在性退出码（`security find-generic-password` 只看退出码，不打印内容）、
`config.toml` 与全部日志 grep 无 `sk-`、界面截图不回显（W-2、走查证据入 `docs/evidence/`）。

## REQ-006 实现与验证

**术语存储改造（core）**：`Config` 增加最后一个字段 `#[serde(rename = "term")] pub terms: Vec<Term>`，
渲染成需求 §4.5 要求的 `[[term]] source="..." target="..."`。`Glossary` 保留为纯逻辑类型
（`upsert/remove/lookup/post_substitute/prompt_fragment/max_prompt_terms` 不动，P3 的行为与测试不变），
持久化职责移到 `config.rs`：

```rust
impl Glossary {
    pub fn from_config(cfg: &Config) -> Self            // cfg.terms.clone()
    pub fn write_to_config(&self, cfg: &mut Config)     // 排序 + 去重后写回 cfg.terms
    pub fn import_legacy(cfg: &mut Config, dir: &Path) -> Result<usize, CoreError>
}
```

`import_legacy` 只在 `cfg.terms.is_empty()` 且 `dir/glossary.toml` 存在时读取（旧的 `[[terms]]` 格式），
返回导入条数；**只读**——不删除、不改写、不重命名用户的 `glossary.toml`。幂等性由
"仅当 config 内无术语时才导入"保证：导入后 `Config::save` 会把术语写进 `config.toml`，
下次启动 `cfg.terms` 非空，不再导入。旧文件损坏 → 返回 0 并给出提示，不阻塞启动。

术语变更即时生效：设置页术语分区里增删改后 → `glossary.write_to_config(&mut cfg)` →
`translator.set_glossary(glossary)`（调度层已有此方法，内部会 bump `glossary_version`，
使 AI 缓存盐值变化、机翻的译后替换立刻用新表）→ `Config::save()`。已缓存结果也走同一后处理，
不重新发请求。

**历史（core 已就绪，UI 接线）**：`views/history.rs` 用 `HistoryStore::open_default()`，
每次翻译成功后 `append(&HistoryRecord { source, target_text, engine, pair, at })`（追加写）；
列表显示 `recent(500)`，搜索框走 `search(query, 500)`（子串匹配），点一条把源文/译文填回
`AppState` 但**不自动提交**；"清空历史"二次确认后 `clear()`。`HISTORY_CAPACITY = 500` 的裁剪在
core 的 `append` 里已实现。文件损坏或缺失 → 历史页显示"历史不可用 + 原因"，翻译照常，不 panic。

验证层次：`Config` 含术语的 TOML 往返、`import_legacy` 的三种输入（正常/损坏/不存在）、
"导入后不重复导入"、"原文件哈希不变"（`tempfile` 造文件后比对）、术语改动让机翻译后替换与
AI 提示词即时变化（`StubEngine` 断言收到的 `glossary_hint`）、历史搜索/裁剪/回显不触发请求
（`calls() == 0`）全部单测；真实文件位置（`~/Library/Application Support/IceTranslator/`）
在真机确认一次并在走查里核对（W-2）。

## REQ-007 实现与验证

`system/hotkeys.rs` 是纯函数：`pub fn action_of(input: &egui::InputState, focus_in_source: bool) -> Vec<Action>`，
`Action` 枚举为 `Translate / ToggleMode / CopyTranslation / ToggleTopmost / HideWindow / Quit / Newline`。
`app.rs` 每帧调一次并按序执行，映射如下：

| 按键 | 动作 | 需求 |
|---|---|---|
| Enter（焦点在源文区，无 Shift） | `Translate` | FR-5.1 |
| Cmd+Enter（任意焦点） | `Translate` | FR-5.1 |
| Shift+Enter | `Newline`（不提交） | FR-1.1 |
| Cmd+Shift+M | `ToggleMode`（保留源文/译文/语言对，不重复请求） | FR-3.3 |
| Cmd+Shift+C | `CopyTranslation`（含空译文保护） | FR-5.1 |
| Cmd+Shift+P | `ToggleTopmost`（持久化到 `cfg.ui.always_on_top`） | FR-3.4 |
| Cmd+W / 窗口关闭按钮 | `HideWindow`（隐藏不退出） | FR-5.2 |
| Cmd+Q | `Quit`（先落盘再退出） | FR-5.2 |
| Esc | 全量模式下无绑定（简洁模式的"清空输入"属 P5） | — |

冲突检查：这五个组合都不是 macOS 系统或常见输入法的保留组合；`Cmd+W`/`Cmd+Q` 走
`ViewportCommand::Close` 的拦截路径而不是自己吞键，保证与系统菜单一致。

关窗常驻：优先 `ViewportCommand::Visible(false)` + 点 Dock 图标时 `Visible(true)`
（eframe 的 `App::ui` 在窗口不可见时是否仍被调用需实测；若不调用则改在 `run_native` 的
事件回调里处理 `WindowEvent::Focused`/激活）。隐藏期间不 `request_repaint()`，
靠 eframe 的空闲等待把 CPU 压到 < 1%。恢复后 `AppState` 原样（源文、译文、页签、语言对都在），
因为进程从未退出。`Cmd+Q` 与 `on_exit` 都先 `Config::save()`（模式、几何、置顶、主题、字号、
默认引擎与语言对、术语）再释放单实例锁。

验证层次：按键→动作映射表逐条无头断言（构造 `egui::RawInput` 的 `events` 与 `modifiers`）、
切换模式后 `AppState` 内容不变且 `StubEngine.calls()` 不增、`HideWindow` 排队的是
`Visible(false)`（沿用 P1 的 `queued(ctx)` 手法）、`Quit` 路径写出了配置文件（`tempfile`）；
"进程常驻 + Dock 恢复 + 空闲 CPU < 1%"必须真机实测（`ps -o %cpu` 取样 30 秒，证据入 `docs/evidence/`）。

## REQ-008 实现与验证

走查表按 `03-ui-design.md` §5 的五步执行，结论逐条写进 `docs/07-acceptance.md` 的新"P4 全量界面"段，
格式沿用 P3 的验收表（验收项 / 依据 / 结论 / 证据）。截图规则不变：整屏采集后用
`sips --cropOffset` 裁到只含本应用窗口矩形，逐张打开确认没有用户其他应用的内容，
裁不干净就改记文本证据；屏幕锁定时不采集。

走查清单（W-1~W-6）：

| 编号 | 内容 | 对应 |
|---|---|---|
| W-1 | 控件清单 F-01~F-12 逐条对照（存在 / 位置符合 / 行为符合三列） | REQ-001 |
| W-2 | 空态、加载态、错误态各一张图（错误态用故意填错的 Key 制造）；粘贴 18k 字符的手感；剪贴板两按钮粘出核对；钥匙串三件套；历史与配置的真实文件位置 | REQ-002~006 |
| W-3 | 缩到 640×420 确认无裁切、无重叠 | REQ-001 |
| W-4 | 逐个快捷键按键验证（含 Cmd+W 隐藏后点 Dock 恢复、双开被拒） | REQ-007、REQ-009 |
| W-5 | Tab 顺序走一遍，确认与视觉顺序一致、无控件不可达、焦点环可见 | REQ-001 |
| W-6 | 亮/暗主题各一张，系统切深浅色后 1 秒内跟随；锁定浅色后系统改深色不变 | REQ-005 |

真实网络只跑两条：机翻真实端点（无需 Key）与 AI 真实通道（用户钥匙串里的 Key），
沿用 P3 的 `crates/ice-providers/tests/live.rs`（`--ignored` 触发），输出脱敏后进证据；
其余全部离线（`StubEngine`）。

文档回写清单：`docs/01-requirements.md` FR-1.5（三动作 → 两按钮）、`docs/03-ui-design.md`
F-09（按钮数 3 → 2）与新增 F-12（设置页术语分区）、`docs/06-test-cases.md` TC-1.5-*、
`docs/traceability.md`（FR-1.1/1.2/1.4/1.5、FR-3.1、FR-4.1/4.2/4.4/4.5、FR-5.1/5.2、需求 §5 单实例、
NFR-4/8 的实现列与结果列）、`docs/02-architecture.md` §2 的 `ice-app` 模块表与 §8 落地表、
`docs/05-dev-guide.md` 的目录约定与测试计数、`docs/07-acceptance.md` 的 P4 段与变更登记
（至少四行：FR-1.5 合并按钮、术语迁入 `config.toml`、单实例落在 P4、Spec 的 MockEngine 事实更正）。
回写完成后 `docs/` 里不得同时存在"待 P4"与"已完成"两种说法。

## REQ-009 实现与验证

`system/single_instance.rs`：

```rust
pub struct InstanceLock { path: PathBuf, pid: u32 }
pub enum AcquireError { AlreadyRunning { pid: u32 }, Io(String) }
pub fn acquire(dir: &Path) -> Result<InstanceLock, AcquireError>  // 锁文件名 ice-translator.lock
impl InstanceLock { pub fn release(self) -> Result<(), AcquireError> }   // Drop 里也调一次，幂等
```

`acquire` 的判定顺序：文件不存在 → 用 `OpenOptions::create_new(true)` 原子创建并写入 PID
（`create_new` 失败说明有并发实例刚建了锁，转"已被占用"分支，避免 TOCTOU 双写）；
文件存在 → 读出 PID，内容不是合法数字视为陈旧锁（可接管）；PID 等于自己视为陈旧锁；
PID 存活（`kill(pid, 0)`，macOS/Linux 用 `libc::kill`，不引新 crate 的话改读 `/proc` 不可行——
macOS 没有 `/proc`，因此用 `unsafe { libc::kill(pid, 0) }`，`libc` 作为 macOS 目标的依赖加入）
→ `AlreadyRunning { pid }`；PID 不存在 → 接管（覆盖写入自己的 PID）。

`main()` 里 `acquire` 失败就 `eprintln!("Ice Translator 已在运行（PID {pid}）")` + `exit(1)`，
**在建窗口之前**，因此第二个实例不会碰 `config.toml` / `history.jsonl` / 钥匙串。
`release` 只在锁文件里的 PID 等于自己时删除（防止删掉接管者的锁），且幂等
（`Drop` 与显式调用都安全）。

不做：跨机器互斥、IPC 把第二个实例的意图转给第一个（点 Dock 图标恢复窗口走 macOS 的应用激活，FR-5.2）、
把它当授权/许可机制用。

验证层次：`tempfile` 造四种锁文件（不存在 / 写入不存在的 PID / 写入自己的 PID / 写入非数字内容）
断言 `acquire` 的四种结果；`release` 两次不报错、且不会删掉别人 PID 的锁；
真机双开一次，证据是第二个进程的退出码非 0 + `config.toml` 的 mtime 不变（W-4）。
