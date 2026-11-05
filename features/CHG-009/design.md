Change: CHG-009
Version: v0.1
Spec-Hash: UNAPPROVED

# CHG-009 · Design

- Change: CHG-009 · v0.1 · **待用户批准**（`./sdd approve CHG-009` 由用户本人跑；批准前本包只动规格、设计与计划，不动业务代码）
- Repo: `ice-translator`（分支 `main`）
- 前置：CHG-006（P7 回写与导出）**十一条任务全部 done、`./sdd validate` 通过、状态仍 IMPLEMENTING**——收口等用户走查 W-4.1~W-4.6 与 `accept`。代码态 `f4c67ea`（2026-11-05）：**619 passed / 0 failed / 5 ignored**（ice-app 400 · ice-core 60 · ice-providers 65 · ice-vision 94）
- 排期由用户 2026-11-05 定："先收口 P7，再做截图"＋"要考虑不同系统……逐步适配 linux 和 win"。**因此本包的批准发生在 P7 收口之后**，批准前不动代码
- 本包不改变任何已批准业务行为；要改变行为走 `./sdd revise` 并重新批准

## 实现方案

### 模块划分

| 文件 | 动作 | 内容 |
|---|---|---|
| `crates/ice-app/src/system/capture.rs` | **新建** | 唯一的"这台机器能不能截、截成什么样"的出处。mac：`spawn_blocking` 里跑 `/usr/sbin/screencapture -i -x -o -t png <唯一路径>`，返回 `Captured(PathBuf)` / `Cancelled`（判据：文件是否出现，见 H1）/ `Failed(类别)`；非 mac：**同签名**恒 `Unsupported`，不写第二套形状。纯函数部分（临时路径生成、取消判定、失败类别→文案）全部能无头测，AppKit/子进程那部分不能 |
| `crates/ice-app/src/state.rs` | 改 | 第四条通道：`Job::ScreenCapture { id, .. }` + `CaptureStage{Idle,Capturing,Done,Failed,Cancelled}` + 第四个自增 `capture_request_id`（与识别/翻译/合成三条互不作废，形状与 CHG-006 那条一致）；`capture_gate()` 给"这颗灰不灰"与"状态行说哪句话"的同一个来源；**不新增配置字段** |
| `crates/ice-app/src/jobs.rs` | 改 | 一条 `Job::ScreenCapture` 分岔：`spawn_blocking` 里发起子进程、结束后 `try_send` 一条 `Outcome::ScreenCaptured` + `request_repaint`。**帧循环里绝不等它**（CHG-005 那条面板纪律的同一条理由） |
| `crates/ice-app/src/views/image.rs` | 改 | E 区第一排加一颗「截图翻译」（禁用而不是消失，非 mac 给一句话）；I 区多一格"截图"段。识别派发复用既有那条 `Effects::job` 口，不另开第二道 |
| `crates/ice-app/src/app.rs` | 改 | 帧末执行 `Effects::capture`（含快捷键那条同源路径）；回信到达后按 Q-001 的裁定决定是只派发识别、还是识别完自动接翻译 |
| `crates/ice-app/src/system/hotkeys.rs` | 改 | 一个不与 `Cmd+Shift+3/4/5` 相撞的组合（候选 `Cmd+Shift+X`），落进既有的 `Action` 枚举而不是新开一条链路 |
| `docs/{02,03,05,06,07}`、`docs/traceability.md` | 改 | 模块表、E 区那一排与 I-01 系列、§6 硬规矩与 §8 三平台那张表（**"截得下来但没有 OCR"这一格与截图那格并排写**）、新 TC 与 W-5.x 走查行、变更登记、FR-2.10 那一行从"延期项"转"本包承担" |
| 依赖 | **零增量** | mac 走系统二进制子进程（不需要任何 crate），兜底形态走既有 arboard 读图。任何"要新增依赖"都属停止条件 4，先回来批准 |

### 数据流（macOS 主路）

