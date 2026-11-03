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
