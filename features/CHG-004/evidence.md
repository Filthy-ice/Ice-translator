Change: CHG-004
Version: v0.7
Spec-Hash: 00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

# Evidence（由 evidence.json 生成，勿直接修改）

- 2026-10-27T14:56:25+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0001-V-001.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-27T14:56:29+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0002-V-002.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-27T14:56:30+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0003-V-003.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-27T14:56:44+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0004-V-004.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-27T14:56:44+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0005-V-005.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-27T14:56:44+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0006-V-006.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-27T14:56:44+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0007-V-007.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-27T14:56:44+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0008-V-008.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T06:27:46+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0009-V-001.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T06:27:50+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0010-V-002.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T06:27:51+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0011-V-003.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T06:27:52+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0012-V-004.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T06:27:52+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0013-V-005.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T06:27:52+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0014-V-006.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T06:27:52+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0015-V-007.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T06:27:53+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0016-V-008.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:08:16+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0017-V-001.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:08:19+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0018-V-002.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:08:21+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0019-V-003.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:08:21+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0020-V-004.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:08:21+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0021-V-005.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:08:21+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0022-V-006.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:08:22+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0023-V-007.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:08:22+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0024-V-008.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:16:18+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0025-V-001.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:16:19+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0026-V-002.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:16:20+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0027-V-003.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:16:20+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0028-V-004.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:16:20+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0029-V-005.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:16:21+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0030-V-006.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:16:21+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0031-V-007.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:16:21+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0032-V-008.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:25:28+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0033-V-001.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:25:28+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0034-V-002.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:25:30+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0035-V-003.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:25:30+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0036-V-004.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:25:30+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0037-V-005.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:25:30+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0038-V-006.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:25:31+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0039-V-007.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T07:25:31+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0040-V-008.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:04:18+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0041-V-001.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:04:21+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0042-V-002.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:04:22+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0043-V-003.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:04:23+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0044-V-004.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:04:23+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0045-V-005.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:04:23+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0046-V-006.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:04:23+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0047-V-007.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:04:24+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0048-V-008.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:49:23+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0049-V-001.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:49:26+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0050-V-002.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:49:27+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0051-V-003.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:49:28+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0052-V-004.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:49:28+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0053-V-005.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:49:28+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0054-V-006.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:49:28+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0055-V-007.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T08:49:28+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0056-V-008.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T09:42:06+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0057-V-001.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T09:42:10+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0058-V-002.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T09:42:12+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0059-V-003.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T09:42:12+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0060-V-004.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T09:42:12+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0061-V-005.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T09:42:13+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0062-V-006.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T09:42:13+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0063-V-007.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T09:42:13+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0064-V-008.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T10:09:09+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0065-V-001.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T10:09:10+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0066-V-002.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T10:09:11+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0067-V-003.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T10:09:11+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0068-V-004.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T10:09:12+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0069-V-005.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T10:09:12+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0070-V-006.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T10:09:12+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0071-V-007.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577

- 2026-10-28T10:09:12+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0072-V-008.txt；Spec：00de0e1d123cadd16c7994cd9c3524a85fb801a68dc85d75c609416199b76577
