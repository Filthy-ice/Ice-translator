# ADR-002 OCR：macOS Vision 本地识别，不引入外部 OCR 引擎

- 状态：已接受
- 日期：2026-10-19
- 关联：FR-2.2、NFR-3、NFR-4、NFR-7

## 背景

图片翻译要求本地、快速、支持中英日混排，且不能把用户截图发出去。

候选：

| 引擎 | 质量（截图类） | 部署体积 | 离线 | 许可/合规 |
|---|---|---|---|---|
| macOS Vision（`objc2-vision`） | 高，系统级优化，支持多语言与方向 | 0（系统内置） | 是 | 系统框架，无风险 |
| Tesseract | 中，艺术字与低对比度差 | 需训练数据，几十 MB | 是 | 良好 |
| PaddleOCR | 高，中文强 | 需 ONNX/模型文件数百 MB + paddle 运行时 | 是 | Apache-2.0，但依赖重 |
| 云 OCR | 高 | 0 | **否** | 违背 NFR-3 |

## 决策

v1 只接 **Vision 框架**（`VNRecognizeTextRequest`，`revision >= 3`，`usesLanguageCorrection`，语言候选 `zh-Hans`/`en-US`/`ja`），通过 `objc2-vision` 从 Rust 调用。非 macOS 平台返回 `VisionError::UnsupportedPlatform`，界面明确提示"当前平台不支持图片翻译"，文本翻译功能不受影响。

## 理由

1. 零模型分发：安装体积与隐私面都最小，符合"自用工具当天就要能用"的目标。
2. 截图类图像正是 Vision 的强项，Live Text 同源能力。
3. 跨平台 OCR 抽象成 `OcrEngine` trait，Windows（Windows.Media.Ocr）与 Linux（tesseract）后续按同一 trait 增加实现，不需要改上层。

## 影响

- 需要处理 Vision 的输出坐标是**归一化、原点左下**：统一在视觉层转换成 `ice-core` 的左上角像素坐标，转换函数单独写单测，避免与识别逻辑混在一起。
- `Send` 约束：Vision 调用在 `spawn_blocking` 的工作线程内完成，不跨线程传 objc 对象，只传转换后的 `OcrDocument`。
- 置信度阈值与"行→段落合并"参数需要真实样图调参，样本集在 `assets/samples/`，结论记录在 P6 验收中。
