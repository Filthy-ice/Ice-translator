# 需求追踪矩阵

列含义：设计=设计文档章节；实现=代码位置（`crate::module`）；验证=测试用例编号（见 `06-test-cases.md`）；结果=通过 / 带限制通过 / 未做。
本表在 P8 收口时必须无空格；确实不做的行移入"延期项"。

## 文本翻译

| 需求 | 级 | 设计 | 实现 | 验证 | 结果 |
|---|---|---|---|---|---|
| FR-1.1 输入区 ≥20k 字符 | M | 02 §3.1、03 §2 | `ice-app::views::full` | TC-1.1-a、TC-2.1-c | ✅ P4：`twenty_thousand_characters_layout_within_budget` 实测 20000 字符布局 15.6ms，对 100ms 预算成立（CHG-003 的 HYPOTHESIS 已消解） |
| FR-1.2 源语言与自动检测回显 | M | 02 §2 core/detect | `ice-core::detect`、`ice-providers::{machine,ai}::detected`、`ice-app::views::full` | TC-1.2-a/b | ✅ P4 补齐界面回显：`auto_source_uses_local_detection` 等 3 条 + `views::full` 的检测行渲染 |
| FR-1.3 双引擎切换即重译 | M | 02 §2 dispatcher | `ice-providers::{dispatcher,machine,ai}`、`ice-app::state::set_engine` | TC-1.3-a/b/c | ✅ P4：`switching_engine_or_target_resubmits_current_source`；两条真实服务端点此前已实测（机翻 MyMemory、AI DeepSeek） |
| FR-1.4 目标语言与最近使用 | M | 02 §2 core/config | `ice-core::config::remember_target` | TC-1.4-a/b | ✅ 单测 `remember_target_keeps_five_and_most_recent_first` |
| FR-1.5 复制译文 / 复制对照（两个按钮） | M | 03 §2 F-09 | `ice-app::system::clipboard`、`ice-app::views::full::copy` | TC-1.5-a/b、W-1.9 | ✅ P4：两个按钮与 Cmd+Shift+C 共用一条路径；空译文只提示不碰剪贴板（`empty_translation_never_touches_clipboard`、`copy_buttons_refuse_to_touch_the_clipboard_without_a_translation`）。2026-10-19 用户决定由三个按钮合并为两个，见 `07-acceptance.md` 变更行 |
| FR-1.6 不可译内容保护 | M | 02 §3.1 protect | `ice-providers::protect`（被 `machine`/`ai` 在请求前调用） | TC-1.6-a..f | ✅ 单测 14 项：六类片段逐字节保持、多次出现全还原、乱序仍还原、源文自带 `[[0]]` 转义、占位符丢失报失败；真实模型链路另由 `ai_live_deepseek` 实测（URL/`12 MB`/`getUserInfo` 逐字节回来） |
| FR-1.7 失败可诊断不丢源文 | M | 02 §5 | `ice-providers::error`（`from_reqwest`/`from_status`/`retryable`） + `ice-app::state::failure_of` + `views::full` 动作按钮（03 §2 F-13） | TC-1.7-a..e | ✅ P4：`failure_is_mapped_and_source_survives` 断言源文完好；十种错误变体逐个渲染可读文案（`every_error_variant_renders_a_message_and_never_panics`）；日志记变体名 + HTTP 状态码，不记源文/译文/端点（NFR-3）。同语言对（中文源文 + 目标简体中文）在发请求前拦下并给中文文案，不再把端点的英文 `PLEASE SELECT TWO DISTINCT LANGUAGES` 抛给用户（`same_source_and_target_is_refused_before_any_request`、`auto_detected_source_matching_target_is_also_refused`） |
| FR-1.8 缓存命中 | S | 02 §2 cache | `ice-providers::cache` | TC-1.8-a/b | ✅ 单测 `second_identical_request_hits_cache_and_skips_engine` |
| FR-1.9 AI 超长分段 | S | 02 §3.1 | `ice-providers::{ai,protect::chunk_chars}` | TC-1.9-a | ✅ 单测 `long_text_is_split_and_second_segment_gets_context`、`segments_join_in_order_and_keep_the_line_break`、`second_segment_failure_returns_error_not_half_output` |

## 图片翻译

