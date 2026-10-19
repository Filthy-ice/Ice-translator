# 验证记录

移植日期：2026-10-19。本机 Python 3.12.9、Git 2.x、macOS 25.6.0（arm64）；工程运行要求 Python 3.11+。
来源：用户本地 SDD 工作区 `project-ssd@8b2df18` 的通用层，规格与代码同仓后在本仓库自持。

## 已完成

- 引擎回归：`python3 -B tests/test_workflow.py` → **Ran 32 tests, OK**（约 12 秒）。测试在系统临时目录建立独立 Git 仓库，
  不接触 `crates/` 与本工作区规格目录。
- 空工作区：`./sdd validate --offline`、`./sdd scan`、`./sdd index` 全部退出码 0；`index` 生成只含表头的 `project/STATE.md`。
- 工作区初始化：`./sdd init .` 生成 `project/`、`codebase/`、`features/`、`quick/`、`incidents/`、`templates/`，
  随后 `./sdd repo ice-translator --path . --branch main` 绑定本仓库；绝对路径写入被忽略的 `.sdd/repos.local.json`。
- 内容门禁：`./sdd scan` 通过；`python3 -B tools/check_staged.py` 通过；
  公司标识关键字扫描（内部域名、其他项目名、其业务术语）对拷入的 `tools/ templates/ skills/ tests/ docs/` 为空。
- 与来源的差异只有三处，且都是本仓库适配：`skills/*/SKILL.md` 描述里的工程名；
  `tools/sdd.py::scan` 跳过 `.DS_Store`（否则 `docs/.DS_Store` 会被当作二进制报"需人工检查"）；
  新增 `tools/check_traceability.py`（CHG-001 REQ-004 的必需验证，实测输出"编号一致：44 条"）。
- Python 语法检查：`python3 -m py_compile tools/*.py tests/*.py` 通过。

## 未验证与实际使用时再验证

- 未重新运行外部技能格式校验器；五个 `SKILL.md` 与 `agents/openai.yaml` 只做了改名后的通读。
  技能安装到仓库外目录（`tools/install_skills.py`）由用户按需执行，本工程不代跑。
- `.githooks/pre-commit` 需要 `git config core.hooksPath .githooks` 才生效；该配置由用户自行决定，
  未生效时门禁靠手工执行，不假设它拦住了提交。
- CHG-001 停在 SPEC_REVIEW：`approve` 需用户对当前哈希的明确确认，工具本身不做身份认证。
- 哈希与状态门禁保证记录一致性，不提供决定人身份认证，也不证明业务语义正确。