```
点「截图翻译」/ 快捷键
  └─ Effects::capture ── 帧末 ── Job::ScreenCapture
        └─ spawn_blocking: screencapture -i -x -o -t png  <tmp>/ice-shot-<n>.png
              ├─ 文件出现 → Outcome::ScreenCaptured(path)  ─┐
              ├─ 文件没出现 → Cancelled（什么都不做）        ├─ 回信通道(有界) + request_repaint
              └─ 子进程起不来/被拒 → Failed(类别)  ──────────┘
                    ↓ 帧循环
        ImageInput::Path(path) → 既有识别任务（admit_path 读完立刻删临时文件，REQ-003）
                    ↓（Q-001 若选 (b)）
        识别 Done → 自动 submit_image_translate → 自动 submit_image_compose?（第三问，见开放点）
```

**临时文件的生命周期只有一句话长**：`admit_path` 成功把像素读进内存之后立即 `let _ = remove_file(path);`——删除失败只记 debug，不影响结果（REQ-003 末句）。文件名带一个自增计数与 pid，两次连点不互相覆盖。

### 三平台分档（写进 `docs/05` §8 的那张表，批准前先如实标 待取证）

| 平台 | 截图这一步 | 识别这一步 | 净效果 | 状态 |
|---|---|---|---|---|
| macOS | `screencapture -i` 交互式框选 → 我们指定的路径 | Vision 本地 OCR（CHG-005 已实现） | **整条链可用**，"框选即译"成立 | 本包实现 |
| Linux（X11 / Wayland） | 候选 `import` / `gnome-screenshot -a` / `grim`+`slurp` / XDG 门户；**哪条可用未取证**（H3） | **无后端**（`native_ocr_supported()` = false，静态核查钉住） | 即便截到也只能停在一颗禁用的入口 + 一句说明 | 待 Q-002 裁定；默认 (a) 本包不做 |
| Windows | 候选 `ms-screenclip:` / SnippingTool，产物很可能只进剪贴板（H4） | 同上，无后端 | 同上 | 同上 |

**这张表的存在理由**：用户那句"逐步适配 linux 和 win"如果只落到"截图"这一格，会得到一个"能截屏、不能翻译"的跨平台，那是更坏的意外。所以天花板（识别后端）与新增能力（截图）必须并排写，批准时看到的就是这一张。

### 开放点（写在这里是为了不偷偷决定）

1. **"一起"到底几步**：截完自动识别是确定的（REQ-001 末句），自动翻译按 Q-001 默认 (b)，**自动出图（再自动合成）本包暂不做**——那是把三段一键串死，失败时用户更难判断哪一段坏了；要的话一句改口。
2. **取消判定**：H1 未实测，实现第一条任务就是拿真机把它证一证，证完再定代码里的判据；写死"退出码 0 = 成功"是不许的。
3. **自家窗遮挡**（REQ-004）：三档方案在 Q-003 里，默认 (a) 临时取消置顶；真机若"闪一下"难看就退 (c) 登记限制。

## REQ-001 实现与验证

**链路**：`views/image::import_bar` 那颗「截图翻译」只置 `Effects::capture = true` → 帧末 `run_effects` 交 `state.submit_capture()` 出 `Job::ScreenCapture{id, …}` → `jobs` 在 `spawn_blocking` 里跑子进程 → `Outcome::ScreenCaptured{id, path|cancelled|failed}` → `apply` 判代之后走既有 `dispatch_image(ImageInput::Path)`。**"复用而不是新造"是这条 REQ 的全部实现风格**：取图这一段是新的，进来之后每一步都是 CHG-005/006 已经跑通的路。

**为什么不许在帧循环里等**（事实 6）：`screencapture -i` 阻塞到用户画完为止，可能是 2 秒也可能是 2 分钟；把它放在 `ui()` 里就是 CHG-005 那条"面板不许 `runModal`"的同一个错误换了个载体。回信用 `try_send` + `request_repaint`，理由与面板回调一字不差（那一下点击属于系统框选层，不属于我们的窗口）。

