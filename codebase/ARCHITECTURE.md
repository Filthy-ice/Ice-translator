# 代码地图 — ice-translator

约定：每条事实后面括号标出可复查的代码位置；未落地的小节标 `计划`，不写推测。
仓库根：`codebase/repositories.json` 里的 `ice-translator`（本机路径在 `.sdd/repos.local.json`，不入库）。

## 责任

单一二进制桌面客户端（macOS 优先）。文本翻译（机翻 + AI 双通道）、图片文字翻译与译文回填、
全量窗口与可置顶贴边的简洁窄条两种形态。（`README.md`、`docs/01-requirements.md`）

## 分层与入口

- 依赖单向向下：`ice-app` → {`ice-providers`, `ice-vision`} → `ice-core`；`ice-core` 无 IO/UI/网络依赖
  （`Cargo.toml` workspace 与四个 `crates/*/Cargo.toml`；已用 `cargo tree -p ice-core -e normal` 核实）。
- 程序入口 `crates/ice-app/src/main.rs`：`eframe::run_native` + `IceApp`（实现 `eframe::App`），
  启动时按 `config.ui.mode` 决定初始尺寸、边框与窗级。
- UI 唯一允许接触的翻译入口是 `ice_providers::dispatcher::Translator`（`crates/ice-providers/src/dispatcher.rs`）。

## 模块现状

| crate | 已落地 | 计划 |
|---|---|---|
| `ice-core` | `language`、`document`、`config`、`history`、`glossary`、`detect`、`error` | — |
| `ice-providers` | `engine`（trait）、`request`、`cache`、`dispatcher`、`error` | `machine`、`ai`、`protect`（P3） |
| `ice-vision` | `ocr`、`composite` 的接口与桩、`error` | `vision_sys`、`layout`、`bg`、`font`（P6/P7） |
| `ice-app` | 单窗口双形态、置顶、几何日志 | `state`、`mode`、`views::*`、`jobs`、`system`（P4/P5） |

## 状态流

用户输入 → `TranslationRequest`（`request.rs`）→ `Translator` 选引擎 → `TranslationCache`
（AI 通道以术语表版本加盐，`cache.rs`）→ 引擎 → 术语后处理 → 回写缓存 → UI 状态。
窗口形态：`IceApp::apply_mode` / `apply_topmost` 下发 `ViewportCommand`，几何在下一帧读回并记日志。
（`crates/ice-app/src/main.rs`）

## 配置与数据

`~/Library/Application Support/IceTranslator/` 下 `config.toml`、`glossary.toml`、`history.jsonl`；
写入用 tmp + rename 原子替换，损坏回退默认配置并记录原因（`config.rs::load_at`/`save_at`）。
AI Key 只在 macOS 钥匙串（service `IceTranslator`，account `ai-api-key`），不进配置文件与日志（ADR-005）。

## 测试基线

`cargo test --workspace` 当前 53 项（`ice-core` 40 · `ice-providers` 10 · `ice-app` 3 · `ice-vision` 0）。
`ice-app` 的 3 项是无头断言：用裸 `egui::Context` 驱动 `apply_mode`/`apply_topmost`，
检查排队的 `ViewportCommand` 序列，不需要开窗口或 GPU。
联网测试以 `net_` 前缀 `#[ignore]`，P3 落地后用 `cargo test -- --ignored` 显式跑（`docs/05-dev-guide.md`）。

## 已知风险与约束

- eframe/egui 0.37 与常见教程 API 差异较大，升级前须重跑 `ice-app` 无头测试（差异清单见 `docs/07-acceptance.md` P1 事实）。
- SIGTERM 不触发 `on_exit`，异常退出丢窗口几何；几何即时落盘是 P5 待办。
- 本机无法合成鼠标点击，UI 交互类验证靠无头断言 + 人工走查（`docs/06-test-cases.md`）。
