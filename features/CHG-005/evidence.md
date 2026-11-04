Change: CHG-005
Version: v0.1
Spec-Hash: 79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

# Evidence（由 evidence.json 生成，勿直接修改）

- 2026-11-03T11:06:37+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0001-V-001.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-03T11:06:43+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-vision"] exit=0；附件：evidence/0002-V-002.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-03T11:06:45+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0003-V-003.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-03T11:06:46+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0004-V-004.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-03T11:06:52+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0005-V-005.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-03T11:06:52+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0006-V-006.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-03T11:06:52+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["cargo", "tree", "-p", "ice-vision", "-e", "normal"] exit=0；附件：evidence/0007-V-007.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-03T11:06:52+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0008-V-008.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-03T11:06:52+00:00 V-009 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0009-V-009.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-03T11:06:53+00:00 V-010 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0010-V-010.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-03T11:07:07+00:00 V-011 **observed** VERIFIED_RUNTIME/sandbox
  运行时两层已取到 sandbox 那一半：自绘度量 5 类×24 轮=120 张（平均 0.9287、最低 0.7059、单次 32~76ms，首图含框架预热 250ms）、samples_run 对着空 assets/samples 如实报'样本不足'并以 0 退出、nettop 对识别进程采 6 窗零条连接行且同命令 curl 阳性对照出现 Established、cargo tree -e normal 网络 crate 与未开 feature 命中 0 行。未闭环的两半：真界面上'已缩放'与'已忽略这次识别'的呈现、识别中可拖窗切页签，以及 HYPOTHESIS 单张 1080p<1s——120 张都是小图不构成延迟达标；FR-2.2 的 ≥0.92 仍记未闭环，没拿自绘度量冒充；附件：evidence/0011-V-011.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:51:20+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0012-V-001.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:51:21+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-vision"] exit=0；附件：evidence/0013-V-002.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:51:21+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0014-V-003.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:51:22+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0015-V-004.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:51:56+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0016-V-005.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:51:57+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0017-V-006.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:51:57+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["cargo", "tree", "-p", "ice-vision", "-e", "normal"] exit=0；附件：evidence/0018-V-007.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:51:57+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0019-V-008.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:51:57+00:00 V-009 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0020-V-009.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:51:57+00:00 V-010 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0021-V-010.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:55:00+00:00 V-011 **observed** VERIFIED_RUNTIME/sandbox
  运行时 sandbox 那一半，2026-11-04 重取（变更 M 打开 setAutomaticallyDetectsLanguage 之后）：① 自绘度量扩到 7 类 × 24 轮 = 168 张，平均 0.9140、最低 0.6835，白名单外语种 ko_plain rows=3/blocks=1/1.0000、ru_plain rows=8/blocks=8/0.6835 两条都非零——用户报的'韩文截图未识别到文字'那类在本地度量里不再返回空；② samples_run 对着仍为空的 assets/samples 如实报'样本不足'并以 0 退出，CSV 只有 filename,rows,blocks,elapsed_ms 四列，识别文本一个字不落；③ 零外发：nettop -p <识别进程> -L 6 -s 1 六个采样窗全是表头、零条连接行，同一命令换 curl 的阳性对照真出现流量行（bytes_in=260821；本机 nettop 这版 state 列为空，阳性判据是那一行本身而不是某个状态字）；④ cargo tree -p ice-vision -e normal 对 reqwest/hyper/rustls/native-tls/openssl/ureq 命中 0 行。仍未闭环：真界面上'已缩放'与'已忽略这次识别'的呈现、识别中能否拖窗切页签、HYPOTHESES 单张 1080p<1s（168 张都是小图，不构成延迟达标）；FR-2.2 的 ≥0.92 继续记'未闭环'，没拿自绘度量冒充它；附件：evidence/0022-V-011.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:09+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0023-V-001.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:09+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-vision"] exit=0；附件：evidence/0024-V-002.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:10+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0025-V-003.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:11+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0026-V-004.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:11+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0027-V-005.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:11+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0028-V-006.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:26+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["cargo", "tree", "-p", "ice-vision", "-e", "normal"] exit=0；附件：evidence/0029-V-007.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:26+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0030-V-008.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:26+00:00 V-009 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0031-V-009.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:26+00:00 V-010 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0032-V-010.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T06:56:27+00:00 V-011 **observed** VERIFIED_RUNTIME/sandbox
  运行时 sandbox 那一半，2026-11-04 重取并在文档回写后按同一份原始采集复录入（变更 M 打开 setAutomaticallyDetectsLanguage 之后）：① 自绘度量扩到 7 类 × 24 轮 = 168 张，平均 0.9140、最低 0.6835，白名单外语种 ko_plain rows=3/blocks=1/1.0000、ru_plain rows=8/blocks=8/0.6835 两条都非零——用户报的'韩文截图未识别到文字'那类在本地度量里不再返回空；② samples_run 对着仍为空的 assets/samples 如实报'样本不足'并以 0 退出，CSV 只有 filename,rows,blocks,elapsed_ms 四列，识别文本一个字不落；③ 零外发：nettop -p <识别进程> -L 6 -s 1 六个采样窗全是表头、零条连接行，同一命令换 curl 的 pid 采到真实流量行（bytes_in=260821；本机 nettop 这版 state 列为空，阳性判据是那一行本身而不是某个状态字）；④ cargo tree -p ice-vision -e normal 对 reqwest/hyper/rustls/native-tls/openssl/ureq 命中 0 行。仍未闭环：真界面上'已缩放'与'已忽略这次识别'的呈现、识别中能否拖窗切页签、HYPOTHESIS 单张 1080p<1s（168 张都是小图，不构成延迟达标）；FR-2.2 的 ≥0.92 继续记'未闭环'，没拿自绘度量冒充它；附件：evidence/0033-V-011.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:42:23+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0034-V-001.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:42:24+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-vision"] exit=0；附件：evidence/0035-V-002.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:42:24+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0036-V-003.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:42:25+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0037-V-004.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:42:38+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0038-V-005.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:42:39+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0039-V-006.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:42:39+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["cargo", "tree", "-p", "ice-vision", "-e", "normal"] exit=0；附件：evidence/0040-V-007.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:42:39+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0041-V-008.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:42:39+00:00 V-009 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0042-V-009.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:42:40+00:00 V-010 **passed** VERIFIED_CODE/local
  argv=["bash", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates"] exit=0；附件：evidence/0043-V-010.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97

- 2026-11-04T07:43:49+00:00 V-011 **observed** VERIFIED_RUNTIME/sandbox
  树指纹因本轮两处改动过期（crates/ice-app/src/main.rs 的 Dock 图标 0.836 网格留白 + docs/ 里 FR-5.4 立档），运行时那一半按 2026-11-04 同一份原始采集复录（附件即 0033 那份），没有新增也没有撤回任何运行时事实：① 自绘度量 7 类 × 24 轮 = 168 张，平均字符正确率 0.9140、最低 0.6835，ko_plain rows=3/blocks=1/1.0000、ru_plain rows=8/blocks=8/0.6835 均非零；② samples_run 对仍为空的 assets/samples 如实报『样本不足』并以 0 退出，CSV 只有 filename,rows,blocks,elapsed_ms 四列，识别文本一个字不落；③ 零外发：nettop -p 识别进程 -L 6 -s 1 六个采样窗全是表头、零条连接行，同一命令换 curl 的 pid 采到真实流量行（bytes_in=260821；本机 nettop 这版 state 列为空，阳性判据是那一行本身）；④ cargo tree -p ice-vision 对 reqwest/hyper/rustls/native-tls/openssl/ureq 命中 0 行。未闭环口径不变：真界面『已缩放』『已忽略这次识别』的呈现、识别中拖窗与切页签、单张 1080p<1s、W-3.1~W-3.6 走查、真实样图的 ≥0.92 —— FR-2.2 继续记未闭环，没拿自绘度量冒充它。本轮另两处（app_icon 网格、CHG-008 托盘规格包）属缺陷 D-19 与新立需求，证据在 V-001/V-004 与走查 W-1.25/W-2.40，不在本条判据范围。；附件：evidence/0044-V-011.txt；Spec：79354e07780e66f9bda80b5f5fe69aef4fc63cf767c95fe2cd71e502d0254d97
