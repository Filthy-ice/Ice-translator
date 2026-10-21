# 测试用例与走查表

编号规则：`TC-<需求号>-<字母>` 为自动化或半自动用例；`W-<序号>` 为人工界面走查；`TC-P-<n>` 为非功能用例；`IMG-<n>` 为图片样张。
每条用例写清**前置 / 步骤 / 预期**。执行结果不写在这里，写在 `07-acceptance.md` 与 `traceability.md`，本文件是稳定的用例库。

## 1. 自动化用例（`cargo test`）

### 文本翻译

| 编号 | 需求 | 步骤 | 预期 | 状态 |
|---|---|---|---|---|
| TC-1.2-a | FR-1.2 | 对中文、英文、日文、韩文、俄文各一句调用 `ice_core::detect`（原名 `guess_language`） | 五种均判对；纯数字/符号句返回 `None` 而非误判 | ✅ 单测 `detects_common_scripts`、`digits_symbols_and_empty_are_not_guessed`、`japanese_with_kanji_still_reads_as_japanese` |
| TC-1.2-b | FR-1.2 | 混合中英的段落 | 按主导字符集判定，且置信度低于纯单语句 | ✅ 单测 `mixed_chinese_english_reads_as_chinese_with_lower_confidence` |
| TC-1.2-c | FR-1.2 / FR-1.4 | **自动检测出的语言与目标语言相同**（源=自动检测、目标=英语、源文为英文） | 目标语言自动翻转到简体中文并**持久写回**（工具条下拉跟着变），提示条说明"检测到英语，已自动改为 英语 → 简体中文"，本次直接出译文；若目标本来就是简体中文则翻转到英语；源语言仍保持自动检测 | ☐ **未实现，转 CHG-004**（用户 2026-10-21 决定）：当前行为是本地拦下并给中文可读错误 `SameLanguage`（`machine.rs:164`），真机复现为"源语言与目标语言相同（都是英语），不需要翻译；请换一个目标语言"。这是对 FR 行为的改动，不在已批准的 CHG-003 范围内 |
| TC-1.3-a | FR-1.3 | `Translator` 注册两个 mock 引擎，分别请求 | 两通道返回不同文本，各自 `engine` 字段正确 | ✅ `different_engines_do_not_share_cache_entries` |
| TC-1.3-b | FR-1.3 | 真实网络（`#[ignore]`）：同一句分别走机翻与 AI | 两条均返回非空译文，且 `elapsed_ms` 均 > 0 | ✅ 两条真实通道均已实测：`machine_live_short`、`machine_live_long_text_is_chunked`、`ai_live_deepseek`（`cargo test -p ice-providers --test live -- --ignored`） |
| TC-1.3-c | FR-1.3 | AI 未配置 Key 时请求 AI 通道 | `ProviderError::NotConfigured`，错误文本含"未配置" | ✅ `unregistered_engine_reports_not_configured_for_ai`、`unavailable_without_key_or_config` |
| TC-1.4-a | FR-1.4 | 连续切换 6 种目标语言 | `recent_targets` 长度为 5，最新在前 | ✅ `remember_target_keeps_five_and_most_recent_first` |
| TC-1.4-b | FR-1.4 | 设置中启用德语后重启 | `visible_languages()` 含德语与中英日，无重复 | ✅ `visible_languages_always_includes_preferred` |
| TC-1.6-a | FR-1.6 | 输入含 `https://example.com/a?b=1` 的句子 | 译文中原样保留该 URL，逐字节一致 | ✅ 单测 `protects_the_declared_kinds_of_untranslatable_content`、`identity_roundtrip_keeps_every_byte`；引擎侧 `protected_content_survives_a_service_that_mangles_numbers` |
| TC-1.6-b | FR-1.6 | 含邮箱、`userId`、`12 MB`、`2026-10-19`、```` ``` ````代码块 | 五类片段均未被翻译或改动 | ✅ 同上（六类片段各构造一例）+ `same_identifier_is_restored_everywhere`（同一标识符出现多次全部还原） |
| TC-1.6-c | FR-1.6 | 服务端返回时把占位符改丢掉或改写（`[[n]]` 在译文中消失） | **报失败**：`restore` 返回的 `lost` 非空 → `ProviderError::Decode`，绝不返回残缺文本，也不静默退回"不保护"路径（预期由 CHG-002 spec REQ-003 定；本条原写"退回不保护并返回原文"，与 spec 冲突，已按 spec 更正） | ✅ 单测 `missing_placeholder_is_reported_as_lost`、`missing_placeholder_from_model_is_a_failure`；另 `reordered_placeholders_still_restore`、`existing_bracket_placeholder_in_source_is_escaped` 覆盖按编号匹配与源文自带 `[[0]]` 的转义 |
| TC-1.6-d | FR-1.6 | 代码块内含 URL | 整块作为一个保护单元，块内不再二次占位 | ✅ 单测 `fenced_code_block_is_protected_as_a_whole` |
| TC-1.7-a | FR-1.7 | 断开网络后请求 | `Timeout`/`Http` 错误，源文仍在输入框 | 部分 ✅ 单测 `timeout_is_reported_with_retry_count`（Timeout 带超时毫秒与重试次数）、`connection_refused_says_unreachable_not_rejected`（区分"不可达"与"被拒绝"）；"源文仍在输入框"属界面行为，待 P4 走查 |
| TC-1.7-b | FR-1.7 | 填错误 Key 请求 AI | `Unauthorized`，提示含"Key 无效"，界面出现"去设置"动作 | 部分 ✅ 单测 `auth_and_rate_limit_map_to_distinct_errors`（401 → `Unauthorized`，不重试）；"去设置"按钮待 P4 |
| TC-1.7-c | FR-1.7 | 服务端返回 200 但译文为空 | `Empty` 错误，界面显示原文 + 警告 | 部分 ✅ 单测 `empty_translation_is_a_failure_not_empty_success`、`body_level_error_is_not_mistaken_for_translation`；界面告警待 P4 |
| TC-1.8-a | FR-1.8 | 同一请求连续两次 | 第二次 `cache_hit=true`、`elapsed_ms=0`、引擎调用次数不变 | ✅ `second_identical_request_hits_cache_and_skips_engine` |
| TC-1.8-b | FR-1.8 | 改目标语言后请求同一句 | 缓存未命中（键含语言对） | ✅ `cache.rs` LRU 用例 + 键定义 |
| TC-1.9-a | FR-1.9 | 12,000 字符输入走 AI 通道 | 分段数 = ⌈长度/3500⌉（3500 为 `config.translation.ai_chunk_chars` 默认值），拼接顺序与原文段落顺序一致，段落间无重复或丢失 | ✅ 单测把上限注入为小值以逼出多段（机制与 12k 一致，不靠魔法数字）：`long_text_is_split_and_second_segment_gets_context`（第二段拿到上一段译文尾巴）、`segments_join_in_order_and_keep_the_line_break`（按序相接、补回被模型吞掉的换行）、`second_segment_failure_returns_error_not_half_output` |
| TC-1.9-b | FR-1.9 | 分段中间含代码块 | 占位符保护在分段之前执行，跨段占位符不冲突 | ✅ 单测 `chunk_never_splits_a_placeholder`、`oversized_code_block_never_reaches_the_chunk_limit`、`no_near_empty_chunks`、`prefer_line_break_over_hard_cut` |

### 配置、历史与术语

| 编号 | 需求 | 步骤 | 预期 | 状态 |
|---|---|---|---|---|
| TC-4.1-a | FR-4.1 | `Config` 全字段序列化后反序列化 | 与原值逐字段相等 | ✅ `roundtrip_keeps_values` |
| TC-4.1-b | FR-4.1 | 只写部分段落的 TOML | 缺失段落取默认值，已写段落生效 | ✅ `partial_config_uses_defaults_for_missing_sections` |
| TC-4.3-a | FR-4.3 | 写入非法 TOML 后 `load_at` | 返回默认配置 + 含"解析失败"的原因，不 panic | ✅ `load_at_falls_back_to_defaults_with_reason` |
| TC-4.3-b | FR-4.3 | 越界数值（字号 99、阈值 0.1、窗口宽 10） | `sanitized()` 夹回合法区间 | ✅ `sanitize_clamps_out_of_range_values` |
| TC-4.3-c | FR-4.3 | 保存到不存在的子目录 | 自动建目录，无残留 `.tmp` 文件 | ✅ `save_at_creates_dirs_and_leaves_no_temp_file` |
| TC-4.4-a | FR-4.4 | 写入 530 条历史 | 只保留最近 500 条，最新在前 | ✅ `trims_to_capacity_keeping_newest` |
| TC-4.4-b | FR-4.4 | 搜索关键词（大小写混写） | 命中原文或译文任一即可，不区分大小写 | ✅ `search_matches_both_sides_case_insensitively` |
| TC-4.4-c | FR-4.4 | 历史文件里插入一行非法 JSON | 该行跳过，其余可读 | ✅ `corrupt_line_is_skipped_not_fatal` |
| TC-4.4-d | FR-4.4 | 清空历史 | 文件存在但为空，读取返回 0 条 | ✅ `clear_empties_history` |
| TC-4.5-a | FR-4.5 | 机翻通道加术语后重查同句 | 命中缓存不重复请求，但译文按术语替换 | ✅ `glossary_applies_to_cached_result_without_new_request` |
| TC-4.5-b | FR-4.5 | AI 通道请求 | 提示词含"原文 => 译法"行 | ✅ `ai_channel_receives_glossary_hint` |
| TC-4.5-c | FR-4.5 | 术语超过 60 条 | 注入提示词的条目被截断到上限 | ✅ `prompt_fragment_caps_at_limit` |
| TC-4.5-d | FR-4.5 | 中文术语大小写无关替换、多字节边界 | 不产生非法 UTF-8、不切半字符 | ✅ `substitute_handles_cjk_and_repeated_occurrences` 等 |

### 窗口几何

| 编号 | 需求 | 步骤 | 预期 | 状态 |
|---|---|---|---|---|
| TC-3.1-a | FR-3.1 | 无头调用 `apply_mode(Compact)`，检查排队的 ViewportCommand | 无边框 + 不可缩 + 460×96 + 落到保存的位置 | ✅ `switching_to_compact_queues_borderless_small_window` |
| TC-3.1-b | FR-3.1 | 无头调用 `apply_mode(Full)` | 恢复边框与可缩放，尺寸回到 `full_rect` | ✅ `switching_back_to_full_restores_decorations_and_size` |
| TC-3.3-a | FR-3.3 | 置 `always_on_top` 后 `apply_topmost` | 排队 `WindowLevel::AlwaysOnTop`，且写回 config | ✅ `topmost_maps_to_always_on_top_window_level` |
| TC-3.6-a | FR-3.6 | 记录的窗口位置在已拔掉的外接屏上 | `within()` 判为 false → 回主屏中心 | ✅ `window_off_screen_detection` |
| TC-3.5-a | FR-3.5 | 松手位置距右边缘 30px（阈值 40） | 停靠到右边，x = area.right − w + sliver | 待 P5（纯函数单测） |
| TC-3.5-b | FR-3.5 | 距左/上/下边缘各 30px | 分别停靠对应边 | 待 P5 |
| TC-3.5-c | FR-3.5 | 距四边均 > 40px | `DockEdge::None`，位置不变 | 待 P5 |
| TC-3.5-d | FR-3.5 | 停靠状态下再拖离 200px | 恢复自由浮动，热区消失 | 待 P5 |
| TC-3.5-e | FR-3.5 | 窗口高度大于屏幕可用高度 | 不产生负坐标，夹在可视区内 | 待 P5 |

### 界面层（P4，`cargo test -p ice-app`）

| 编号 | 需求 | 用例 | 期望 | 状态 |
|---|---|---|---|---|
| TC-1.1-a | FR-1.1 | 无头渲染文本页后检查源文框的 `return_key` | 裸 Enter 提交翻译、Shift+Enter 只换行；粘贴不自动提交 | ✅ `plain_enter_translates_only_when_focus_is_in_source`、`shift_enter_is_a_newline_even_with_focus_in_source` |
| TC-1.5-a | FR-1.5 | 有译文时点「复制译文」/「复制对照」 | 前者只写译文，后者写"原文+分隔线+译文"，保留源文换行 | ✅ `translation_copy_writes_only_the_translation`、`bilingual_copy_keeps_source_newlines_and_fixed_separator` |
| TC-1.5-b | FR-1.5 | 译文为空时点任一复制按钮 | 剪贴板**完全不被触碰**，只出一条提示 | ✅ `empty_translation_never_touches_clipboard`、`copy_buttons_refuse_to_touch_the_clipboard_without_a_translation` |
| TC-1.7-d | FR-1.7 | 逐一注入 10 个 `ProviderError` 变体 | 每个都有可读文案、不 panic、源文保留 | ✅ `every_error_variant_renders_a_message_and_never_panics`、`failure_is_mapped_and_source_survives` |
| TC-1.7-e | FR-1.7 | 401 → 点「去设置页填 Key」；AI 超时 → 点「改用机翻」 | 前者切到设置页并聚焦 Key 输入框，后者用机翻重发同一句 | ✅ `failure_action_go_settings_switches_tab_and_focuses_the_key_field`、`failure_action_use_machine_resubmits_with_the_machine_engine`、`timeout_on_ai_offers_machine_fallback_but_not_on_machine` |
| TC-2.1-a | REQ-002 | 结果回来时 `request_id` 已变（用户改了源文/语言） | 过期结果静默丢弃，界面不闪旧内容 | ✅ `stale_outcome_is_dropped_without_touching_the_ui`、`stale_outcome_is_dropped_without_touching_visible_text` |
| TC-2.1-b | REQ-002 | 请求在飞时再次提交 | 忽略第二次，同一时刻只有一个在飞 | ✅ `submit_while_in_flight_is_ignored` |
| TC-2.1-c | REQ-002 | 20,000 字符中英混排的单帧布局耗时 | < 100ms（**实测 15.6ms**，REQ-002 的 HYPOTHESIS 成立） | ✅ `twenty_thousand_characters_layout_within_budget` |
| TC-2.1-d | REQ-002 | 慢任务（人为延迟）期间的帧循环 | UI 线程不阻塞，结果晚到也能落盘 | ✅ `a_slow_job_does_not_block_the_ui_thread` |
| TC-4.2-a | FR-4.2 | 保存 Key / 空 Key / 已配置态的界面呈现 | 明文只交出一次且随即清空缓冲区；空 Key 永不写出；界面只显示位数，不显示任何片段 | ✅ `saving_a_key_hands_over_the_plaintext_once_and_clears_the_buffer`、`an_empty_key_is_never_handed_over`、`a_configured_key_shows_only_its_length`、`fingerprint_is_stable_and_hides_the_key`、`describe_error_never_echoes_payload_bytes` |
| TC-4.2-b | FR-4.2 | 启动时钥匙串读取被系统授权框阻塞 | 窗口**先**建好，读取在后台线程；结果回来后补装 AI 引擎并请求重绘 | ✅ `the_probe_result_actually_arrives_over_the_channel`、`a_pending_key_probe_leaves_ai_unavailable_until_it_lands`；真机 W-1.18 已过（2026-10-20，日志时序取证）：建窗与取锁在 `09:24:49.513`，钥匙串回值在 `09:25:07.796`——中间 18.3s 是用户在回答 macOS 授权框，窗口全程可交互；证据 `features/CHG-003/evidence/*-V-010.txt`（取编号最大者） |
| TC-4.3-d | FR-4.3 | 钥匙串里条目被手动删除，config 仍标记已配置 | 以钥匙串为准清掉标记并落盘 | ✅ `the_probe_overrides_a_stale_configured_flag`、`a_stale_key_configured_flag_is_cleared` |
| TC-4.5-e | FR-4.5 | `config.toml` 里已有术语，启动后第一次派发 | 术语提示词已经在请求里，不需要用户先改一次术语 | ✅ `glossary_from_config_reaches_the_engine_on_the_first_job` |
| TC-4.5-f | FR-4.5 | 设置页增/删术语 | 同帧把新术语表推给派发器，并标脏落盘 | ✅ `adding_a_term_pushes_the_new_glossary_to_the_dispatcher`、`removing_a_term_also_syncs_and_dirties_the_config`、`glossary_change_reaches_the_engine_prompt` |
| TC-4.5-g | FR-4.5 | 换供应商/存 Key 触发引擎整套重建 | 重建后术语表仍然生效（不随旧实例丢失） | ✅ `rebuilding_the_translator_keeps_the_glossary`、`replacing_the_translator_takes_effect_on_the_next_job` |
| TC-5.2-a | FR-5.2 | Cmd+W / 点窗口关闭按钮 | 撤销关闭并最小化，进程不退、内容原样 | ✅ `hide_window_keeps_the_process_and_the_content`、`hide_cancels_close_and_minimizes`；⚠ **真机两轮失败（缺陷 D-2，见 W-1.21）**：收进后台没问题，点 Dock 恢复回来的是一张被拉伸的旧画面、完全不能交互。无头测试看不出来，因为它手工把 `minimized` 塞进 `RawInput`，而真机上 egui-winit 在 macOS 运行时根本不刷新这个字段；第二轮才查出更硬的一层——隐藏期间 eframe 只调 `App::logic`，写在 `ui()` 里的修复代码从未执行。恢复判据现由 4 条 `RevealGuard` 测试锁住（静默期内不判、焦点单独成立、几何单独成立、可见时不成立）；**第三轮真机复验通过**（2026-10-21，用户确认"收起叫回后依然能够使用"），FR-5.2 成立 |
| TC-5.2-b | FR-5.2 | Cmd+Q | 先落盘配置再关窗，最后才释放单实例锁 | ✅ `quit_writes_the_config_before_releasing_the_lock`、`quit_sets_the_flag_and_really_closes` |
| TC-5.1-a | REQ-007 | 逐条按键分类 | Cmd+Enter 翻译、Cmd+Shift+M/C/P 全局、Cmd+W 隐藏、Cmd+Q 退出；按住不重复触发；未绑定键放行 | ✅ `system::hotkeys::tests` 全部 8 条 |
| TC-6.1-a | REQ-006 | 历史页渲染与点选 | 渲染本身不提交任何请求；点一行只回填到文本页，**不自动翻译** | ✅ `rendering_the_history_page_does_not_submit_anything`、`picking_a_row_fills_the_text_page_without_translating` |
| TC-6.1-b | REQ-006 | 历史文件损坏 / 读不出来 | 坏行跳过、整页仍可用；读不出来时给可读原因，翻译不受影响 | ✅ `corrupt_history_lines_are_skipped_and_the_page_stays_usable`、`an_unreadable_history_file_reports_a_reason` |
| TC-9.1-a | REQ-009 | 另一进程持锁时启动 | 第二个实例拿不到锁、非零码退出，不碰配置/历史/钥匙串 | ✅ `second_acquire_in_same_dir_is_rejected_while_holder_lives`；真机 W-1.19 已过（2026-10-20）：第二实例退出码 **1**，打印中文三行（含持有者 PID 21841 与自愈步骤）；`config.toml`/`history.jsonl` 的 mtime 与大小均未变，钥匙串未被访问；第一实例存活，锁文件内容仍是 `21841`（没有删别人的锁）；证据 `features/CHG-003/evidence/*-V-010.txt`（取编号最大者） |
| TC-9.1-b | REQ-009 | 陈旧锁（PID 已消失 / 内容损坏 / PID 被回收） | 一律接管；释放时只删 PID 与自己相符的那一份 | ✅ `stale_lock_from_dead_pid_is_taken_over`、`corrupt_lock_content_is_treated_as_stale`、`a_stale_lock_recording_our_own_recycled_pid_is_taken_over`、`release_does_not_remove_someone_elses_lock` |
| TC-N-1 | NFR-2 | 空闲帧循环 | 无提示条、无在飞请求时不请求重绘 | ✅ 代码审查 `app.rs::ui`（只有 toast 与结果到达才请求重绘）；真机 W-1.20 已过（2026-10-20）。**`ps -o %cpu` 测不了空闲窗口**（它是进程生命周期的平均值），改用 cputime 差值：30.2s 观察窗内 cputime `0:15.62 → 0:15.62`，即消耗 **0.000s = 0.0000%**（阈值 <1%），RSS 93.3MB；证据 `features/CHG-003/evidence/*-V-010.txt`（取编号最大者） |

## 2. 界面人工走查（P4）

对照 `03-ui-design.md` 的控件清单逐行勾选。走查结论表：

| 编号 | 检查项 | 存在 | 位置符合 | 行为符合 | 备注 |
|---|---|---|---|---|---|
| W-1.1 | 页签四项（文本/图片/历史/设置）齐全且切换保留状态 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.2 | 语言对下拉在顶部居中，源→目标方向箭头清晰 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.3 | 机翻/AI 为分段选择器（非两个独立按钮），当前项高亮 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.4 | 悬浮窗按钮在最右上角，与页签不粘连 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过；**第四轮复核**（2026-10-21）：位置不变，文案由 `⌗ 悬浮` 改为纯文字 `悬浮窗`（缺陷 D-5，装饰字形在真机上显示为豆腐块），点击可切到简洁窗、简洁窗点"全量"可切回 |
| W-1.5 | 源文区与译文区 50/50 分栏，间距 8px，缩窗后不重叠 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.6 | 空态提示文案"输入后按 Enter 翻译"存在且为灰色 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.7 | 加载态主按钮禁用并显示"翻译中…"，旧译文不被清空 | ✅ | ✅ | ✅ | **第一轮用户报通过，但结论不可采**：那时整条按钮行被裁出面板（缺陷 D-1），用户看不到主按钮，只可能凭译文区的 spinner 判定。D-1 修复后重看：**第四轮走查通过**（2026-10-21，动作条已移进译文框右下角，按钮变灰 + "翻译中…" + 旧译文保留） |
| W-1.8 | 错误态：故意填错 Key → 红色原因文本 + "去设置"按钮可点 | ✅ | ✅ | ⚠ | **假 Key 分支未在真机复现**：填假 Key 会覆盖用户钥匙串里的真 Key，用户 2026-10-21 选择不走（钥匙串写入/清除本身已验，见 V-012）。**已有的替代证据**：① 错误态渲染路径在真机上被 429 走通——状态条转红 + 可读中文原因"被限流（HTTP 429），请稍后再试"，进程不崩、源文不丢（`/tmp/ice-round4.log` 三条 WARN，V-010 §4）；② `Unauthorized`/`NotConfigured` → `FailureAction::GoSettings` → 状态条出现「去设置」→ 切到设置页签并把焦点送进 Key 输入框，这条链由 `failure_keeps_source_and_maps_action` 与 `failure_action_go_settings_switches_tab_and_focuses_the_key_field` 覆盖。**缺口只有"红字 + 去设置按钮"视觉同框未经真机确认**，P8 回归时用一把废 Key 补上 |
| W-1.9 | 两个复制按钮（复制译文 / 复制对照）命名与设计一致、行为不串台；译文为空时点击只提示、剪贴板原内容不变 | ✅ | ✅ | ✅ | **第一轮失败 → 缺陷 D-1**：用户"没有找到复制按钮"，整条底部动作行被 `CentralPanel` 裁掉。D-1 修复后用户又提出位置改版（2026-10-21）：按钮改放译文框内右下角的自带外框动作条，**常驻不隐藏**（先做过"碰边框浮出"版，用户看到实物后改为常驻）。**第四轮走查通过**（2026-10-21）：动作条在译文框内右下角可见、不压框、点击行为正确；空译文不碰剪贴板这一支由 `copy_buttons_refuse_to_touch_the_clipboard_without_a_translation` 锁住（只发提示 `没有可复制的译文`，`Effects::copy` 保持 `None`） |
| W-1.10 | 缓存/耗时徽标只在对应状态下出现 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.11 | 状态条三态（就绪/未配置/错误）文案正确 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.12 | 最小尺寸 640×420 下无控件被裁掉 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.13 | 暗色主题下所有文本与徽标对比度可读 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.14 | 亮色主题下同 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.15 | Tab 顺序与视觉顺序一致，主要操作键盘可达 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.16 | 18,000 字符粘贴后界面不掉帧、能出结果 | ✅ | ✅ | ✅ | 配合 TC-P-2。**第一轮未验成**：样本按加权字符数是英文占多数，触发同语言拦截；样本已重做为 18,000 字符纯简体中文（汉字 15,987 / 拉丁 16 / 繁体样本字 0，置信度 0.9995 → 简体中文），文件 `/tmp/ice-18k-paste.txt`（生成脚本 `/tmp/gen_ice_18k.py`）。**第二轮跑出缺陷 D-4**，用户原话"左边的文本框变形了，贴底了，而且文本框无法下拉，右边翻译后没有变形，但是也没有下拉"：左栏根因是源文编辑器画进"丢弃的子 Ui"，而 `Ui::new_child` 不继承裁剪（只克隆父 painter 的 clip_rect，egui ui.rs:210-235），`TextEdit::multiline` 又按内容全高分配矩形，18k 字符把框撑到窗口外、既裁不掉也没有滚动容器。修复：两栏正文都进定高 `ScrollArea`，源文外框改由定高 `Frame::group` 画（TextEdit 自带框跟着内容高度走，溢出时底边会被裁掉）；新增 2 条回归测试。**第四轮真机复验通过**（2026-10-21）：粘贴后左框不变形、不贴底、四边边框完整，两指/滚轮能一路滚到末尾，字数行显示 18,000；译文栏同样可滚到底——用户报的"右边也没有下拉"随滚动容器一起消失（无头滚轮测试 8 帧偏移增长，与真机一致）。布局耗时本身已由 TC-2.1-c 证明 15.6ms/100ms |
| W-1.17 | 中文界面文案无错别字、无中英混排破版 | ✅ | ✅ | ✅ | ✅ 2026-10-20 第一轮走查通过 |
| W-1.18 | 启动时钥匙串授权框不冻住窗口（窗口先建好、读取在后台线程） | ✅ | ✅ | ✅ | 2026-10-20 日志时序：建窗 09:24:49.513 → 钥匙串回值 09:25:07.796，中间 18.3s 窗口持续可交互 |
| W-1.19 | 第二个实例被单实例锁挡住且不留痕 | ✅ | ✅ | ✅ | 2026-10-20：退出码 1、中文三行提示含持有者 PID；配置/历史/钥匙串均未被触碰，锁未被误删 |
| W-1.20 | 空闲时不空转 CPU（NFR-2） | ✅ | — | ✅ | 2026-10-20：cputime 30.2s 内零增长 = 0.0000%（阈值 <1%），RSS 93.3MB |
| W-1.21 | **Cmd+W 收进后台后点 Dock 图标能恢复成可用窗口**（FR-5.2） | ✅ | ✅ | ✅ | **第一、二轮失败 → 缺陷 D-2**：回来的是一张被非等比拉伸的旧画面，边框拖不动、字变窄变扁、按钮输入框点上去毫无反应，用户只能关窗；第二轮"最小化重新叫出来，还是贴图"，根因是判定与心跳写在 `App::ui` 里，而 eframe 在隐藏期间只调 `App::logic`、`ui` 一帧都不跑。**第三轮通过**（2026-10-21，用户原话"可以了，收起叫回后依然能够使用"）：看护挂到 `logic`、判据加宽为几何或焦点、800ms 静默期、补发 `Minimized(false)`。日志取证：收起后 248ms 进入 `revealed=false focused=false`，2084ms 时两者同时回来并触发"窗口恢复可见，重发几何" |
| W-1.22 | **源文栏与译文栏的底边对齐**（两框等高，视觉上没有一高一矮） | ✅ | ✅ | ✅ | **第二轮走查截图发现（缺陷 D-3，2026-10-21）**：译文框撑满整栏、源文框为字数行让位而矮一截，"两边窗口大小不一样"。已改为两栏共用定高页脚行 + 正文精确占位，由 `both_panes_end_at_the_same_line` / `both_panes_align_even_with_an_empty_translation_footer` 锁住。**第四轮真机复验通过**（2026-10-21）：两栏底边齐平，字数行同一高度 |
| W-1.23 | **右下角动作条**：常驻译文框内右下角、自带外框、四边不压译文框边框；窄窗口（640px）下四个按钮换行不溢出 | ✅ | ✅ | ✅ | 用户提出的位置改版（2026-10-21，两轮反馈：先要悬浮浮出，看到实物后改常驻＋"边框有点遮挡"修间隙）。**第四轮真机复验通过**（2026-10-21）：动作条在译文框内、不压边框，窗口拖到最窄（640px）不溢出 |
| W-1.24 | **界面无缺字（豆腐块）**：按钮与提示里的符号都显示得出来 | ✅ | — | ✅ | **第三轮走查发现（缺陷 D-5，2026-10-21）**：用户截图里提示条的关闭按钮是一个方块，原话"不光是这一个按钮"。根因：`⌗ 悬浮`、`▣ 全量`、`⧉ 复制`、`✕` 四个装饰字形既不在 egui 内置字体（Ubuntu-Light / Hack / egui-icons）里，eframe 的 macOS 系统字体回退也匹配不到，只能画豆腐块。改为纯文字（`悬浮窗` / `全量` / `复制`）与 Latin-1 的 `×`；提示条的关闭符同时从"文字下方"移到"文字之后同一行"。**第四轮真机复验通过**（2026-10-21，用户原话"可以了"）：全量界面工具条、简洁窗按钮、提示条均无豆腐块，× 在文字右侧同一行、可点掉、四秒自动消失 |

## 3. 简洁模式走查与实测（P5）

| 编号 | 检查项 | 方法 | 预期 |
|---|---|---|---|
| W-2.1 | 窄条尺寸约 460×96、无系统标题栏 | 目测 + 截图 | 与 `03-ui-design.md` §3 一致 |
| W-2.2 | 输入行/译文行/三个动作图标齐全 | 对照 K-01~K-05 | 存在且不重叠 |
| W-2.3 | 置顶 | 开启后把 Safari 全屏，观察悬浮条 | 悬浮条在上层且可点击 |
| W-2.4 | 置顶可关 | 关闭后被其他窗口覆盖 | 恢复常规层级 |
| W-2.5 | 四边吸附 | 依次拖到上下左右松手 | 贴边停靠并保留 6px 热区；停靠指示文字随之改变 |
| W-2.6 | 拖离 | 停靠后向外拖 200px | 回到自由浮动，不再贴边 |
| W-2.7 | 切换保留状态 | 有内容时 Cmd+Shift+M 往返 20 次 | 源文、译文、语言对、引擎均不变；网络请求不重复发起 |
| W-2.8 | 位置记忆 | 停靠右边缘后退出重启 | 仍在右边缘停靠；全量模式下尺寸位置恢复 |
| W-2.9 | 失焦半透明 | 点其他应用观察悬浮条 | 85% 不透明；设置中关闭后恒为 100% |
| W-2.10 | 剪贴板即翻 | 开启后在别处复制一段英文 | 悬浮条自动填入并出译文 |
| W-2.11 | Esc 行为 | 简洁模式按 Esc | 清空输入，不退出应用 |
| W-2.12 | 关窗不退出 | Cmd+W 后查进程与 Dock | 进程在、CPU < 1%，点 Dock 图标恢复 |
| W-2.13 | **无边框可拖动**（P1 用户反馈补强） | 置顶开启状态下，分别拖动顶部把手条、图标行空白、译文行尾部空白 | 三种把手位均可拖动窗口到屏幕任意位置；拖输入框/按钮时不触发窗口移动；松手即触发吸附判定 |
| W-2.14 | **形变动画**（FR-3.9） | 连续切换全量↔简洁 10 次并录屏 | 看得出是同一窗口在伸缩（180~220ms 缓出），无跳窗、无黑屏、无内容闪烁；动画中点击按钮立即响应；两端尺寸与规范一致（±2px） |
| W-2.15 | **简洁窗内可切换机翻 / AI**（P4 第四轮用户预报） | 简洁窗内切引擎后再译一次 | 有可达的引擎切换控件；切到未配置 Key 的 AI 时给中文提示而不是静默失败；切换后当前源文/译文不丢 |
| W-2.16 | **简洁窗译文可滚动**（P4 第四轮用户预报） | 在简洁窗里译一段超出一行/一屏的文本 | 译文区能滚动或用等效方式看全（窄条版式可用展开/换行，但不得截断且无法查看）；滚轮与两指手势都生效 |

> **P4 第四轮走查预报的简洁窗问题（2026-10-21，用户原话）**：① "悬浮窗点击后还是和原框一样大，应该是一个很精致的窗口" → 归 W-2.1；② "悬浮窗无法拖动" → 归 W-2.13；③ "悬浮窗内无法更换机翻还是 ai" → 新增 W-2.15；④ "悬浮窗内的译文不能滚动" → 新增 W-2.16。四条都属 CHG-003 规格第 19/25 行明确划给 P5 的"简洁窗内部版式"，本包只保证切换按钮/快捷键存在与切换后内容不丢，故不计入 P4 走查失败项，转为 CHG-004 的入场清单（`07-acceptance.md` §P4 遗留）。

## 4. 图片翻译用例（P6/P7）

### 样图集（`assets/samples/`）

| 编号 | 内容 | 考察点 | 需求 |
|---|---|---|---|
| IMG-1 | 终端报错截图（纯色背景，等宽字体） | 基线正确率、回填无痕度 | FR-2.2/2.4 |
| IMG-2 | 英文 PPT 单页（浅底深字，含标题与项目符号） | 行合并、对齐还原 | FR-2.2/2.8 |
| IMG-3 | 产品说明书扫描页（小字密排） | 字号估计、超容量降级 | FR-2.7 |
| IMG-4 | 软件设置界面截图（表单两列） | 表格防护、不跨列合并 | FR-2.2 |
| IMG-5 | 中英混排网页段落 | 混排检测、术语 | FR-1.2/4.5 |
| IMG-6 | 深色模式截图 | 前景/背景采样反向情形 | FR-2.8 |
| IMG-7 | 照片上的白色描边标题字 | 判定为 Complex → 覆盖层 | FR-2.5 |
| IMG-8 | 渐变按钮上的文字 | Gradient 采样、双线性填充 | FR-2.4 |
| IMG-9 | 手写体/艺术字海报 | 已知失败样例，只记录不归入达标计算 | L-2 |
| IMG-10 | 日文菜单截图 | ja→zh 通道质量 | FR-2.2 |
| IMG-11 | 低对比度灰底灰字 | 置信度阈值与 ⚠ 提示 | FR-2.3 |
| IMG-12 | 竖排中文书法标题 | 已知失败样例 | L-5 |

IMG-9/12 为**故意纳入**的失败样例：验收时它们不参与达标均值计算，但要确认系统给出的是"降级为覆盖层/提示低置信"，而不是崩溃或产出空白图。

### 用例

| 编号 | 需求 | 步骤 | 预期 |
|---|---|---|---|
| TC-2.1-a..d | FR-2.1 | 分别用拖拽、粘贴、文件选择器、路径导入 IMG-1 | 四种入口都进入识别流程，结果一致 |
| TC-2.2-a | FR-2.2 | 对 IMG-1~IMG-12 跑 OCR，与人工标注比对 | 排除 IMG-9/12 后平均字符正确率 ≥ 0.92 |
| TC-2.2-b | FR-2.2 | 在跑 OCR 的同时用 `nettop -p <pid>` 观察网络连接 | 无任何出站长连接（隐私断言，NFR-3） |
| TC-2.2-c | FR-2.2 | 检查每个块 | 均有 bbox（在图内）、text 非空、confidence ∈ [0,1] |
| TC-2.2-d | FR-2.2 | 归一化坐标转换 | 单测：y=0 与 y=1 的翻转结果对应图像顶/底行 |
| TC-2.3-a | FR-2.3 | 手改列表中一处错字再生成图片 | 生成图反映修改 |
| TC-2.3-b | FR-2.3 | 取消勾选某块 | 该块不翻译、不绘制，其余正常 |
| TC-2.4-a | FR-2.4 | 对 IMG-1 生成结果 | 原图文件哈希不变；结果图中原文消失、中文出现在原区域 |
| TC-2.5-a | FR-2.5 | 对 IMG-7 生成 | 自动走覆盖层，文字可读，无擦除痕迹 |
| TC-2.5-b | FR-2.5 | 强制全局覆盖层模式处理 IMG-1 | 全部块用覆盖层绘制，不修改原图像素 |
| TC-2.6-a | FR-2.6 | 保存 PNG 与复制图片 | 文件可被预览打开；剪贴板粘贴到别处得到同图 |
| TC-2.7-a | FR-2.7 | 中→德长译文窄块（构造 IMG-13） | 先缩字号；到下限仍放不下则标 `degraded` 并走覆盖层，无文字丢失 |
| TC-2.8-a | FR-2.8 | IMG-6 深色截图 | 回填文字颜色与原前景色一致（允许 ±8 通道误差） |
| TC-2.9-a | FR-2.9 | 处理 IMG-5（多块） | 进度按三阶段推进且块数递增 |
| TC-2.9-b | FR-2.9 | 处理中点取消 | 及时停止、不生成结果文件、可立即重开 |
| TC-2.2-e | NFR-7 | 在非 macOS 构建运行 | 图片页显示"当前平台不支持"，文本功能正常 |

### 人工评分表（P7 用，每张图一行）

| 样张 | 回填可读性(1-5) | 位置准确性(1-5) | 观感自然度(1-5) | 模式 | 备注 |
|---|---|---|---|---|---|
| IMG-1 | | | | | |
| IMG-2 | | | | | |
| … | | | | | |

目标：可读性均值 ≥ 4.0，位置 ≥ 3.5，观感 ≥ 3.0（`04-image-pipeline.md` §5）。

## 5. 非功能用例

| 编号 | 需求 | 方法 | 预期 |
|---|---|---|---|
| TC-P-1 | NFR-1 | 20 次采样：`time` 一段 200 字符英译中（机翻通道，冷/热各 10 次） | 冷 P50 < 800ms；热（缓存）< 5ms |
| TC-P-2 | NFR-1 | 18,000 字符粘贴并连续输入 200 次按键 | 无掉帧卡顿（主观 + 帧时间标签 < 16ms），输入不被吞 |
| TC-P-3 | NFR-1 | IMG-1（1080p）OCR 计时 5 次取中位 | < 1.5s |
| TC-P-4 | NFR-2 | 喂 1×1 图、超大 20000px 图、损坏 JPEG、非图像文件 | 全部给出可读提示，进程不 panic |
| TC-P-5 | NFR-3 | 审查 `RUST_LOG=debug` 全量日志输出 + `grep` 密钥与源文 | 日志无源文全文、无密钥、无 URL 查询串中的明文 |
| TC-P-6 | NFR-5 | 简洁模式停靠空闲 5 分钟，Activity Monitor 采样；`du -sh` release 产物 | 内存 < 200MB，CPU < 1%，产物 < 40MB |
| TC-P-7 | NFR-7 | `cargo check --target x86_64-pc-windows-msvc`、`x86_64-unknown-linux-gnu` | 通过（缺平台 SDK 时记录为"未验证"而非"通过"） |
| TC-P-8 | NFR-6 | `cargo tree -p ice-core`、`-p ice-providers` | 无 egui/eframe；providers 无 image |
| TC-P-9 | NFR-4 | 主题三态：①设为"跟随系统"后在系统设置里切换深/浅色 ②设为"强制浅色"后再切系统深色 ③重启应用 | ①应用在下一次重绘（≤1s）跟随，无需重启 ②保持浅色 ③重启后仍是上次选择的档位 |

## 6. 回归执行清单

每次发版前按顺序执行：

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace -- --ignored        # 需要真实网络，人工在场
cargo build --release -p ice-app
```

UI 走查在代码变更后至少重跑：W-1.5、W-1.7、W-1.8、W-1.12、W-2.3、W-2.5、W-2.7。