| 需求 | 级 | 设计 | 实现 | 验证 | 结果 |
|---|---|---|---|---|---|
| FR-2.1 四种图片来源 | M | 04 §1 S0 | `ice-app::views::image` | TC-2.1-a..d | 待 P6 |
| FR-2.2 本地 OCR 带坐标与置信度 | M | 04 §1 S1~S3、ADR-002 | `ice-vision::{ocr,vision_sys,layout}` | TC-2.2-a/b + 样图集 | 待 P6 |
| FR-2.3 识别结果可编辑 | M | 03 §2 I-03 | `ice-app::views::image` | TC-2.3-a | 待 P6 |
| FR-2.4 擦除并回填生成副本 | M | 04 §1 S5~S7、ADR-003 | `ice-vision::composite` | TC-2.4-a/b + 人工评分 | 待 P7 |
| FR-2.5 覆盖层模式 | M | 04 §3、ADR-003 | `ice-vision::bg` + `composite` | TC-2.5-a/b | 待 P7 |
| FR-2.6 导出 PNG/剪贴板 | M | 04 §1 S8 | `ice-app::views::image` | TC-2.6-a/b | 待 P7 |
| FR-2.7 字号自适应与降级 | S | 04 §4 | `ice-vision::composite` | TC-2.7-a | 待 P7 |
| FR-2.8 颜色与对齐还原 | S | 04 §3 | `ice-vision::bg` | TC-2.8-a | 待 P7 |
| FR-2.9 阶段进度与取消 | S | 04 §1 状态机 | `ice-app::jobs` | TC-2.9-a/b | 待 P7 |
| FR-2.10 区域截图 | C | — | — | — | 延期项（立项书 §3） |

## 双界面

| 需求 | 级 | 设计 | 实现 | 验证 | 结果 |
|---|---|---|---|---|---|
| FR-3.1 全量模式布局 | M | 03 §2 | `ice-app::views::{mod,full,history,settings,image}` | 走查表 W-1、TC-3.1-a/b | ✅ P4：四页签 + 顶部工具条 + 状态条，最小尺寸下逐页渲染不 panic（`every_tab_renders_without_panicking_at_the_minimum_size`）；控件清单较封存版多出 F-12 翻译按钮、F-13 失败动作按钮、F-14 术语分区，已回写 `03-ui-design.md` |
| FR-3.2 简洁模式窄条（含无边框可拖动） | M | 03 §3 K-06 | `ice-app::views::compact`（`StartDrag`） | 走查表 W-2、W-2.13 | 待 P5（P4 只做了切换时的 `ViewportCommand` 编排：`switching_to_compact_queues_a_borderless_small_window`） |
| FR-3.3 切换保留内容 | M | 03 §4、ADR-001 | `ice-app::mode` | TC-3.3-a | 部分 ✅ P4：内容与源/目标语言在切换中不动且不触发请求（`toggling_mode_keeps_content_and_marks_config_dirty`、`mode_toggle_preserves_content_and_does_not_request`）；切换几何见 FR-3.9，待 P5 |
| FR-3.4 置顶 | M | 03 §3 K-09、ADR-001 | `ice-app::mode`（`WindowLevel`） | TC-3.4-a | ✅ P4：`topmost_maps_to_window_level_and_is_persisted`，并在设置页留了手动开关（NFR-4 同源决定：跟随系统之外必须有手动项） |
| FR-3.5 边缘吸附与脱离 | M | 03 §3 吸附几何 | `ice-app::mode::snap`（纯函数） | TC-3.5-a..d | 待 P5 |
| FR-3.6 记忆位置尺寸与越界回正 | M | 02 §3.3、§6 | `ice-core::config` + `ice-app::mode` | TC-3.6-a/b | 部分 ✅ P4：`window_off_screen_detection` + 几何捕获三测（`capture_takes_position_from_outer_and_size_from_inner`、`capture_ignores_the_first_frame_where_geometry_is_unknown`、`initial_viewport_follows_the_saved_mode`）。**其中修掉一个真实缺陷**：外框缺失时曾退回用内框原点，会把标题栏高度算进保存的位置，每启动一次窗口下移一截 |
| FR-3.7 失焦半透明 | S | 03 §3 K-08 | `ice-app::mode` | 走查 W-3 | 待 P5 |
| FR-3.8 剪贴板即翻 | S | 03 §4 | `ice-app::system::ClipboardWatcher` | TC-3.8-a | 待 P5 |
| FR-3.9 切换瞬时几何 | M | 03 §4 过渡 | `ice-app::mode`（同帧一次性下发 `Decorations`/`InnerSize`/`OuterPosition`） | 走查 W-2.14 | 待 P5。**2026-10-22 需求变更**：由"180~220ms 形变动画"改为瞬时切换，`mode::Morph` 不实现（原因见本节末条与 `features/CHG-004/evidence/T-001-hypothesis-verdicts.md`） |

