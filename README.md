# Ice Translator

自用桌面翻译客户端。Rust 编写，macOS 优先。

两件事它做得比现成工具顺手：

1. **两种翻译通道**：机翻（零配置、快）与 AI（自备服务、质量高），同一句随手切换；
2. **图片里的文字**：截图丢进来，本机 OCR（不上传原图），翻译后**把中文画回图片**，复杂背景自动降级为覆盖层。

界面有两种形态：全量窗口，以及可以拖到屏幕边缘停靠、始终置顶的简洁窄条。

## 状态

v1.0 开发中，按 P0~P8 阶段推进，每阶段有验收记录。当前进度见 `docs/07-acceptance.md`。

| 阶段 | 内容 |
|---|---|
| P0 | 立项与需求 |
| P1 | 工程骨架 |
| P2 | 架构与界面设计 |
| P3 | 翻译引擎层 |
| P4 | 全量界面 |
| P5 | 简洁悬浮模式（置顶/贴边） |
| P6 | 图片 OCR |
| P7 | 译文回填 |
| P8 | 测试与验收收口 |

## 快速开始

```bash
xcode-select --install            # 若未安装命令行工具
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"

cargo run -p ice-app              # 启动客户端
cargo test --workspace            # 跑测试
```

首次编译要拉取并编译 eframe/wgpu 依赖树，Apple Silicon 上约 3~6 分钟。

配置与数据位置：`~/Library/Application Support/IceTranslator/`（`config.toml`、`glossary.toml`、`history.jsonl`）。
**AI 通道的 API Key 不进配置文件**，存 macOS 钥匙串：

```bash
# 也可以手工写入，service 与 account 固定
security add-generic-password -s IceTranslator -a ai-api-key -w
```

## 隐私

- 图片解码、OCR、原文擦除、译文绘制全部在本机完成，图片不上传任何服务；
- 文本只在你主动触发翻译时发往所选通道；
- 日志只记录文本长度与哈希前缀，不记录原文，不记录密钥。

## 已知限制

- 照片、渐变、纹理背景上的文字无法无痕擦除，自动走覆盖层；
- 艺术字、手写体、描边字的效果不复现；
- PDF 重排、区域截图取词、Windows/Linux 的 OCR 不在 v1 范围；
- 机翻通道使用公共服务，质量以"能看懂"为目标，长句和专业表述请用 AI 通道。

完整边界见 `docs/00-charter.md` 第 3 节与 `docs/04-image-pipeline.md` 第 6 节。

## 文档

| 文件 | 内容 |
|---|---|
| `docs/00-charter.md` | 立项：目标、不做清单、里程碑、风险 |
| `docs/01-requirements.md` | 需求：FR/NFR 编号与逐条验收标准 |
| `docs/02-architecture.md` | 架构：分层、模块、数据流、并发、错误映射 |
| `docs/03-ui-design.md` | 界面：布局、控件清单、快捷键、吸附几何、走查表 |
| `docs/04-image-pipeline.md` | 图片流水线：九个阶段、算法参数、评分办法 |
| `docs/05-dev-guide.md` | 开发：环境、命令、规范、故障 |
| `docs/adr/` | 关键取舍的理由 |
| `docs/traceability.md` | 需求 → 设计 → 实现 → 验证 矩阵 |
| `docs/06-test-cases.md`、`docs/07-acceptance.md` | 测试用例与阶段验收记录 |

`document/`（仓库同级）是 `docs/` 的软链，两处指向同一份文件。

## 许可

Apache-2.0，见 `LICENSE`。