**幂等与并发**：`capture_in_flight.is_some()` 时 `submit_capture()` 返回 `None`（第二次点击被忽略，不是"顶掉第一次"——顶掉会让用户看到一张他没画过的图）。

**验证**：无头——那颗只给意图不给路径（路径要等回调）、在飞时第二次派发返回 `None`、回信按代丢弃、`dispatch_image` 被调用时切到图片页（复用 CHG-005 那条"切页不是可选项"的断言）。真机——W-5.x 的"框选界面真出来了、画完真进来了"。

## REQ-002 实现与验证

三态在 `CaptureStage` 里就是三个变体，**取消不是失败**：`Cancelled` 那一支不落提示条、不改 `image`/`ocr`/`composed` 任何东西（与面板 `Reply::Cancelled` 同一条纪律，CHG-005 已锁过一次）。

**取消的判据按 H1 定**（T-001 先证）：默认写"指定路径没出现文件 = 取消"，出现文件 = 成功，**退出码不参与判定**——因为系统工具在取消时也可能返回 0。证完之后把结论写进 `capture.rs` 的注释与 `docs/05`，不留"我们猜的是退出码"这种第二判据。

**失败只给能行动的话**：三类（系统工具不可用 / 被权限拒 / 未知）共用一句"这台机器上截不出来；可以先用系统截图（`Cmd+Shift+4`）再点「粘贴」"——退路是既有的、真的能走通的那条（事实 3、4），不是客套话。

**验证**：无头——回信枚举的三态各断一条（取消不清任何东西、失败只提示条、空图仍走 `NoText` 那一档不新造）；真机——W-5.2 读不读得懂，以及 H1 本身。

## REQ-003 实现与验证

**路径生成**是纯函数：`<系统临时目录>/ice-translator/<pid>-<自增>.png`。自增而不是时间戳，因为时间戳在同一帧内两次点击会撞（`Instant` 精度不是保证），撞了就是后写的覆盖前一张——而前一张可能正在被识别线程读。**pid 进去**是为了两个实例（不同锁文件、开发版与正式版并跑）不互相覆盖。

**删除时机**在 `admit_path` 成功之后（Q-005 默认），代码形状是"读到内存 → 立刻 `let _ = remove_file` → 再进识别"。识别失败也要删（放在读到之后而不是成功之后，否则失败的那张会留在盘上——那正是最不该留的那张）。删除失败只 `tracing::debug!` 一句，**不许因为清理失败而让识别结果消失**（用户拿到的东西优先于我们没扫干净的临时目录）。

**日志与文案里不许有文件名**：文件名本身就是用户内容（`报销-11月.png` 那种）。允许的字段是尺寸、耗时、失败类别——与 `secrets::fingerprint` 那套"只记长度与哈希"同一条纪律。

**验证**：无头——路径唯一性（连点两次不等）、识别失败路径下文件仍被删（`tempfile` 造一张真 PNG，跑一遍断言文件消失）、错误与日志文本不含该文件名（把文件名塞进断言的反向检查）；真机——`ls` 那个临时目录证不残留。

## REQ-004 实现与验证

**这一条唯一诚实的写法是先取证再决定**（T-001），所以设计只定形状不定方案：`capture.rs` 暴露一个"发起前 / 结束后"的钩子对（`prepare()` / `restore()`），`app.rs` 在派发前后各调一次，**三档候选都塞在这一对里**（Q-003：临时取消置顶 / 收进托盘 / 什么都不做只在 hover 里说明）。

**不许新增唤回方式**：中间那档（收进托盘）能选，但走的是 `mode::hide`/`mode::reveal` 既有那对函数——那套"叫得回来"的判定在 D-21（泵挂错钩子）与 D-25（叫了班没人读）上各踩过一次，本包再开第三条路就是第三次。

**恢复失败不许半死不活**：`restore()` 只做一件事——把发起前记下的那一组状态（置顶与否、可见与否、几何）原样发回去；发不回去（拿不到句柄）时 SHALL 照常完成截图，只是窗口留在收起态并给一句提示，**SHALL NOT 因为恢复失败就把截好的图丢掉**。

