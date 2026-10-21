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

## 更新（2026-10-20，P4 实施后）

ADR 正文按惯例不改写，以下两条**取代**上文对应条款，决策方向不变：

1. **取代决策 3 中的"术语表 `glossary.toml`"**：术语改存 `config.toml` 的 `[[term]]`（`CHG-003` Q-003，用户决定），持久化职责移入 `ice-core::config`。理由是需求 §4.5 原文写的就是 config 内 `[[term]]`，是实现走偏；术语不是秘密，放在明文可读、可 diff、可手工恢复的 `config.toml` 里与本 ADR 的取舍一致。**旧的 `glossary.toml` 启动时一次性只读导入，绝不删除或改写**——用户自己的文件不由程序处置。
2. **细化决策 2 的 `key_configured`**：它只是**缓存标记**，钥匙串才是事实来源。启动时后台探测到"条目已被用户在『钥匙串访问』里手动删除"就清掉该标记并落盘（`the_probe_overrides_a_stale_configured_flag`），反之读到 Key 就补上。另：读取必须排在**建窗之后**、在 `spawn_blocking` 里做——macOS 对不由本程序写入的条目会弹授权框并同步阻塞 `get_password()`，放在建窗前窗口根本出不来，表现为"启动卡死"。这两点不改"密钥只存钥匙串、不进 `config.toml`、不进日志"的核心决策。
