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
