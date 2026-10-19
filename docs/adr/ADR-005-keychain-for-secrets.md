# ADR-005 密钥与配置存储：钥匙串存密钥，配置文件只存引用

- 状态：已接受
- 日期：2026-10-19
- 关联：FR-4.1、FR-4.2、FR-4.3、NFR-3、R-5

## 背景

AI 通道需要用户自备 API Key。自用工具最容易犯的错误是"反正是自己的机器"，把密钥写进 `config.toml`，然后某次截图分享配置、或某次把目录同步到网盘时泄露。

## 决策

1. API Key 只存 **macOS 钥匙串**：`keyring` crate，service = `IceTranslator`，account = `ai-api-key`。
2. `config.toml` 中的 `ai` 段**不出现密钥字段**，只有 `key_configured: bool` 标记"用户已经保存过 Key"。运行时若需要实际值再向钥匙串取。
3. base_url、模型名、语言对、窗口几何等放 TOML（`~/Library/Application Support/IceTranslator/config.toml`），术语表 `glossary.toml`，历史 `history.jsonl`。
4. 配置读取采用"缺失即默认、损坏即回退 + 提示"，写入采用临时文件改名（原子替换）。
5. 日志与错误信息里不得出现密钥；错误提示遇到 401 时只说"Key 无效"。

## 理由

- 钥匙串按用户与设备加密，且不会随配置目录被误分享。
- `key_configured` 标记让"AI 是否可用"的判断不需要每次解密，界面能廉价地显示状态（R-4）。
- TOML 明文可读是有意为之：自用工具需要能手动改、能 diff、能在应用坏掉时手工恢复。

## 影响

- 首次配置 AI 通道必须走界面输入（或 `security add-generic-password` 手工写入），不能像很多工具那样支持环境变量注入密钥——这条限制写进 README，避免以后被当成 bug。
- Linux 上 `keyring` 走 Secret Service，若桌面环境没有密钥服务则明确报错并回退为"AI 通道不可用"，绝不静默写入文件。
