Change: CHG-008
Version: v0.2
Spec-Hash: UNAPPROVED

# Evidence（由 evidence.json 生成，勿直接修改）

- 2026-11-04T09:48:28+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0001-V-001.txt；Spec：bb8b66e8f5c5dd49e712f8ca988089c3cff268f29bc74c1be75330e2c282d354

- 2026-11-04T09:48:39+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0002-V-002.txt；Spec：bb8b66e8f5c5dd49e712f8ca988089c3cff268f29bc74c1be75330e2c282d354

- 2026-11-04T09:48:39+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0003-V-003.txt；Spec：bb8b66e8f5c5dd49e712f8ca988089c3cff268f29bc74c1be75330e2c282d354

- 2026-11-04T09:48:46+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0004-V-004.txt；Spec：bb8b66e8f5c5dd49e712f8ca988089c3cff268f29bc74c1be75330e2c282d354

- 2026-11-04T09:48:46+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "tree", "-p", "ice-app", "-e", "normal"] exit=0；附件：evidence/0005-V-005.txt；Spec：bb8b66e8f5c5dd49e712f8ca988089c3cff268f29bc74c1be75330e2c282d354

- 2026-11-04T09:48:46+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0006-V-006.txt；Spec：bb8b66e8f5c5dd49e712f8ca988089c3cff268f29bc74c1be75330e2c282d354

- 2026-11-04T09:48:47+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0007-V-007.txt；Spec：bb8b66e8f5c5dd49e712f8ca988089c3cff268f29bc74c1be75330e2c282d354

- 2026-11-04T09:48:47+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0008-V-008.txt；Spec：bb8b66e8f5c5dd49e712f8ca988089c3cff268f29bc74c1be75330e2c282d354

- 2026-11-04T09:49:00+00:00 V-009 **observed** VERIFIED_RUNTIME/sandbox
  真机运行（本机 macOS 26.6.2，release 构建）拿到的一半：① 托盘在第一帧真的装出来了——日志行『托盘这一档定下来了 next=Available』（ 里 Unknown→install() 的定档日志），同一次启动只有这一行，进程唯一（pid 65572，单实例锁先于托盘）；② 日志全文只有锁路径与这一行档位，无窗口内容、无输入文本、无 Key，符合 NFR-3。尚未定论的七件里只有『装出来了』这一件有当刻证据：**唯一一枚不重复、深浅色菜单栏都看得见、左键真的唤回、右键三项各点一次、关窗后 Dock 磁贴真的消失且唤回后回来、唤回后内容与尺寸原样**——这六件都要真人看，未由代理判定；**隐藏静置十分钟的 CPU < 1% 未采到**（先前那次   PID TTY           TIME CMD 采样时窗口从未关过，属可见空闲样本，不能顶这一格）。进程仍在运行等用户按 docs/06 W-2.40 走查；附件：evidence/0009-V-009.txt；Spec：bb8b66e8f5c5dd49e712f8ca988089c3cff268f29bc74c1be75330e2c282d354

- 2026-11-04T09:49:22+00:00 V-009 **observed** VERIFIED_RUNTIME/sandbox
  真机运行（本机 macOS 26.6.2，release 构建）拿到的一半；覆盖上一条 V-009（那条把两处反引号当成命令替换，app::poll_tray 与 ps 的字样被 shell 输出顶掉了，事实内容与本条一致）。① 托盘在第一帧真的装出来了——日志行『托盘这一档定下来了 next=Available』，即 app::poll_tray 里 Unknown → tray::install() 的定档日志；同一次启动只有这一行，进程唯一（pid 65572，单实例锁先于托盘）。② 日志全文只有锁路径与这一行档位，无窗口内容、无输入文本、无 Key（NFR-3）。③ 真机七件里只有『装出来了』这一件有当刻证据：唯一一枚不重复、深浅色菜单栏都看得见、左键真的唤回、右键三项各点一次、关窗后 Dock 磁贴真的消失且唤回后回来、唤回后内容与尺寸原样——这六件都要真人看，不由代理判定。④ 隐藏静置十分钟的 CPU 未采到：先前那次采样时窗口从未关过，属可见空闲样本，不能顶这一格。进程仍在运行，等用户按 docs/06 的 W-2.40 走查；附件：evidence/0010-V-009.txt；Spec：bb8b66e8f5c5dd49e712f8ca988089c3cff268f29bc74c1be75330e2c282d354
