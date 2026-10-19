Change: CHG-002
Version: v0.1
Spec-Hash: 88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

# Evidence（由 evidence.json 生成，勿直接修改）

- 2026-10-19T10:50:22+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core", "-p", "ice-providers"] exit=0；附件：evidence/0001-V-001.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T10:50:23+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-providers", "protect"] exit=0；附件：evidence/0002-V-002.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T10:50:23+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["grep", "-q", "状态：已接受", "docs/adr/ADR-004-machine-translate-endpoint.md"] exit=0；附件：evidence/0003-V-005.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T10:50:23+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0004-V-006.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T10:50:29+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0005-V-007.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T10:50:29+00:00 V-003 **passed** VERIFIED_RUNTIME/sandbox
  真实机翻端点 MyMemory：短句 1 段、超长 1060 字符切 3 段拼回，2 项 live 测试通过；附件：evidence/0006-V-003.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:57:02+00:00 V-004 **passed** VERIFIED_RUNTIME/sandbox
  真实 DeepSeek 通道：英文单句 1 段返回中文译文（1226ms）；含 URL/12 MB/getUserInfo 的句子经 protect→模型→restore 后三类片段逐字节回来（证据件已确认不含 Key）；附件：evidence/0007-V-004.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:58:08+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core", "-p", "ice-providers"] exit=0；附件：evidence/0008-V-001.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:58:08+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-providers", "protect"] exit=0；附件：evidence/0009-V-002.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:58:09+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0010-V-007.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:58:09+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["grep", "-q", "状态：已接受", "docs/adr/ADR-004-machine-translate-endpoint.md"] exit=0；附件：evidence/0011-V-005.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:58:09+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0012-V-006.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:59:45+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core", "-p", "ice-providers"] exit=0；附件：evidence/0013-V-001.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:59:45+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-providers", "protect"] exit=0；附件：evidence/0014-V-002.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:59:46+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0015-V-007.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:59:46+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["grep", "-q", "状态：已接受", "docs/adr/ADR-004-machine-translate-endpoint.md"] exit=0；附件：evidence/0016-V-005.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:59:46+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0017-V-006.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:59:46+00:00 V-003 **passed** VERIFIED_RUNTIME/sandbox
  真实机翻端点 MyMemory：单句 1 段；1060 字符切 3 段拼回且无 QUERY LENGTH 报错；单句 731/2133ms 抖动明显；附件：evidence/0018-V-003.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

- 2026-10-19T11:59:47+00:00 V-004 **passed** VERIFIED_RUNTIME/sandbox
  真实 DeepSeek 通道：单句 1 段返回译文；URL/12 MB/getUserInfo 经 protect→模型→restore 逐字节回来；证据件不含 Key；附件：evidence/0019-V-004.txt；Spec：88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c
