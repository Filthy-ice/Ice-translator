---
name: sdd-continue-work
description: "从 Ice Translator SDD 的 Git 与本地工作树恢复 CHANGE-ID、批准哈希、任务、证据和阻塞，跨天或换 Agent 继续已授权工作。用于恢复 SDD 现场，不用于仅依赖聊天记忆的普通续写。"
---

# Continue Work

从持久化产物恢复现场，保留 Git 提交后尚未提交的新进度。

1. 使用用户指定工程；否则查当前目录及其上级的 `project/config.json`，找不到就询问用户工程路径。读 AGENTS.md、STATE 导航，进入指定 Feature；未给 ID 时按索引与实际任务推断，存在多个同样可能的活动变更才询问。
2. 读 change.json/spec.md，核对 Git HEAD、工作树修改、审批快照/哈希和批准出处。Feature 为权威，STATE 是导航。运行 `status ID`、`validate ID`；换机先 `validate ID --offline` 检查存档，再重新绑定实际代码仓库路径。offline 通过不证明本机代码或证据仍然匹配。
3. 按阶段加载：批准前读 Spec 与 Q 表；设计/实施阶段再读 design.md、tasks.json、traceability.json；核对相关 Evidence 与附件，区分旧版本、失败记录、Mock 与真实运行证据。只读关联 Feature/Incident，不把全部历史装进上下文。
4. 核对登记目标分支、实际仓库 HEAD、dirty 修改、当前代码内容哈希。路径错、分支错、规格漂移或失败结果都是阻塞，不通过手改状态/哈希绕过。不要覆盖用户工作树来恢复旧基线。
5. 报告简短恢复结果：ID@version、当前状态/批准哈希、已完成/剩余任务、阻塞与下一步。已有批准和会话授权有效时立即继续相应阶段，不重新索取同一批准。

INTAKE/CLARIFIED/SPEC_REVIEW 继续调查与规格，不能实施；APPROVED/DESIGNED 继续设计/校验；IMPLEMENTING 继续映射任务；VERIFIED 提供验收证据，不冒充 ACCEPTED。已 ACCEPTED 保存为历史记录，新业务变化新建 Feature；未来代码更新不会反证历史验收。
发生业务偏差先 deviation，行为改变 revise 升版；实现方案调整 replan 后重封存并重新验证。修复生成视图可 `refresh ID`，不手改 tasks.md/evidence.md。最后 `index` 更新导航。命令和数据细节按需读工程 docs/workflow.md。
