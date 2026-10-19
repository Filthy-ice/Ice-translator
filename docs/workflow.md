# 工作流与机器记录

> 本仓库的适配：Ice Translator 是**规格与代码同仓**的单仓工作区，`ice-translator` 既是业务仓库也是 SDD 工程根。
> 因此下文的"业务仓库"就是本仓库，`codebase/repositories.json` 只登记它一个；
> 提交时规格变更与代码变更进同一个 commit（原文的"SDD 工程单独提交"针对跨仓场景，此处不适用）。
> 数据格式、状态机与门禁规则与来源实现一致，未做修改。

## 权威来源

spec.md 是应有业务行为；change.json 管状态、版本、仓库范围、Q 表和批准/验收出处。
design.md 是实现选择；tasks.json 和 traceability.json 是可执行追踪记录。
tasks.md、evidence.md、project/STATE.md 是生成视图，不直接编辑。
手工更新 JSON 后可用 `refresh ID` 恢复对应生成视图；它不修改批准或任务进度。
每份资料负责不同事实，不把源码、日志、业务确认或假设互相替代。

## 状态与门禁

| 状态 | 允许工作 | 进入下一阶段 |
|---|---|---|
| INTAKE | 读需求、文档、相关源码，定位真实仓库和分支 | transition CLARIFIED |
| CLARIFIED | 写业务行为和验收；把未知项放 Q 表 | transition SPEC_REVIEW |
| SPEC_REVIEW | 完成可评审规格，呈现版本、哈希和需人决定的问题 | 实际确认后 approve |
| APPROVED | 写 Design、Tasks、验证计划 | plan-stamp + seal-plan |
| DESIGNED | 检查仓库路径、分支和现有修改 | transition IMPLEMENTING |
| IMPLEMENTING | 只执行映射任务、验证、保存证据 | 完成任务与必需证据后 transition VERIFIED |
| VERIFIED | 提供证据供业务验收；外部行动按实际授权 | 实际验收后 accept |
| ACCEPTED | 保留历史批准/证据/验收基线 | 下一变更新建 ID |

默认 approve 仅记录 local-delivery 范围。不能自行制造批准；会话里对当前版本、业务行为和边界的明确授权可作为 reference，不要求重复确认。对旧版的批准不能自动延续到新版本。
`--by` 是决定人的标识，`--reference` 是脱敏的确认原文/可复查出处。CLI 记录出处并检查内容完整性，不具备身份认证或防篡改权限隔离。可写文件的进程仍可以绕开流程；多人协作可在 Git 托管方增加评审保护。

Spec SHA-256 对文件原始字节计算，空白、换行变动也会失效。批准快照位于 Feature 的 approvals/；业务范围/Q 表/risk_assessment 另有 context_hash，目标分支另存。Design/Tasks/追踪计划用 plan_hash 封存。Task status/code_refs 属于实施进度，不进入 plan_hash。
追踪验证证明映射存在，不能自动判断需求语义、测试质量或根因正确性，这部分仍由 Agent/评审结合代码证据判断。

## Q 表

在 change.json.questions 编辑：

```json
[
  {"id":"Q-001","blocking":true,"resolution":null,"source":null}
]
```

影响业务验收的未知项 blocking=true。解决后写实际答案与业务确认或可验证事实来源。已关闭问题缺来源、阻塞问题未关闭均不得批准。不要把待执行的验证当成已通过；无法预先验证的外部前提写入 Spec 的失败与停止条件。

## Tasks 与追踪计划

先创建 Feature 并批准；把 tasks.json、traceability.json 的业务内容填好，再执行 plan-stamp 自动写 baseline。不要手抄哈希。

tasks.json（baseline 由工具生成）：

```json
{
  "baseline": null,
  "tasks": [
    {"id":"T-001","title":"实现并验证返回行为","requirements":["REQ-001"],"repo":"api","status":"todo","code_refs":[]}
  ]
}
```

traceability.json：

```json
{
  "baseline": null,
  "design": [
    {"requirement":"REQ-001","section":"## REQ-001 实现与验证"}
  ],
  "verifications": [
    {"id":"V-001","requirements":["REQ-001"],"repo":"api","kind":"VERIFIED_CODE","environment":"local","required":true,"command":["python3","-B","test_behavior.py"]}
  ]
}
```

每条 Requirement 必须映射 Design 小节、Task、至少一个 required 验证，所有引用必须存在，仓库不得越界。Task id/Verification id 必须唯一。命令为 argv 数组，在登记仓库根目录执行，不经过 shell。
`check` 运行的是你列出的命令；命令自身可能产生副作用，运行前确认属于已授权的本地验证，工具不提供网络/进程沙箱。模型测试/外部服务 Mock 仍归 VERIFIED_CODE，environment=mock；VERIFIED_RUNTIME 仅标真实 sandbox/production 证据，BUSINESS_CONFIRMED 对应 business；HYPOTHESIS 不能用于 required 验证。