**验证**：只有真机（W-5.1）。无头不假装有判据——这是本包唯一一条完全交给眼睛的 REQ，写在这里就是为了批准时看清楚这一点。

## REQ-005 实现与验证

**一处判定，两个用途**：`capture::capability() -> Capability{Interactive, ClipboardFallback, Unsupported}` 是全局唯一出处，视图的置灰和 `docs/05` §8 那张表都读它（与 `native_ocr_supported()` 同族形状）。视图里 SHALL NOT 出现 `#[cfg(target_os)]`——散开的 cfg 是"某平台编译不过"的温床。

**编译面的保证方式**：非 mac 那一支是**同签名**的门控版本（`ice-vision` 的 `ocr.rs` 就是这么写的，并有静态核查测试钉住），所以 Linux/Windows 上 `cargo check` 起得来是本包的硬要求；但本机只有 `aarch64-apple-darwin` 一个 target（CHG-005 记过），因此**编译级按 TC-P-7 的老规矩记"未验证"**，不许拿"源码扫描看起来对"当"三平台能编译"。

**天花板与新增能力并排写**：`docs/05` §8 那一行的右半格必须写"截图：可/不可 + 识别：无后端"。用户那句"逐步适配 linux 和 win"被翻译成两件事：**取图这一格本包按裁定推进，识别那一格仍是 L-6 的既有形态**——这两格不在同一个包里解决，写清楚比含混乐观便宜得多。

**验证**：静态（源码扫描那条测试 + clippy 的全 target 面）+ 真机（非 mac 那颗禁用入口的说明，W-5.4）。

## REQ-006 实现与验证

**快捷键与按钮落同一条路**：`hotkeys::action_of` 多吐一个 `Action::CaptureArea`，`run()` 里那一支与帧末那颗按钮一样只是置 `Effects::capture`——**不开第二条实现**（图钉那次改动的教训：两条通道迟早不一样）。

**组合键的选法**：避开 `Cmd+Shift+3/4/5`（系统截图）与既有的 `Cmd+Shift+M/P` 家族，候选 `Cmd+Shift+X`；`docs/03` 与 `docs/05` 那张快捷键表同步加一行。焦点在输入框里时照常生效（这不是文本类快捷键，没有"用户在打字"的冲突，与 `Cmd+Shift+C` 那条同档）。

**hover 说三件事**：会框选、esc 取消、截完直接进翻译（Q-001 选 (b) 时第三句才这么写，选 (a) 就写"识别完还要点翻译"）——**提示跟着裁定走，不许写死**。

**验证**：无头——快捷键与按钮置同一个字段、不与系统组合相撞（常量层面一条断言）、提示文案随 `capability()` 变；真机——W-5.3 顺不顺手。

## REQ-007 实现与验证

**隐私口径按本包收窄**（不照抄 P6 那句"识别期间零外发"）：截图像素与临时文件全程本地；`screencapture` 是系统二进制，不经任何网络；识别在本地 Vision；**文本经 P3 通道送出去发生在"翻译"那一步**——Q-001 选 (b) 时那一步由截图自动触发，所以界面上 SHALL 看得出来（I 区从"待翻译"直接跳到"翻译中 N/M"，且那条自动触发在提示条或状态行里点名"截图后自动翻译"）。**SHALL NOT 出现"我没让它联网它联网了"这种观感**。

**零依赖增量**是本包的结构保证：mac 走子进程、兜底走既有 arboard，所以 `cargo tree -p ice-app -e normal` 的输出必须与本包之前**逐行相同**（任何新增名字都是停止条件 4）。

**验证**：`cargo tree`（本机可跑，进 `check`）+ `nettop -p <pid>` 一次采样带阳性对照（真机）+ `./sdd scan` 与密钥形态那条 grep 继续成立。

## REQ-008 实现与验证

