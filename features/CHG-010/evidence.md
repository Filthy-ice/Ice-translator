Change: CHG-010
Version: v0.5
Spec-Hash: a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

# Evidence（由 evidence.json 生成，勿直接修改）

- 2026-11-09T08:40:42+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0001-V-001.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:40:43+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "selection"] exit=0；附件：evidence/0002-V-002.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:40:44+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "capture"] exit=0；附件：evidence/0003-V-003.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:40:44+00:00 V-004 **failed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl 'screencapture' crates/ice-app/src | wc -l) -eq 1 && ! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=1；附件：evidence/0004-V-004.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:40:44+00:00 V-007 **failed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl 'setLevel' crates/ice-app/src | wc -l) -le 1 && ! grep -Rn 'Fullscreen(true)' crates/ice-app/src"] exit=1；附件：evidence/0005-V-007.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:41:45+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0006-V-001.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:41:45+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "selection"] exit=0；附件：evidence/0007-V-002.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:41:46+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "capture"] exit=0；附件：evidence/0008-V-003.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:41:46+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '/usr/sbin/screencapture' crates/ice-app/src | wc -l) -eq 1 && ! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0009-V-004.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:41:46+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '\\.setLevel(' crates/ice-app/src | wc -l) -le 1 && ! (grep -Rn 'Fullscreen(true)' crates/ice-app/src --include='*.rs' | grep -vE ':[0-9]+: *//') | grep -q ."] exit=0；附件：evidence/0010-V-007.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:41:46+00:00 V-005 **passed** VERIFIED_RUNTIME/sandbox
  六轮仓外探针读数（H1/H2/H5 + 抬层三档台阶 + 每屏一扇 + esc 靠激活），文件在仓外、探针不进仓；附件：evidence/0011-V-005.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T08:42:12+00:00 V-005 **observed** VERIFIED_RUNTIME/sandbox
  六轮仓外探针读数的**转录汇总**（原始 out.log 在 /tmp，跑完即弃；探针不进仓）。含：H1/H2/H5 三条成立、抬层三档台阶（0,63,1920x953 → 0,31,1920x1080 → layer=25 整屏）、每屏一扇可行、esc 靠应用激活且激活异步、AppKit 与窗口服务器短时不一致、多屏抓屏必须点名 -D 且 -R 不吃负坐标。一条读数作废：Swift 自建进程没能激活，故'收尾后焦点回不回'仍无结论（W-6.6）；附件：evidence/0012-V-005.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:39:04+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0013-V-001.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:39:04+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "selection"] exit=0；附件：evidence/0014-V-002.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:39:05+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "capture"] exit=0；附件：evidence/0015-V-003.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:39:05+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '/usr/sbin/screencapture' crates/ice-app/src | wc -l) -eq 1 && ! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0016-V-004.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:39:05+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '\\.setLevel(' crates/ice-app/src | wc -l) -le 1 && ! (grep -Rn 'Fullscreen(true)' crates/ice-app/src --include='*.rs' | grep -vE ':[0-9]+: *//') | grep -q ."] exit=0；附件：evidence/0017-V-007.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:40:11+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0018-V-001.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:40:11+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "selection"] exit=0；附件：evidence/0019-V-002.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:40:11+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "capture"] exit=0；附件：evidence/0020-V-003.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:40:12+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '/usr/sbin/screencapture' crates/ice-app/src | wc -l) -eq 1 && ! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0021-V-004.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:40:12+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '\\.setLevel(' crates/ice-app/src | wc -l) -le 1 && ! (grep -Rn 'Fullscreen(true)' crates/ice-app/src --include='*.rs' | grep -vE ':[0-9]+: *//') | grep -q ."] exit=0；附件：evidence/0022-V-007.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:45:07+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0023-V-001.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:45:08+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "selection"] exit=0；附件：evidence/0024-V-002.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:45:08+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "capture"] exit=0；附件：evidence/0025-V-003.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:45:08+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '/usr/sbin/screencapture' crates/ice-app/src | wc -l) -eq 1 && ! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0026-V-004.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T10:45:09+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '\\.setLevel(' crates/ice-app/src | wc -l) -le 1 && ! (grep -Rn 'Fullscreen(true)' crates/ice-app/src --include='*.rs' | grep -vE ':[0-9]+: *//') | grep -q ."] exit=0；附件：evidence/0027-V-007.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:42:03+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0028-V-001.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:42:04+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "selection"] exit=0；附件：evidence/0029-V-002.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:42:04+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "capture"] exit=0；附件：evidence/0030-V-003.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:42:04+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '/usr/sbin/screencapture' crates/ice-app/src | wc -l) -eq 1 && ! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0031-V-004.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:42:04+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '\\.setLevel(' crates/ice-app/src | wc -l) -le 1 && ! (grep -Rn 'Fullscreen(true)' crates/ice-app/src --include='*.rs' | grep -vE ':[0-9]+: *//') | grep -q ."] exit=0；附件：evidence/0032-V-007.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:43:07+00:00 V-005 **passed** VERIFIED_RUNTIME/sandbox
  真应用端到端由 Agent 自己开车跑通并留读数（2026-11-09 深夜）：遮罩 layer=25 盖满 1920×1080、esc 退得出、拖框出八颗把手且药丸缝实测 3 点、对勾钉在原地 (1100,300) 300×234、拖动 3 秒不回弹、空格 234→200、右键恰好四项、复制交出 240226 字节图像（只记长度与哈希前 8 位）、悬停沿那扇窗四边亮框、单击整扇拿下 1404×919。三把全局热键与观感手感不在这条里，分别归 W-5.1 与 W-6.1~W-6.7；附件：evidence/0033-V-005.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:45:45+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0034-V-001.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:45:46+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "selection"] exit=0；附件：evidence/0035-V-002.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:45:46+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "capture"] exit=0；附件：evidence/0036-V-003.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:45:46+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '/usr/sbin/screencapture' crates/ice-app/src | wc -l) -eq 1 && ! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0037-V-004.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:45:46+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '\\.setLevel(' crates/ice-app/src | wc -l) -le 1 && ! (grep -Rn 'Fullscreen(true)' crates/ice-app/src --include='*.rs' | grep -vE ':[0-9]+: *//') | grep -q ."] exit=0；附件：evidence/0038-V-007.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16

- 2026-11-09T12:45:47+00:00 V-005 **passed** VERIFIED_RUNTIME/sandbox
  真应用端到端由 Agent 自己开车跑通并留读数（2026-11-09 深夜）：遮罩 layer=25 盖满 1920×1080、esc 退得出、拖框出八颗把手且药丸那道缝实测 3 点、对勾钉在原地 (1100,300) 300×234、拖动之后静置不回弹、空格 234→200、右键恰好四项、复制交出 240226 字节图像（只记长度与哈希前 8 位）、悬停沿那扇窗四边亮框、单击整扇拿下 1404×919。三把全局热键与观感手感不在这条里，分别归 W-5.1 与 W-6.1~W-6.7；附件：evidence/0039-V-005.txt；Spec：a17322e2ac3d254dc148b9dd625c7d7e499215870ea45872cad7611459c1be16
