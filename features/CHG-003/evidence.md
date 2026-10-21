Change: CHG-003
Version: v0.1
Spec-Hash: 84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

# Evidence（由 evidence.json 生成，勿直接修改）

- 2026-10-20T10:19:48+00:00 V-011 **passed** VERIFIED_RUNTIME/sandbox
  真机界面实测两条真实通道：机翻 MyMemory 695–1512ms（3 次）、AI DeepSeek 708–988ms（3 次，其中一次 cache_hit=true elapsed_ms=0）；引擎来回切换均出译文。同语言对由新增的 SameLanguage 本地拦下（status=0，未发请求）。日志不含源文/译文/查询串/密钥明文，Key 只有 key_len=35 与 hash8。；附件：evidence/0001-V-011.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-20T10:23:20+00:00 V-010 **passed** VERIFIED_RUNTIME/sandbox
  真机三项：① 启动时序——窗口 09:24:49 建好，钥匙串在后台线程 09:25:07 才返回（用户在系统授权框上花了 18s），期间窗口持续渲染，'启动卡死'已修；② 单实例双开——第二个实例退出码 1、中文提示含持锁者 PID、config.toml 与 history.jsonl 的 mtime+size 前后一致、锁文件未被第二个实例删掉；③ 空闲 30.2s 消耗 CPU 恰好 0.000s（0.0000%，阈值 <1%），RSS 93.3MB。；附件：evidence/0002-V-010.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-20T10:31:07+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0003-V-001.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-20T10:31:07+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0004-V-002.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-20T10:31:08+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0005-V-003.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-20T10:31:08+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0006-V-004.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-20T10:31:08+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0007-V-005.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-20T10:31:08+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0008-V-006.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-20T10:31:09+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0009-V-007.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-20T10:31:09+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["sh", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates 2>/dev/null"] exit=0；附件：evidence/0010-V-008.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T08:17:23+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0011-V-001.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T08:17:34+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0012-V-002.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T08:17:35+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0013-V-003.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T08:17:36+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0014-V-004.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T08:17:36+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0015-V-005.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T08:17:36+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0016-V-006.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T08:17:36+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0017-V-007.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T08:17:37+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["sh", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates 2>/dev/null"] exit=0；附件：evidence/0018-V-008.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T08:29:40+00:00 V-010 **passed** VERIFIED_RUNTIME/sandbox
  五个子项全齐（覆盖 2026-10-20 首采）：① 启动时序——窗口 09:24:49 建好、钥匙串后台线程 09:25:07 才返回，期间持续渲染；② 单实例双开——第二实例退出码 1、提示含持锁者 PID，config/history 的 mtime+size 未变、锁未被误删；③ 空闲 30.2s 消耗 CPU 0.000s（0.0000%），RSS 93.3MB；④ 18,000 字符粘贴——第二轮实测 71,211 字符/6 段/53.4s 出译文（暴露 D-4），第四轮修复后用户确认左框不变形不贴底、边框完整、两栏都能滚到末尾、字数 18,000；同轮 AI 被供应商限流（429×2、Timeout×1），界面给可读中文原因、进程不崩、源文不丢，日志只记 error+status；⑤ Cmd+W 隐藏后点 Dock 恢复——248ms 进入 revealed=false focused=false、2084ms 两者回来并触发'窗口恢复可见，重发几何'，FR-5.2 成立。全件不含源文/译文/查询串/Key 明文；附件：evidence/0019-V-010.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T12:10:45+00:00 V-010 **passed** VERIFIED_RUNTIME/sandbox
  五个子项全齐（第二次覆盖）：① 启动时序——窗口 09:24:49 建好、钥匙串后台线程 09:25:07 返回，期间持续渲染；② 单实例双开——第二实例退出码 1、提示含持锁者 PID，config/history 的 mtime+size 未变、锁未被误删；③ 空闲 30.2s 消耗 CPU 0.000s（0.0000%），RSS 93.3MB；④ 18,000 字符粘贴——D-4 修复后的构建上机翻 6 段 11.85s 完整出结果（译文 9,181 字符），用户确认左框不变形不贴底、边框完整、两栏都能滚到末尾、字数 18,000；修复前另有 71,211 字符 / 6 段 / 53.4s 的成功记录；同轮 AI 被供应商限流（429×2、Timeout×1），界面给可读中文原因、进程不崩、源文不丢，日志只记 error+status；⑤ Cmd+W 隐藏后点 Dock 恢复——248ms 进入 revealed=false focused=false、2084ms 两者回来并触发'窗口恢复可见，重发几何'，FR-5.2 成立。全件不含源文/译文/查询串/Key 明文；附件：evidence/0020-V-010.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:12:10+00:00 V-012 **passed** VERIFIED_RUNTIME/sandbox
  钥匙串三件套（用户真机操作，2026-10-21）：读取有两次直接日志证据（key_len=35，指纹 d5477b63 → 4b507e46，只记长度与哈希前缀）；写入与清除为用户口述（原话'钥匙串我已经试过了'）+ 指纹变化间接印证——两次启动指纹不同只能来自条目被重写，指纹是 SipHash 摘要前 8 位、单测断言不含 Key 任何片段。无当刻日志行的原因已写明：那次实例用 RUST_LOG=ice_app=debug 启动而 crate 名是 ice_translator，日志被全过滤；代理不得处置用户真实凭据故未代跑。边界核对四项全过：Config 结构无 Key 字段（api_key 仅出现在取值/入参/测试三处）、env::var 取 Key 0 处、设置页只显示'已配置（35 位，存在系统钥匙串）'连指纹都不显示、describe_error 对 BadEncoding/BadDataFormat 不转 Display。未取到的一支：W-1.8 假 Key 的红色原因 + 去设置按钮未真机复现（会覆盖用户真 Key），替代证据为 429 走通同一错误态渲染路径 + 两条无头测试覆盖 GoSettings 动作链；附件：evidence/0021-V-012.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:14:25+00:00 V-009 **passed** BUSINESS_CONFIRMED/business
  真人走查四轮完成（2026-10-20~21，用户本人操作+截图+口述，代理不参与判定）：W-1.1~W-1.7、W-1.9~W-1.24 共 23 项通过（第四轮用户原话'都可以了'）。四轮跑出 5 个缺陷并全部修复+复验：D-1 底部动作行被裁出面板、D-2 Cmd+W 后 Dock 恢复回来一张拉伸贴图（修三轮才过）、D-3 两栏底边不齐、D-4 18k 粘贴把源文框撑出窗口且滚不动、D-5 装饰字形显示为豆腐块；根因与'旧测试为何放走它'见 07-acceptance.md §P4 偏差清单。W-1.8 的假 Key 分支未真机复现（会覆盖用户真 Key，用户选择不走），替代证据为 429 真机走通同一错误态渲染路径 + 两条无头测试覆盖 GoSettings 动作链，缺口只有'红字+去设置按钮'视觉同框，转 P8 用废 Key 补。第四轮新报 5 条经确认全部不在本包范围：简洁窗尺寸/拖动/引擎切换/译文滚动四条属 spec.md:19/25 明划给 P5 的内部版式（转为 W-2.1/W-2.13/W-2.15/W-2.16），自动检测撞目标语言的行为改动由用户决定推到 CHG-004（登记为 TC-1.2-c）；附件：evidence/0022-V-009.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:18:19+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0023-V-001.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:18:20+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0024-V-002.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:18:21+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0025-V-003.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:18:21+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0026-V-004.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:18:22+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0027-V-005.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:18:22+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0028-V-006.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:18:22+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0029-V-007.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:18:22+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["sh", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates 2>/dev/null"] exit=0；附件：evidence/0030-V-008.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:08+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0031-V-001.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:09+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-core"] exit=0；附件：evidence/0032-V-002.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:10+00:00 V-003 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "-p", "ice-app"] exit=0；附件：evidence/0033-V-003.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:10+00:00 V-004 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0034-V-004.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:11+00:00 V-005 **passed** VERIFIED_CODE/local
  argv=["cargo", "fmt", "--all", "--", "--check"] exit=0；附件：evidence/0035-V-005.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:11+00:00 V-006 **passed** VERIFIED_CODE/local
  argv=["python3", "-B", "tools/check_traceability.py"] exit=0；附件：evidence/0036-V-006.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:11+00:00 V-007 **passed** VERIFIED_CODE/local
  argv=["./sdd", "scan"] exit=0；附件：evidence/0037-V-007.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:11+00:00 V-008 **passed** VERIFIED_CODE/local
  argv=["sh", "-c", "! grep -RInE 'sk-[A-Za-z0-9]{20,}' docs features crates 2>/dev/null"] exit=0；附件：evidence/0038-V-008.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:50+00:00 V-009 **passed** BUSINESS_CONFIRMED/business
  真人走查四轮完成（2026-10-20~21，用户本人操作+截图+口述，代理不参与判定）：W-1.1~W-1.7、W-1.9~W-1.24 共 23 项通过（第四轮用户原话'都可以了'）。四轮跑出 5 个缺陷并全部修复+复验：D-1 底部动作行被裁出面板、D-2 Cmd+W 后 Dock 恢复回来一张拉伸贴图（修三轮才过）、D-3 两栏底边不齐、D-4 18k 粘贴把源文框撑出窗口且滚不动、D-5 装饰字形显示为豆腐块；根因与'旧测试为何放走它'见 07-acceptance.md §P4 偏差清单。W-1.8 的假 Key 分支未真机复现（会覆盖用户真 Key，用户选择不走），替代证据为 429 真机走通同一错误态渲染路径 + 两条无头测试覆盖 GoSettings 动作链，缺口只有'红字+去设置按钮'视觉同框，转 P8。第四轮新报 5 条经确认全部不在本包范围：简洁窗尺寸/拖动/引擎切换/译文滚动四条属 spec.md:19/25 明划给 P5 的内部版式（转 W-2.1/W-2.13/W-2.15/W-2.16），自动检测撞目标语言的行为改动由用户决定推到 CHG-004（登记为 TC-1.2-c）；附件：evidence/0039-V-009.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:50+00:00 V-010 **passed** VERIFIED_RUNTIME/sandbox
  五个子项全齐：① 启动时序——窗口 09:24:49 建好、钥匙串后台线程 09:25:07 返回，期间持续渲染；② 单实例双开——第二实例退出码 1、提示含持锁者 PID，config/history 的 mtime+size 未变、锁未被误删；③ 空闲 30.2s 消耗 CPU 0.000s（0.0000%），RSS 93.3MB；④ 18,000 字符粘贴——D-4 修复后的构建上机翻 6 段 11.85s 完整出结果（chars=9181 为译文长度），用户确认左框不变形不贴底、边框完整、两栏都能滚到末尾、字数 18,000；修复前另有 71,211 字符 / 6 段 / 53.4s 的成功记录；同轮 AI 被供应商限流（429×2、Timeout×1），界面给可读中文原因、进程不崩、源文不丢，失败日志只记 error+status；⑤ Cmd+W 隐藏后点 Dock 恢复——248ms 进入 revealed=false focused=false、2084ms 两者回来并触发'窗口恢复可见，重发几何'，FR-5.2 成立。全件不含源文/译文/查询串/Key 明文；附件：evidence/0040-V-010.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:51+00:00 V-011 **passed** VERIFIED_RUNTIME/sandbox
  原样重盖（内容与测量均未变，仍为 2026-10-20 用户在真实界面上的实测）：机翻 MyMemory 3 次 695–1512ms、AI DeepSeek 3 次 708–988ms（其中一次 cache_hit=true elapsed_ms=0，FR-1.8 真机成立）、引擎来回切换均出译文（FR-1.3）、同语言对由 SameLanguage 本地拦下 status=0 未发请求。重盖原因：文档回写使树哈希变化，按门禁'证据过期'规则以同一份证据重新登记；附件：evidence/0041-V-011.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

