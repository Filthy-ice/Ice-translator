Change: CHG-009
Version: v0.6
Spec-Hash: f08b72302fcf9864297894202527d308da8addce1b8615ca3906a6912c447a01

# Evidence（由 evidence.json 生成，勿直接修改）

- 2026-11-09T02:23:38+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0001-V-001.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:18+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0002-V-002.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:18+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core", "hotkey"] exit=0；附件：evidence/0003-V-003.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:19+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0004-V-004.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:19+00:00 V-005 **failed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl 'screencapture' crates/ice-app/src | wc -l) -eq 1"] exit=1；附件：evidence/0005-V-005.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:19+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["cargo", "tree", "-p", "ice-app", "-e", "normal"] exit=0；附件：evidence/0006-V-006.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:19+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0007-V-007.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:51+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0008-V-001.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:52+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0009-V-002.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:53+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core", "hotkey"] exit=0；附件：evidence/0010-V-003.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:53+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0011-V-004.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:53+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '/usr/sbin/screencapture' crates/ice-app/src | wc -l) -eq 1"] exit=0；附件：evidence/0012-V-005.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:53+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["cargo", "tree", "-p", "ice-app", "-e", "normal"] exit=0；附件：evidence/0013-V-006.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:24:54+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0014-V-007.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:26:11+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0015-V-001.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:26:12+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0016-V-002.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:26:12+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core", "hotkey"] exit=0；附件：evidence/0017-V-003.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:26:12+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0018-V-004.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:26:12+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '/usr/sbin/screencapture' crates/ice-app/src | wc -l) -eq 1"] exit=0；附件：evidence/0019-V-005.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:26:13+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["cargo", "tree", "-p", "ice-app", "-e", "normal"] exit=0；附件：evidence/0020-V-006.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:26:13+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0021-V-007.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:28:24+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0022-V-001.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:28:25+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0023-V-002.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:28:26+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core", "hotkey"] exit=0；附件：evidence/0024-V-003.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:28:26+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0025-V-004.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:28:26+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '/usr/sbin/screencapture' crates/ice-app/src | wc -l) -eq 1"] exit=0；附件：evidence/0026-V-005.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:28:26+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["cargo", "tree", "-p", "ice-app", "-e", "normal"] exit=0；附件：evidence/0027-V-006.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T02:28:27+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0028-V-007.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T03:21:17+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0029-V-001.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T03:21:18+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0030-V-002.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T03:21:20+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core", "hotkey"] exit=0；附件：evidence/0031-V-003.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T03:21:20+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0032-V-004.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T03:21:20+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '/usr/sbin/screencapture' crates/ice-app/src | wc -l) -eq 1"] exit=0；附件：evidence/0033-V-005.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T03:21:21+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["cargo", "tree", "-p", "ice-app", "-e", "normal"] exit=0；附件：evidence/0034-V-006.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f

- 2026-11-09T03:21:21+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0035-V-007.txt；Spec：890b9cbffdd4202a56e9f20d1e5ce2e9c33454cc45ce59d269c023051a26137f
