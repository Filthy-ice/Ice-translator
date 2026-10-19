# ADR-006 分层：四个 crate 与单向依赖

- 状态：已接受
- 日期：2026-10-19
- 关联：NFR-6、R-1、R-2

## 决策

| crate | 职责 | 允许依赖 | 禁止 |
|---|---|---|---|
| `ice-core` | 语言模型、图片文档模型（`TextBlock`/`PixelRect`）、配置、历史、术语表、纯计算 | 仅 serde/toml/thiserror 等基础设施 | 任何 IO 网络、UI、图像解码 |
| `ice-providers` | `Translator` 调度、缓存、机翻通道、AI 通道、语言检测、内容保护 | `ice-core` + reqwest/tokio | egui、image |
| `ice-vision` | OCR、背景估计、译文回写 | `ice-core` + image/objc2 | 网络、UI |
| `ice-app` | 事件循环、双模式布局、异步任务编排、系统集成 | 上面三个 + eframe | 业务计算（应下沉） |

依赖方向单向向下，`cargo tree` 可验证。

## 理由

1. **可测性**：翻译调度、缓存、术语替换、几何计算、行合并这些逻辑全部能在无窗口、无真机环境下断言（R-2 的对策依赖这一点）。
2. **故障隔离**：OCR 与图像合成是本项目最容易崩的部分。把它们关在 `ice-vision` 里、只通过 `OcrDocument` 这种纯数据结构与外界交换，崩溃面不扩散。
3. **平台差异收敛**：objc2/Vision 只出现在 `ice-vision` 的 `cfg(target_os = "macos")` 分支，其它 crate 完全平台无关。
4. 拆分成本很低（一个 workspace 四个目录），比"一个大 crate 内部按模块约定"更能靠编译器强制约束。

## 后果

- 跨层改动要动多个 `Cargo.toml`，小需求提交量变大——可接受。
- `ice-core` 必须保持"无副作用"，一旦有人往里塞网络调用，`cargo tree` 检查就会暴露。
- UI 状态机留在 `ice-app`，但它引用的类型（`UiMode`、`DockEdge`、`WindowRect`）定义在 `ice-core`，保证配置层能直接序列化窗口状态。