- 2026-10-21T13:20:51+00:00 V-012 **passed** VERIFIED_RUNTIME/sandbox
  钥匙串三件套（用户真机操作，2026-10-21）：读取有两次直接日志证据（key_len=35，指纹 d5477b63 → 4b507e46，只记长度与哈希前缀）；写入与清除为用户口述（原话'钥匙串我已经试过了'）+ 指纹变化间接印证——两次启动指纹不同只能来自条目被重写，指纹是 SipHash 摘要前 8 位、单测断言不含 Key 任何片段。无当刻日志行的原因已写明：那次实例用 RUST_LOG=ice_app=debug 启动而 crate 名是 ice_translator，日志被全过滤；代理不得处置用户真实凭据故未代跑。边界核对四项全过：Config 结构无 Key 字段（api_key 仅出现在取值/入参/测试三处）、env::var 取 Key 0 处、设置页只显示'已配置（35 位，存在系统钥匙串）'连指纹都不显示、describe_error 对 BadEncoding/BadDataFormat 不转 Display。未取到的一支：W-1.8 假 Key 的红色原因 + 去设置按钮未真机复现，替代证据为 429 走通同一错误态渲染路径 + 两条无头测试覆盖 GoSettings 动作链；附件：evidence/0042-V-012.txt；Spec：84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7