外部契约必需时，另增 required Verification，不写 command：

```json
{"id":"V-002","requirements":["REQ-001"],"repo":"api","kind":"VERIFIED_RUNTIME","environment":"sandbox","required":true}
```

这会使 VERIFIED 等待实际沙箱证据。本地通过不会代替它。工具不执行沙箱、生产写入或部署；取得实际授权并完成验证后，导入脱敏文本附件：

```bash
./sdd record CHG-001 V-002 --source /tmp/sanitized-sandbox.txt --result passed --summary '真实沙箱契约验证，来源与环境见附件'
```

手工导入是声明式证据，保留真实来源与时间，由评审校对；不能把 Mock 或假设包装成运行证据。有 command 的 Verification 必须用 check，不能手工声明通过。

## 验证与恢复

```bash
./sdd check CHG-001 V-001
./sdd task-done CHG-001 T-001 --code src/handler.java src/test/HandlerTest.java
./sdd transition CHG-001 VERIFIED
./sdd status CHG-001
./sdd validate CHG-001
./sdd index
```

检查前后对 Git 跟踪文件和非忽略新文件计算内容哈希（含执行位），Git SHA 和 dirty 状态单独留存。构建/缓存目录应在代码仓库正确忽略；测试修改源码或非忽略文件时本次检查不作为有效证据。失败或超时保存失败记录并返回非零；同一 Verification 的最新当前基线结果决定门禁。
规格与代码同仓时（本仓库布局），账本自身的写入不计入被测哈希：排除 `features/ quick/ incidents/ project/STATE.md .sdd/`，
否则 `check` 写完证据就使自己刚记录的证据过期。排除集只在登记仓库等于工作区根时生效，分开布局与来源行为一致；
被测代码（`crates/`、`tools/`、`docs/`、`templates/`）的变动仍会使证据过期。
源码变动后旧证据过期；已 VERIFIED 需 `replan --reason`，再次封存并进入 IMPLEMENTING 后重跑。基线/计划变化要求新证据，不复用旧通过结果。
ACCEPTED 保留历史代码哈希和验收，不与将来的业务仓库内容比较。`validate --offline` 只校验存档的结构、批准/计划与附件完整性，用于 CI/换机；不能证明当前本机代码通过，也不能用来进入 VERIFIED 或 ACCEPTED。

## 偏差与重新规划

```bash
./sdd deviation CHG-001 --reason '沙箱拒绝原 reopen 前提，停止后续 handover'
./sdd revise CHG-001 --version v0.2 --reason '新业务规则需要重新评审'
./sdd replan CHG-001 --reason '行为不变，调整实现与测试计划'
```

deviation 立即撤销当前批准并回到评审，保留原快照、历史和证据；调整规格必须 revise 递增版本。不能重新使用同一版同一快照，需升版。replan 保持行为批准有效，允许技术方案变化；设计若影响业务行为，则用 revise。

## Quick 与 Incident

Quick 只适合单仓、规则已明确、风险极低改动，仍须批准与验收，可合并 Design 内容到简短实现段但保留机器追踪链。
`new ... --quick` 创建 risk_assessment；逐项 false 之前不能批准。message、rpc、business-state、redis、database、config、permission、personal-data、external-write、cross-repo 中任何一项涉及或不确定都走完整 Feature。将 Quick 保留为待升级记录并关联新的 Feature，不把风险改成 false 来绕门禁。
Incident 使用 templates/incident.md 在 incidents/INC-ID/ 保存时间线、事实/假设、影响、止血授权。确认根因后建立单独 Feature，双方相互链接；事故现象不自动成为永久规则。

## 敏感信息与提交

`scan` 检查 project、codebase、features、quick、incidents、templates、skills、docs、examples 的文本，覆盖常见令牌、私钥头、明文凭据、内网 IP、邮件、国内手机号和常见内部域名。只报告位置/规则，不打印命中内容；证据附件的这些模式自动替换为 [REDACTED]。摘要、Spec、确认出处仍需自行脱敏。
这是基线规则，不是完整 DLP；真实工单 ID、自定义内部域名、分段或未知格式凭据仍需业务侧检查，可用 Gitleaks/DLP 增强。不要把真实账号、内部链接、凭证或未经脱敏的原始日志放入 Git。
提交/PR 写 CHANGE-ID@version + Spec SHA-256；代码仓库照各自习惯提交，SDD 工程单独提交。工具不自动提交/推送/部署，也不改业务仓库的指令或配置。
