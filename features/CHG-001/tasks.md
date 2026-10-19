Change: CHG-001
Version: v0.1
Spec-Hash: 213335f8c4be153a769af710f4e6d51127c1876ccf53684f68924c80823b8b3f

# Tasks（由 tasks.json 生成，勿直接修改）
- [x] T-001 工作区自持：init + repo 绑定，根目录与子目录均可运行 ./sdd | ice-translator | REQ-001
  Code/Test: sdd, tools/sdd.py, project/config.json, codebase/repositories.json
- [x] T-002 移植引擎回归测试并确认只操作隔离临时仓库 | ice-translator | REQ-002
  Code/Test: tests/test_workflow.py, tools/sdd.py
- [x] T-003 建立公开仓库内容门禁：scan、check_staged、外部文件关键字核查 | ice-translator | REQ-003
  Code/Test: tools/sdd.py, tools/check_staged.py, AGENTS.md, docs/verification.md
- [x] T-004 新增需求基线与追踪矩阵的编号一致性检查脚本 | ice-translator | REQ-004
  Code/Test: tools/check_traceability.py, docs/traceability.md
