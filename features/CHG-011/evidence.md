Change: CHG-011
Version: v0.1
Spec-Hash: e2567d90c527d1f2b27a33bc5ace190793e3f5661779587b5b613779d3915873

# Evidence（由 evidence.json 生成，勿直接修改）

- 2026-11-12T04:26:14+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0001-V-001.txt；Spec：b96416c677dad0355940d9c636edca045404efb93b40338e0a9525297e6d61f6

- 2026-11-12T04:27:06+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "bar"] exit=0；附件：evidence/0002-V-002.txt；Spec：b96416c677dad0355940d9c636edca045404efb93b40338e0a9525297e6d61f6

- 2026-11-12T04:27:06+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "pill"] exit=0；附件：evidence/0003-V-003.txt；Spec：b96416c677dad0355940d9c636edca045404efb93b40338e0a9525297e6d61f6

- 2026-11-12T04:27:07+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "pin"] exit=0；附件：evidence/0004-V-004.txt；Spec：b96416c677dad0355940d9c636edca045404efb93b40338e0a9525297e6d61f6

- 2026-11-12T04:27:07+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app", "state"] exit=0；附件：evidence/0005-V-005.txt；Spec：b96416c677dad0355940d9c636edca045404efb93b40338e0a9525297e6d61f6

- 2026-11-12T04:27:07+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core", "config"] exit=0；附件：evidence/0006-V-006.txt；Spec：b96416c677dad0355940d9c636edca045404efb93b40338e0a9525297e6d61f6

- 2026-11-12T04:27:07+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "test $(grep -Rl '\\.setLevel(' crates/ice-app/src | wc -l) -le 1 && ! grep -Rn 'cfg(target_os' crates/ice-app/src/views"] exit=0；附件：evidence/0007-V-007.txt；Spec：b96416c677dad0355940d9c636edca045404efb93b40338e0a9525297e6d61f6

- 2026-11-12T04:27:19+00:00 V-009 **passed** VERIFIED_RUNTIME/sandbox
  T-001 六轮探针读数转录（spec.md 事实13/14/20/21，探针2026-11-09晚跑完、仓外取样后即时删除）：三格全开真透明(244,0,0)⇒H1成立；透明区按整扇窗矩形吃点击、五点逐点吻合⇒H2成立；进程格与根窗格同一字段；注入坐标准。量不到三格明写：跟手与闪归W-7.2、本机无高分屏2×档、主窗失焦半透明与进程alpha同开归T-005真应用量；附件：evidence/0008-V-009.txt；Spec：b96416c677dad0355940d9c636edca045404efb93b40338e0a9525297e6d61f6

- 2026-11-12T04:27:33+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["./sdd", "validate", "CHG-011", "--offline"] exit=0；附件：evidence/0009-V-008.txt；Spec：b96416c677dad0355940d9c636edca045404efb93b40338e0a9525297e6d61f6
