---
name: sdd-incident-repair
description: "在 Ice Translator SDD 中记录线上事件证据、止血授权，并把已确认根因转换为独立永久修复 Feature。用于 Incident 与永久修复衔接，不替代普通调试或在根因未证实时猜修复。"
---

# Incident Repair

把事故现场与永久业务规则分开，让下一位 Agent 知道证据、根因和允许动作。

使用用户指定工程；否则查当前目录及其上级的 `project/config.json`，找不到就询问用户工程路径。读工程 AGENTS.md 和相关代码地图，已有 Incident 继续其记录，不因换会话再建一份。

- 使用 `templates/incident.md` 在 incidents/INC-ID/incident.md 保存影响、环境、时间线（保留时区）、预期与实际行为、来源、止血建议/授权/执行结果。真实账号、工单标识、内部链接和凭证先脱敏。
- 明确 VERIFIED_CODE、VERIFIED_RUNTIME、BUSINESS_CONFIRMED、HYPOTHESIS。源码结构不证明生产路径执行；单次日志不证明通用根因；可疑方向继续调查，不写成确认结论。
- 根因未确认时继续证据调查；可使用实际可用的诊断工具/技能。不得通过本技能臆造根因，不能把临时止血配置或偶发状态写成永久业务规则。
- 外部查询和止血按实际授权范围进行，尤其记录环境、对象、有效期、回滚和停止条件。会话已有授权可继续，不重复确认；未经授权的外部写动作不执行。
- 根因确认后建立独立 CHG-ID Feature，Spec 引用 Incident 根因及证据，Incident 回链 Feature。定义正常/异常/回归验收，再沿 intake → 批准 → delivery；临时修复已获授权不自动批准另一套永久方案。
- 永久修复的任务、代码/测试和证据属于 Feature；Incident 保留事故事实、止血结果、恢复观察和复盘。只把经验证、长期可复用的信息更新到 codebase/project。

交付时说明影响、根因置信与出处、止血/恢复状态、关联 Feature 和待确认业务决定。工具 `scan` 检查脱敏基线；它无法识别所有业务标识，需要结合事件内容检查。
