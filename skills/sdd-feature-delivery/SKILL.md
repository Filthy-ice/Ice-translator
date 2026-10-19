---
name: sdd-feature-delivery
description: "在 Ice Translator SDD 中交付已批准 Feature：设计、任务、跨仓实现、验证和证据，遇到规格偏差退回评审。用于有 CHANGE-ID 的批准需求，不用于未经批准的新需求。"
---

# Feature Delivery

交付批准行为，保持 Requirement → Design → Task → Code/Test → Evidence → Acceptance 的可复查链。

## 恢复批准基线

使用用户指定工程；否则找当前目录及其上级的 `project/config.json`，找不到就询问用户工程路径。读工程 AGENTS.md、当前 Feature 的 change.json/spec.md；先运行 `status ID` 和 `validate ID`。路径未绑定、目标分支不符或用户有未提交修改时，先定位实际现场，不覆盖、重置或混入用户改动。
缺少批准、阻塞 Q 未解决、批准哈希漂移都不能进入实施。已存在且仍有效的批准/授权继续有效，不为流程重复请求。具体 JSON 格式按需读工程 `docs/workflow.md`。

## 设计与实施

1. APPROVED：按相关源码编写 design.md，明确仓库、入口、复用、状态流、外部契约、幂等/并发、异常、影响和回滚。实现选择可以自主决定；改变业务行为必须 revise 递增版本后再评审。
2. 编写 tasks.json 和 traceability.json；每条 REQ 有 Design 小节、可验证 Task、至少一个 required Verification。任务不引用规格外要求。真实外部契约必需时登记独立 required sandbox Verification，不用本地/Mock 代替它。
3. `plan-stamp ID` 自动写跨文件基线，`seal-plan ID` 封存计划，`transition ID IMPLEMENTING` 进入实施。仅技术计划变化用 `replan ID --reason ...`；业务变化用 revise。
4. 在登记代码仓库实施任务，验证业务边界和有意义的回归；先获得能区分正确/错误行为的测试，不为低影响改动制造镜像测试。通过实际文件引用执行 `task-done ID T-001 --code ...`。状态完成不等于验证成功。
5. 本地命令在 traceability.json 以 argv 声明，经授权范围核对后运行 `check ID V-001`。命令自身没有运行沙箱，不能把部署/生产写命令包装成本地测试。失败/超时保留证据。检查后源码变化会使证据过期，重新验证。
6. 只有实际授权的沙箱/生产验证才能执行外部动作，按授权范围和停止条件操作。脱敏文本用 `record ID V-002 --source ... --result ... --summary ...` 导入，写真实来源/时间/环境；人工导入不能代替有 command 的检查。

## 偏差、验证与验收

新事实推翻批准前提时，记录请求/响应、失败阶段和未执行动作，运行 `deviation ID --reason ...`，回到 SPEC_REVIEW。不得尝试未批准的替代业务写路径；可继续整理不依赖该决定的证据。
任务、代码引用和必需证据均有效后 `transition ID VERIFIED`，`validate ID`，`index`。给用户当前 ID@version/Spec 哈希、变更、验证层次、结果与未验证风险。实际业务验收后才 `accept ID --hash ... --by ... --reference ...`；本地测试通过不制造业务验收。
提交/PR 引用同一 ID@version 与哈希，按用户授权和业务仓库约定执行。tasks.md/evidence.md 是生成视图，编辑其 JSON 源；不会把单项局部方案提升为全局项目规则。