## 配置、历史与术语

| 需求 | 级 | 设计 | 实现 | 验证 | 结果 |
|---|---|---|---|---|---|
| FR-4.1 设置页各项持久化 | M | 02 §6 | `ice-core::config`、`ice-app::views::settings` | TC-4.1-a/b | ✅ P4：设置页改任一项即标脏、退避 500ms 落盘（`config_writes_are_debounced_but_a_forced_save_always_happens`、`a_failed_save_keeps_the_dirty_flag_for_the_next_try`）；供应商预设与手填值互不覆盖（`provider_preset_fills_the_edit_buffers`、`manual_values_survive_a_preset_selection_and_still_win`） |
| FR-4.2 密钥入钥匙串且不入日志 | M | ADR-005 | `ice-app::system::secrets`、`ice-app::app::spawn_key_probe` | TC-4.2-a/b、TC-4.3-d | ✅ P4：明文只交出一次并随即清空缓冲（`saving_a_key_hands_over_the_plaintext_once_and_clears_the_buffer`），空 Key 永不写出，界面只显示位数不显示任何片段（`a_configured_key_shows_only_its_length`），错误文本不回显密钥字节（`describe_error_never_echoes_payload_bytes`）。**启动时读钥匙串挪到后台线程**：macOS 会弹授权框阻塞 `get_password()`，放在建窗之前表现为"启动卡死"（见 `07-acceptance.md` 变更行） |
| FR-4.3 配置目录规范与损坏容错 | M | 02 §6 | `ice-core::config::load_at` | TC-4.3-a/b | ✅ 单测 `load_at_falls_back_to_defaults_with_reason` 等 3 条 |
| FR-4.4 历史 500 条可搜可回显 | S | 02 §2 history | `ice-core::history` + `ice-app::views::history` | TC-4.4-a..d、TC-6.1-a/b | ✅ P4 补齐界面：点一行只回填文本页、不自动翻译（`picking_a_row_fills_the_text_page_without_translating`）；渲染本身不提交请求；坏行跳过整页仍可用，读不出来给可读原因（TC-6.1-b 两条） |
| FR-4.5 术语表即时生效 | S | 02 §2 glossary | `ice-core::glossary` + `config`（`[[term]]`）+ `dispatcher` | TC-4.5-a..g | ✅ P4：**存储迁到 `config.toml` 的 `[[term]]`**（Q-003 决定），旧 `glossary.toml` 一次性只读导入且不删除（`legacy_glossary_is_imported_once_and_the_config_is_marked_for_saving`、`the_legacy_file_is_never_touched`）；启动即推给派发器（`glossary_from_config_reaches_the_engine_on_the_first_job`），引擎重建后重灌（`rebuilding_the_translator_keeps_the_glossary`） |

## 系统集成与非功能

