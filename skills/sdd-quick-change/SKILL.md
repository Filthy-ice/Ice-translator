---
name: sdd-quick-change
description: "在 Ice Translator SDD 中处理单仓、规则明确、风险极低的小变更，保留简短规格与验证；涉及业务状态、集成、数据、配置、权限或跨仓时升级 Feature。仅在明确使用 SDD 快速路径时触发。"
---

# Quick Change

缩短低风险工作的产物，不缩短业务判断边界。

使用用户指定工程；否则查当前目录及其上级的 `project/config.json`，找不到就询问用户工程路径。读工程 AGENTS.md 与实际代码仓库指令，先核对路径、目标分支和用户未提交修改。

## 判断是否适用

只有单仓、现有规则已明确、不会改变业务决策的小变更才使用 Quick。逐项排除消息、RPC、业务状态、Redis、数据库、配置、权限、个人信息、外部写操作和跨仓依赖。任何一项涉及或不确定都升级完整 Feature；一行代码不代表低风险。不要把风险字段改成 false 以满足门禁。

## 轻量实施

1. 登记实际 repo 后 `new CHG-ID --title ... --repos repo --quick`；在 change.json.risk_assessment 逐项记真实判断，全部明确 false 才能批准。
2. spec.md 用少量 REQ 说明当前/期望行为与验收，Design 写简短实现和回滚；不适用的复杂领域无需长篇占位。tasks.json 与 traceability.json 保留最小真实追踪链。
3. 按 CLARIFIED → SPEC_REVIEW 呈现版本与 hash。用户请求已明确授权具体行为且草稿完整对应这一范围时，可引用该真实授权记录批准，无需为了形式重复问；有新增业务判断则等待实际决定。无法对应当前 Spec 的授权不能自行扩展。
4. APPROVED 后 plan-stamp、seal-plan、transition IMPLEMENTING。执行任务与适当验证：文字/样式等低影响变更优先针对性检查，不制造重复实现的测试。可用 VERIFIED_CODE/static 的实际脱敏检查附件证明人工/Agent 检查，Verification 不设 command 时通过 record 导入，不伪造自动测试。
5. task-done 记录实际代码/测试文件；必需证据完成后 VERIFIED，真实验收后 ACCEPTED。已有当前范围授权持续有效；外部副作用仍遵循实际授权。

发现超出 Quick 范围时停止，保留 quick 记录并注明升级原因和新 Feature ID；新 Feature 回链 Quick，经实际业务确认后交付。不会以“快速修复”为由跳过批准哈希、证据或偏差退回。
字段格式与命令见工程 docs/workflow.md；tasks.md/evidence.md 是生成视图。
