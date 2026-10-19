# 从原实践到这次重建

依据：[原博客](https://blog.inshocking.com/posts/sdd-practice/)，读取日期 2026-10-08。
这是按文章恢复并增强工作方式的新工程，没有找回旧机器上的代码或历史 Spec。

## 保留

独立规格仓库、根目录平铺的六类规格目录、四份 Feature 文档、五个 Skill、八状态门禁、四级事实标签、需求追踪链和 SPEC_DEVIATION 退路。业务源码仍留在各自仓库。局部方案与事故事实不会被全局化。

## 优化

原文提到的手工 Status/Version 校验变为 JSON 元数据与合法状态迁移；批准保存原字节 SHA-256 和快照；跨文件基线自动写入；计划独立封存。每条 Requirement 检查 Design/Task/必需验证，附件带内容哈希，测试后源码变动使证据过期。STATE 从单项记录生成。绝对代码路径保存在不提交的本机映射，跨机只需重新绑定。

Spec 与 Design 继续用 Markdown，容易审阅；Tasks、追踪、Evidence 增加 JSON，只把需要确定性校验的字段结构化。默认只需要 Python 3.11+ 和 Git，不引入数据库、Web 管理平台、模型 SDK 或常驻 Agent 运行时。

## 主流工具取舍（2026-10-08 核对官方材料）

| 工具 | 借鉴点 | 本工程取舍 |
|---|---|---|
| [GitHub Spec Kit](https://github.com/github/spec-kit/blob/main/docs/index.md) | 规格到计划、任务、实施与收敛；不同编码工具接入 | 保留先规格后实施的可交接产物；不复制整套命令与第二个事实源 |
| [OpenSpec](https://github.com/Fission-AI/OpenSpec) | 增量变更、轻量迭代；独立规格 Store（官方标记 beta） | 平铺的规格目录已足够支撑跨仓场景；采用独立变更包思想，不让 beta Store 成为启动依赖 |
| [Kiro Specs](https://kiro.dev/docs/specs/) | Requirements / Design / Tasks 的规格流程 | 验收写明确 WHEN/THEN/SHALL 和边界场景；工程不依赖专用 IDE |
| [Codex / Agent Skills](https://learn.chatgpt.com/docs/build-skills) | SKILL.md、按需读取、可显式/自动选择 | 保留五个独立入口，具体校验交给本地工具；技能不捆绑模型和会话历史 |

这是一套独立的轻量实现，不是 Spec Kit/OpenSpec 的官方发行版或格式兼容层。若未来团队选定某个工具，先选唯一的规格事实源，再写适配；不并行维护当前目录与另一套重复业务规格。

## 当前边界

哈希保证内容关联，不证明确认人身份；本地工具不能强制控制可写文件的 Agent。任务映射保证结构覆盖，不证明业务语义正确。敏感信息检查是基线规则，不能覆盖所有业务标识。没有真实业务仓库和外部环境，交付验证是隔离样例与工具测试，真实接入在第一个 Feature 中完成。

后续可按真实需求补 Git 托管评审保护、构建产物 Spec 哈希、跨仓契约测试和可复查运行来源；优先跑通一个中等复杂需求，避免先建设平台。
