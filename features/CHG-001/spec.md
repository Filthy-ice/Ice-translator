# CHG-001 · 引入 SDD 工作流引擎到 Ice-translator 仓库

## 原始意图与来源

2026-10-19 用户在 P1/P2 验收后提出：本地另有一套 SDD（规格驱动开发）工作区，希望把其中的**通用模板层**
提取进 Ice-translator，让后续每个阶段的改动都有结构化的批准、任务、追踪与证据记录，而不是只靠
`docs/07-acceptance.md` 的表格叙述。

来源快照：用户本地工作区 `project-ssd@8b2df18`（其远端是公司内部 Git 服务，地址按要求不记录进本仓库）。
用户明确的边界是"所有 robot 相关的为公司项目，不能放入"，提取范围是"整套引擎"。

可复查事实：来源工作区中命中公司标识的只有 `project/config.json`、`project/ROADMAP.md`、
`.sdd/repos.local.json` 与 `codebase/` 下的全部文件（含其登记的另一个项目目录）；
`tools/`、`templates/`、`skills/`、`docs/workflow.md`、`docs/verification.md`、`docs/design-decisions.md`、
`tests/`、`.githooks/` 经关键字扫描无公司标识、无凭据。本工程只取后者，`project/` 与 `codebase/` 的内容
由 `./sdd init` 在本仓库重新生成为 Ice Translator 自己的实例。

## 目标与非目标

目标：

- 本仓库成为自持的 SDD 工作区：规格目录与 Rust 代码同仓，`./sdd` 在仓库根目录可直接使用；
- 引擎的回归测试随仓库提交，任何一次改动都能本地重跑证明工具链没坏；
- 建立"公开仓库内容门禁"：外部引入的文件在提交前必须过敏感信息扫描与来源标识核查；
- P3 起的每个阶段变更包（`features/CHG-0NN/`）有批准哈希、任务映射、追踪与证据链。

非目标：

- 不改变 `crates/` 的任何 Rust 代码与 `docs/00~07` 基线内容（除引用本工作流的说明）；
- 不引入数据库、Web 平台、常驻 Agent 运行时或外部服务；
- 不在本变更包内实现打包隔离（属 P8 / CHG-007），只登记规则；
- 不安装技能到仓库外的目录（`tools/install_skills.py` 由用户按需自行执行）。

仓库范围：`ice-translator`（本仓库自身），目标分支 `main`。

## 事实与假设

| 标签 | 结论 | 来源 / commit / 环境 / 时间 |
|---|---|---|
| VERIFIED_CODE | 引擎只依赖 Python 3.11+ 与 Git 标准库，无第三方包 | `tools/sdd.py` 第 1-12 行 import 列表；本机 Python 3.12.9，2026-10-19 |
| VERIFIED_RUNTIME | 移植后 32 项 unittest 全绿，测试只操作隔离临时 Git 仓库 | `python3 -B tests/test_workflow.py` → `Ran 32 tests ... OK`，2026-10-19 |
| VERIFIED_RUNTIME | 空工作区下 `validate --offline`、`scan`、`index` 全部通过 | 本仓库根目录执行输出，2026-10-19 |
| VERIFIED_CODE | `init` 生成的实例文件不含来源工作区内容 | `tools/sdd.py::init` 只写死通用文案；`project/config.json` 名称随后改为 `Ice Translator SDD` |
| VERIFIED_CODE | `scan` 会遍历 `docs/` 等目录，把非 UTF-8 文件报为待人工检查 | `tools/sdd.py::scan`；实测被 `docs/.DS_Store` 触发 |
| VERIFIED_CODE | 本机绝对路径只落在 `.sdd/repos.local.json`，已被 `.gitignore` 忽略 | `.gitignore` 的 `# SDD 工作区` 段；`./sdd repo` 写入位置见 `tools/sdd.py` 第 319-322 行 |
| HYPOTHESIS | 公开仓库不会因此暴露任何第三方项目内容 | 依赖 REQ-003 的门禁持续生效，需每次提交时验证，不是一次性结论 |

## 行为与验收

### REQ-001: 仓库根目录即可用的 SDD 工作区

WHEN 在仓库根目录执行 `./sdd validate --offline` 与 `./sdd scan`
THEN 系统 SHALL 退出码 0 并给出通过结论，且不依赖仓库外的任何路径或工作区。

WHEN 从 `crates/` 等子目录执行 `./sdd --root . status CHG-001`
THEN 系统 SHALL 通过 `project/config.json` 定位工作区并输出当前状态，不要求设置环境变量。

边界：`.sdd/` 缺失（换机器、新克隆）时 `./sdd repo` 可重建映射，其余命令不得因缺目录而误判为已初始化。

### REQ-002: 引擎自带回归测试可重跑

WHEN 执行 `python3 -B tests/test_workflow.py`
THEN 系统 SHALL 全部通过，且测试只在系统临时目录里创建独立 Git 仓库，不读写 `crates/`、`docs/` 与本工作区的规格目录。

异常路径：若未来修改 `tools/sdd.py` 导致任一测试失败，该修改不得进入提交。
不适用说明：本 REQ 无并发与权限维度（单进程本地脚本）。

### REQ-003: 公开仓库的内容门禁

WHEN 从仓库外部引入任何文件
THEN 系统 SHALL 在提交前同时满足：`./sdd scan` 通过、`python3 -B tools/check_staged.py` 通过、
且按公司标识关键字（内部域名、其他项目名与业务术语）扫描结果为空；三者任一不过即不得提交。

WHEN 扫描命中敏感模式
THEN 工具 SHALL 只报告文件、行号与规则名，不打印命中内容本身（`tools/sdd.py::scan` 的既有行为）。

边界：这是基线规则而非完整 DLP，真实项目名、自定义内部域名、分段凭据仍需人工判断；
无法自动判定的二进制文件按"需人工检查"处理，不默认放行。

### REQ-004: 基线文档与变更包不互相替代

WHEN 需求基线（`docs/01-requirements.md`）与追踪矩阵（`docs/traceability.md`）的需求编号集合不一致
THEN 系统 SHALL 由 `python3 -B tools/check_traceability.py` 以非零退出码报出缺失或多余的编号。

WHEN 变更记录涉及需求语义变化
THEN 系统 SHALL 先改 `docs/` 基线并在 `07-acceptance.md` 变更登记原因，再在对应 Feature 的 spec 中引用编号；
Feature 不另立第二套需求事实源。

## 未决问题

无阻塞问题。`project/ROADMAP.md` 已把 P3~P8 预分配为 CHG-002~CHG-007，若某阶段实际拆成多个变更包，
以新建 ID 为准并回写 ROADMAP，不预先创建空壳。

## 允许动作与停止条件

批准范围仅 local-delivery：在本仓库内编辑规格、代码与工作区文件，本地跑测试，commit 并 push 到 `origin/main`。
不包含：改写 `~/Desktop/project-ssd` 或任何来源工作区（只读）、写入其他仓库、部署、外部平台操作。

事实推翻前提时（例如发现来源文件其实含公司标识，或引擎测试依赖外部服务）记录 SPEC_DEVIATION，
退回 SPEC_REVIEW 重新评审，不自行替换成"看起来更安全"的临时方案。

## 时间估算

开发与移植：0.5 人日内（已完成实际拷贝、实例初始化与文案适配）。
验证：引擎回归 32 项约 12 秒，加工作区命令与门禁扫描约 1 分钟，可重复。
外部等待：批准需用户确认当前 Spec 哈希，无其他外部依赖。
依据：本次实测的命令耗时与已改动文件数（新增约 20 个文件，均为工作区层，无 Rust 代码变更）。