| 需求 | 级 | 设计 | 实现 | 验证 | 结果 |
|---|---|---|---|---|---|
| FR-5.1 应用内快捷键 | M | 03 §4 | `ice-app::system::hotkeys` | 走查 W-4、TC-5.1-a | ✅ P4：Cmd+Enter 翻译、Cmd+Shift+M/C/P 全局、Cmd+W 隐藏、Cmd+Q 退出；按住不重复触发；裸 Enter 只在源文框有焦点时提交，Shift+Enter 始终是换行（`system::hotkeys::tests` 8 条） |
| FR-5.2 关窗不退出 | M | 03 §4 | `ice-app::mode::hide`（`CancelClose` + `Minimized(true)`） | TC-5.2-a/b | ✅ P4：进程不退、内容原样（`hide_window_keeps_the_process_and_the_content`）；Cmd+Q 先落盘再释放单实例锁（`quit_writes_the_config_before_releasing_the_lock`）。**与封存设计的偏差**：设计写 `Visible(false)`，eframe 0.37 在 macOS 上不投递 `Reopen`，隐藏后点 Dock 图标叫不回来，改用"取消关闭 + 最小化"，详见 `07-acceptance.md` §5；Dock 恢复待真机走查 |
| FR-5.3 屏变化后状态正常 | S | 02 §3.3 | `ice-app::mode` | TC-5.3-a | 待 P5 |
| 需求 §5 第 5 条 单实例运行 | M | CHG-003 REQ-009（Q-001 归属裁定） | `ice-app::system::single_instance` | TC-9.1-a/b | ✅ P4：另一进程持锁时非零码退出且不碰配置/历史/钥匙串；陈旧锁（PID 已消失、内容损坏、PID 被回收）一律接管；释放时只删 PID 与自己相符的那一份（`release_does_not_remove_someone_elses_lock`）。**真机已验**：kill 掉旧实例后重启，陈旧锁被自动接管（`/tmp/ice-live2.log`）；双开走查见 W-1.19 |
| NFR-1 性能指标 | M | 05 §4 | `ice-app::jobs` + 采样脚本；引擎层实测样本记录在 `ADR-004` 与 `features/CHG-002/evidence.md` | TC-P-1..3、TC-2.1-c/d | ⚠ 待 P8 判定：机翻实测单句 731–2133ms、1060 字符 3 段 1.9–3.2s，P50<800ms 的目标**尚未确认达标**（免费端点抖动大），需 P8 的 20 次采样脚本判定；AI 单句实测 1.1–1.2s。P4 新增的 UI 侧数据点已达标：20k 字符单帧布局 15.6ms（预算 100ms）、慢任务不阻塞 UI 线程（TC-2.1-d） |
| NFR-2 无 panic | M | 02 §5 | 全 crate | TC-P-4、TC-N-1 | 部分 ✅ 引擎层无输入可触发的 panic：`expect` 只用于字面量正则与非零容量；曾真实发生的切块字符边界 panic 由 `chunks_respect_char_limit_and_order` 锁死。P4 新增：四页签在最小尺寸下逐页渲染不 panic、11 个错误变体逐个渲染不 panic、历史里的控制字符压平而非直接渲染（`control_characters_are_flattened_not_rendered`）。图像流水线待 P6/P7 |
| NFR-3 隐私 | M | ADR-002/005 | `ice-vision`、`ice-app::system::secrets`、`ice-app::jobs` 日志 | TC-P-5 | 部分 ✅ Key 不进 `config.toml`/`Debug`/错误文本（`debug_output_hides_the_key`、`toml` 序列化断言），日志只记长度与哈希前 8 位（`fingerprint_is_stable_and_short`），错误文本剔除查询串（`query_string_is_scrubbed_from_error_text`），源文与译文一律不进日志。P4 新增：失败日志在变体名之外补记 HTTP 状态码——状态码不是秘密（响应体已在 `from_status` 里截断），只记 `error="Status"` 会让线上故障无从下手（`status_code_is_logged_for_http_failures_only`）。OCR 本地性与整体日志审查待 P6/P8 |
| NFR-4 中文界面·主题跟随系统+三态开关 | M | 03 §1 | `ice-app::theme` | TC-P-9 | ✅ P4：三态（跟随系统/浅色/深色）映射到 `ctx.set_theme`（`theme_maps_to_the_three_preferences`）；字号缩放乘在 egui 默认值上、限幅且逐帧幂等（`font_scale_is_clamped_to_the_legal_range`、`font_scale_is_idempotent_across_frames`）；四种提示条各有自己的颜色。界面文案走查见 W-1.17 |
| NFR-5 资源占用 | S | 05 §1 | release 构建 | TC-P-6 | 待 P8（P4 已做到空闲帧不请求重绘，30s CPU 采样见 W-1.20） |
| NFR-6 分层依赖 | M | ADR-006 | workspace | `cargo tree` 检查 | 部分 ✅ P1 已核（core 依赖树无 egui/eframe/reqwest/image），P8 三目标复核。P4 未引入反向依赖：`ice-app` 单向依赖 `ice-core`/`ice-providers` |
| NFR-7 跨平台可用与降级 | S | ADR-002 | `ice-vision::native_ocr_supported` | TC-P-7 | 待 P8。P4 有一处跨平台取舍已就地标注：历史页的本地时区格式化在 unix 走 `localtime_r`、其他平台退回 UTC，`i64`/`i32` 的 `time_t` 差异保留转换并对本平台的 `useless_conversion` 告警放行 |
| NFR-8 可访问性：字号缩放与键盘可达 | C | 03 §1、§5 | `ice-app::theme`（字号）+ `system::hotkeys`（键盘） | 走查 W-1、§5 第 5 步 | 部分 ✅ P4：字号缩放 0.85–1.6 倍可调并落盘；全部快捷键不依赖鼠标（TC-5.1-a）；等宽字体族在缩放后仍是等宽（`monospace_keeps_its_font_family`）。Tab 顺序与读屏需真机走查 |

## 统计

