---
name: sdd-feature-intake
description: "在 Ice Translator SDD 中调查新需求、建立可评审 Spec，定位跨仓入口并区分事实与假设。用于需求 Intake 或明确要求先写规格；不用于已批准需求的交付或普通即时小修。"
---

# Feature Intake

把原始诉求变成下一位 Agent 能接手的业务事实源，保留未知项和决策边界。

## 找到工作区

使用用户指定的 SDD 工程；未指定时先查当前目录及其上级的 `project/config.json`。找不到则说明缺失并请用户给出工程路径，不能凭旧聊天或历史关键词创建业务事实。
读工作区 `AGENTS.md`、`project/STATE.md`、相关 codebase 地图，随后只读当前 Feature 和直接关联的历史证据。工具在工程根目录用 `./sdd ...` 运行，从其他目录启动时用 `sdd --root <工程实际路径> ...`。

## 调查与产物

1. 有 CHANGE-ID 先 `status ID`，避免重复建立或覆盖；新需求通过 `repo` 登记实际代码根目录/目标分支后 `new ID --title ... --repos ...`。先核对真实入口和仓库自身指令；相似旧需求不能证明本次仍在同仓。登记只建立映射，不切分支、不改源码。
2. 原始诉求、源码、运行日志和业务确认分开记录。每项事实标 VERIFIED_CODE、VERIFIED_RUNTIME、BUSINESS_CONFIRMED 或 HYPOTHESIS，附可复查来源/提交/环境/时间。读到代码不证明生产配置生效，Mock 不证明外部平台接受。
3. 按 `templates/spec.md` 填业务目标、非目标、仓库范围、稳定 `### REQ-001: 标题`、WHEN/THEN/SHALL 验收、边界/回归、验证层次、工时范围和停止条件。无关的目录或验证层不强加到需求。
4. 把会改变业务验收的未知项同步到 `change.json.questions`（id、blocking、resolution、source）。可验证的技术问题自主调查；新增业务选择交给用户。问题关闭必须有真实确认或事实来源，不能代替人做决定。
5. 草稿充分后 `transition ID CLARIFIED`，可评审时 `transition ID SPEC_REVIEW`，`hash ID` 获取原字节 SHA-256，`index` 刷新导航。INTAKE 到 SPEC_REVIEW 不修改业务代码；可编写测试计划，但不实施业务变更。

## 交接

呈现 Spec 路径、ID@version、完整哈希、影响范围、需要确认的少量业务问题和估算依据。用户已明确批准当前版本和范围时，按实际出处运行 `approve ID --hash ... --by ... --reference ...`，无需重复确认。没有当前版本批准时停在评审，不能自称批准。
批准只记录 local-delivery；外部写操作按会话实际授权。获批后交付走 sdd-feature-delivery，数据格式按需读取工程 `docs/workflow.md`。
