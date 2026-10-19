# 需求追踪矩阵

列含义：设计=设计文档章节；实现=代码位置（`crate::module`）；验证=测试用例编号（见 `06-test-cases.md`）；结果=通过 / 带限制通过 / 未做。
本表在 P8 收口时必须无空格；确实不做的行移入"延期项"。

## 文本翻译

| 需求 | 级 | 设计 | 实现 | 验证 | 结果 |
|---|---|---|---|---|---|
| FR-1.1 输入区 ≥20k 字符 | M | 02 §3.1、03 §2 | `ice-app::views::full` | TC-1.1-a/b | 待 P4 |
| FR-1.2 源语言与自动检测回显 | M | 02 §2 core/detect | `ice-core::detect`、`ice-app::views::full` | TC-1.2-a/b | 待 P3 |
| FR-1.3 双引擎切换即重译 | M | 02 §2 dispatcher | `ice-providers::dispatcher` | TC-1.3-a/b/c | 待 P3 |
| FR-1.4 目标语言与最近使用 | M | 02 §2 core/config | `ice-core::config::remember_target` | TC-1.4-a/b | ✅ 单测 `remember_target_keeps_five_and_most_recent_first` |
| FR-1.5 复制/对照/替换剪贴板 | M | 03 §2 F-09 | `ice-app::system::clipboard` | TC-1.5-a/b/c | 待 P4 |
| FR-1.6 不可译内容保护 | M | 02 §3.1 protect | `ice-providers::protect` | TC-1.6-a..f | 待 P3 |
| FR-1.7 失败可诊断不丢源文 | M | 02 §5 | `ice-providers::error` + `ice-app` | TC-1.7-a/b/c | 部分 ✅ `unregistered_engine_reports_not_configured_for_ai` |
| FR-1.8 缓存命中 | S | 02 §2 cache | `ice-providers::cache` | TC-1.8-a/b | ✅ 单测 `second_identical_request_hits_cache_and_skips_engine` |
| FR-1.9 AI 超长分段 | S | 02 §3.1 | `ice-providers::ai` | TC-1.9-a | 待 P3 |

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
| FR-3.1 全量模式布局 | M | 03 §2 | `ice-app::views::full` | 走查表 W-1 | 待 P4 |
| FR-3.2 简洁模式窄条（含无边框可拖动） | M | 03 §3 K-06 | `ice-app::views::compact`（`StartDrag`） | 走查表 W-2、W-2.13 | 待 P5 |
| FR-3.3 切换保留内容 | M | 03 §4、ADR-001 | `ice-app::mode` | TC-3.3-a | 待 P5 |
| FR-3.4 置顶 | M | 03 §3 K-09、ADR-001 | `ice-app::mode`（`WindowLevel`） | TC-3.4-a | 骨架已验证 API 可用 |
| FR-3.5 边缘吸附与脱离 | M | 03 §3 吸附几何 | `ice-app::mode::snap`（纯函数） | TC-3.5-a..d | 待 P5 |
| FR-3.6 记忆位置尺寸与越界回正 | M | 02 §3.3、§6 | `ice-core::config` + `ice-app::mode` | TC-3.6-a/b | 部分 ✅ `window_off_screen_detection` |
| FR-3.7 失焦半透明 | S | 03 §3 K-08 | `ice-app::mode` | 走查 W-3 | 待 P5 |
| FR-3.8 剪贴板即翻 | S | 03 §4 | `ice-app::system::ClipboardWatcher` | TC-3.8-a | 待 P5 |
| FR-3.9 切换形变动画 | M | 03 §4 过渡 | `ice-app::mode::Morph`（逐帧补间） | 走查 W-2.14 | 待 P5 |

## 配置、历史与术语

| 需求 | 级 | 设计 | 实现 | 验证 | 结果 |
|---|---|---|---|---|---|
| FR-4.1 设置页各项持久化 | M | 02 §6 | `ice-core::config`、`ice-app::views::settings` | TC-4.1-a | 部分 ✅ `save_at_creates_dirs_and_leaves_no_temp_file`、`roundtrip_keeps_values` |
| FR-4.2 密钥入钥匙串且不入日志 | M | ADR-005 | `ice-app::system::Secrets` | TC-4.2-a/b | 待 P4 |
| FR-4.3 配置目录规范与损坏容错 | M | 02 §6 | `ice-core::config::load_at` | TC-4.3-a/b | ✅ 单测 `load_at_falls_back_to_defaults_with_reason` 等 3 条 |
| FR-4.4 历史 500 条可搜可回显 | S | 02 §2 history | `ice-core::history` | TC-4.4-a..d | ✅ 单测 5 条 |
| FR-4.5 术语表即时生效 | S | 02 §2 glossary | `ice-core::glossary` + `dispatcher` | TC-4.5-a/b/c | ✅ 单测 `glossary_applies_to_cached_result_without_new_request` 等 6 条 |

## 系统集成与非功能

| 需求 | 级 | 设计 | 实现 | 验证 | 结果 |
|---|---|---|---|---|---|
| FR-5.1 应用内快捷键 | M | 03 §4 | `ice-app::system::HotKeys` | 走查 W-4 | 待 P4 |
| FR-5.2 关窗不退出 | M | 03 §4 | `ice-app`（`Visible(false)` + Dock 恢复） | TC-5.2-a/b | 待 P4 |
| FR-5.3 屏变化后状态正常 | S | 02 §3.3 | `ice-app::mode` | TC-5.3-a | 待 P5 |
| NFR-1 性能指标 | M | 05 §4 | `ice-app::jobs` + 采样脚本 | TC-P-1..3 | 待 P8 |
| NFR-2 无 panic | M | 02 §5 | 全 crate | TC-P-4 | 待 P8 |
| NFR-3 隐私 | M | ADR-002/005 | `ice-vision`、日志实现 | TC-P-5 | 待 P8 |
| NFR-4 中文界面·主题跟随系统+三态开关 | M | 03 §1 | `ice-app`（`ThemePreference`） | TC-P-9 | 待 P4 |
| NFR-5 资源占用 | S | 05 §1 | release 构建 | TC-P-6 | 待 P8 |
| NFR-6 分层依赖 | M | ADR-006 | workspace | `cargo tree` 检查 | 部分 ✅ P1 已核（core 依赖树无 egui/eframe/reqwest/image），P8 三目标复核 |
| NFR-7 跨平台可用与降级 | S | ADR-002 | `ice-vision::native_ocr_supported` | TC-P-7 | 待 P8 |
| NFR-8 可访问性：字号缩放与键盘可达 | C | 03 §1、§5 | `ice-app`（`pixels_per_point` 跟随 + Tab 顺序） | 走查 W-1、§5 第 5 步 | 待 P4 |

## 统计

- 需求共 44 条：**Must 30 · Should 12 · Could 2**（按 `01-requirements.md` 逐行重新计数得出；此前记的 27/16/2 是 P0 时的旧值，FR-3.9 加入后未重算，已在 P2 自检时更正）。
- 骨架阶段（P1）已闭环的 Must：FR-3.4 的置顶能力、FR-4.3 全部、FR-3.6 的几何判定部分、FR-1.4 的一部分（语言模型与持久化）。
- 2026-10-19 需求变更（用户在 P1 走查后提出）：FR-3.2 明确"无边框必须仍可拖动"、新增 FR-3.9 形变动画、NFR-4 改为默认跟随系统深浅色并保留三态开关。三项均落在 P4/P5。