- 需求共 44 条：**Must 30 · Should 12 · Could 2**（按 `01-requirements.md` 逐行重新计数得出；此前记的 27/16/2 是 P0 时的旧值，FR-3.9 加入后未重算，已在 P2 自检时更正）。需求 §5 第 5 条"单实例运行"不带 FR 编号，P4 起单列一行追踪。
- 骨架阶段（P1）已闭环的 Must：FR-3.4 的置顶能力、FR-4.3 全部、FR-3.6 的几何判定部分、FR-1.4 的一部分（语言模型与持久化）。
- 2026-10-19 需求变更（用户在 P1 走查后提出）：FR-3.2 明确"无边框必须仍可拖动"、新增 FR-3.9 形变动画、NFR-4 改为默认跟随系统深浅色并保留三态开关。三项均落在 P4/P5。
- **P4（CHG-003）闭环**：FR-1.1、FR-1.2、FR-1.3、FR-1.5、FR-1.7、FR-3.1、FR-3.4、FR-4.1、FR-4.2、FR-4.4、FR-4.5、FR-5.1、FR-5.2、NFR-4、需求 §5 第 5 条，共 15 条转为 ✅；FR-3.3、FR-3.6、NFR-2、NFR-8 由"待 P4"转为部分闭环（剩余部分属 P5 或真机走查）。仍未闭环的 Must：FR-2.1~FR-2.6（P6/P7）、FR-3.2、FR-3.5、FR-3.9（P5）。
- 2026-10-20 P4 实测新发现并修掉的两个真实缺陷：① 启动路径上同步读钥匙串，被 macOS 授权框阻塞时窗口建不出来（表现为"启动卡死"）；② `config.toml` 里的术语从未推给派发器，且每次引擎重建都会丢掉。另有一个由用户真机操作暴露的第三个缺陷：源语言与目标语言相同时机翻端点返回英文 `PLEASE SELECT TWO DISTINCT LANGUAGES`，现改为发请求前就地拦下并给中文可读文案（新增 `ProviderError::SameLanguage`）。三者详见 `07-acceptance.md` 变更行。
- **2026-10-21 P4 走查四轮收口（CHG-003 → VERIFIED）**：真人走查跑出并修掉五个界面缺陷 D-1（底部动作行被裁出面板）、D-2（Cmd+W 后 Dock 恢复回来一张拉伸贴图，修了三轮）、D-3（两栏底边不齐）、D-4（18,000 字符粘贴把源文框撑出窗口且滚不动）、D-5（装饰字形在真机上显示为豆腐块）；W-1.1~W-1.7、W-1.9~W-1.24 共 23 项通过，V-001~V-012 十二项验证全部有证据。**三组转出项**：① 简洁窗尺寸/拖动/引擎切换/译文滚动四条属规格明划给 P5 的内部版式，转 CHG-004（新增走查项 W-2.15/W-2.16）；② 用户决定 FR-1.2/FR-1.4 增加"自动检测撞到目标语言时自动反向并改目标语言"，转 CHG-004（新增 TC-1.2-c，本包不改行为）；③ W-1.8 的假 Key 分支未在真机复现（会覆盖用户真 Key），转 P8 用废 Key 补。**这五条界面缺陷都不改任何 REQ 编号**，但 D-4/D-5 说明了一件事：界面层测试只看"做了什么"（`Effects` 断言）与"光标推进到哪"（`min_rect`），看不见丢弃子 Ui 里的溢出，也看不见真机字形覆盖——前者已用滚动容器实测数据补上断言，后者无头环境无法验证，只能靠真人走查。
- **2026-10-22 FR-3.9 需求变更（CHG-004 实测后用户决定）**：形态切换由"180~220ms 形变动画"改为**瞬时切换**，Must 级别与编号不变（需求总数仍 44 · Must 30）。经过：CHG-004 T-001 用一次性探针 `crates/ice-app/examples/viewport_probe.rs` 实测三条假设 —— 逐帧 `InnerSize` 补间**不成立**（200ms 内只有 4~5 次请求落到窗口，三次复现 ~1.0s 事件循环停摆，用户判定"一卡一卡的，ppt 了"），设计里写的第一级降级（30fps）同样不成立；替代方案"窗内补间"（窗口先变成两端外接矩形、只插值画在窗口内的内容矩形）**实测成立**（204ms / 14 帧真 60fps，端点误差 0px，用户判定"很丝滑"），但用户看过效果后决定"算了不要动画了，都做成瞬间的"。连带改动：`01-requirements.md` FR-3.9、`03-ui-design.md` §4、`02-architecture.md` §3.3、`06-test-cases.md` W-2.14；`ice-app::mode::Morph` 不再实现。证据：`features/CHG-004/evidence/T-001-hypothesis-verdicts.md` 与探针日志 `T-001-probe{,-v2,-v3}.log`。