**默认值写死且每个都有出处**（延续 CHG-005 Q-006 那条纪律："用户调不动也解释不清的设置比写死的更糟"）：临时目录基名、文件名前缀、`-t png`（PNG 是无损且保住透明，截图不需要 JPEG）、是否自动翻译（Q-001 的裁定）、框选起始模式（不设 `-J`，让系统按用户上一次的选择走）。每一个常量旁边一行注释说"为什么是这个"。

**不新增配置字段**：`config.toml` 格式不变，因此也不涉及 `sanitized()` 那一套边界（无新数值区间要夹取）。若实现中确实需要（比如"临时目录要不要指到别处"），那是停止条件 6：先回来批准，不自己加。

**验证**：无头——常量集合与 `config` 结构无变化（既有的"配置字段不许漂"那条锁继续通过）；`./sdd scan`。

## 影响与回滚

- **涉及仓库/文件**：只动 `ice-translator`，且集中在 `ice-app`（一个新模块 + `state`/`jobs`/`views::image`/`app`/`hotkeys` 各一处）与文档。`ice-vision` **零改动**（本包只在它前面接一段取图），`ice-core`、`ice-providers` 同样零改动。
- **配置与数据**：**不新增配置字段**；不进 `config.toml`、不写历史、不落盘任何用户可见文件（临时文件即用即删）。
- **兼容性**：图片页既有四条入口与 CHG-006 的三段状态机不动，只多第四条通道；`Outcome`/`Effects` 新增字段是加法，既有测试（含 P6/P7 那批意图矩阵）应原样通过——**它们不许被改宽来适配新行为**。
- **回滚**：`git revert` 那批提交即可；`system/capture.rs` 是纯新文件，回滚不留残留。文本翻译链、识别链、回写链不受影响是设计约束而不是运气（本包不调用 `ice-providers` 的任何新东西）。
- **SDD 操作注意**：批准后照固定顺序 `./sdd approve CHG-009` → `plan-stamp` → `refresh` → `seal-plan` → `transition IMPLEMENTING`；`docs/` 在树指纹内，证据 `check` + `record` 要在文档定稿后一次性做完（CHG-006 收口那趟同一条纪律）。

## 验证层次

| 层 | 手段 | 覆盖 | 谁判 |
|---|---|---|---|
| `VERIFIED_CODE`（local/mock） | `cargo test --workspace`（**619 项不许回退**）+ `capture.rs` 的纯函数断言（路径唯一性、取消判定、失败类别→文案、`capture_gate` 与按钮置灰同源）、`state` 的第四条通道过期互不作废、`views/image` 那颗入口的意图矩阵、`hotkeys` 不与系统组合相撞 | REQ-001/002/003/006/008 的数据层与状态机 | Agent（无头） |
| `VERIFIED_CODE`（static） | `./sdd scan`、`check_traceability.py`、`grep` 确认无 `#[cfg]` 散落在视图里、`cargo tree` 零依赖增量 | REQ-005 的"编译面结构"那一半、REQ-007 的静态那半 | Agent，结论写进证据件 |
| `VERIFIED_RUNTIME`（sandbox） | 真机一次完整链：发起框选 → esc 取消（证 H1）→ 再截一块 → 自动进识别 →（Q-001）自动翻译；`ls` 临时目录证不留文件；`nettop -p <pid>` 证像素与文本在点翻译前零外发 | REQ-001/002/003/007 的真机那一半、H1 | Agent 跑（要用户在机器前配合按 esc） |
| `BUSINESS_CONFIRMED`（business） | 新走查 **W-5.1~W-5.4**：截图里有没有我们自己的窗（REQ-004 的唯一判据）、失败文案读不读得懂、快捷键顺不顺手、非 mac 那颗禁用入口的说明是否像"这台机器不行"而不是"没做" | 只有真机能定的那部分 | **只有用户** |
| 不在本包验收范围 | Linux/Windows 真机（Q-002 默认不做；即便做也只到"取证"，识别那格仍是无解） | — | — |
