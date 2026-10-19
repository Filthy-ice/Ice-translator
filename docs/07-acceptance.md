# 阶段验收记录

每阶段一条记录，字段固定：**目标 / 验收方法 / 证据 / 结论 / 遗留**。
结论只有三种：`通过`、`带已知限制通过`、`不通过（回退到本阶段返工）`。

---

## P0 立项与需求 — 2026-10-19

| 项 | 内容 |
|---|---|
| 目标 | 冻结 v1.0 需求基线，明确不做项，定义八个阶段的验收门槛 |
| 验收方法 | 按 `01-requirements.md` 第 6 节自检清单逐条核对：编号唯一性、Must 项可测性、无不可测表述、范围自洽 |
| 证据 | `docs/00-charter.md`、`docs/01-requirements.md`；提交 `docs(P0)` |
| 结论 | **通过** |
| 遗留 | 需求基线锁定；后续变更须改需求文档并在本文件登记原因。机翻端点选择尚未实测（转 P3 的 ADR-004） |

## P1 工程骨架 — 2026-10-19

| 项 | 内容 |
|---|---|
| 目标 | 工具链就绪；四 crate 分层骨架可编译；空窗口能启动并验证置顶与模式切换 API；开发文档可用 |
| 验收方法 | 命令逐条原样执行 + 真机窗口实测 + 几何日志（`docs/evidence/P1-03-window-geometry.txt`）+ 无头断言 ViewportCommand 序列。**截图不入库**：本机 `screencapture -R 区域` 失效，只能全屏采集，画面必然带用户桌面其他内容，出于隐私删除（见事实清单最后两条） |
| 结论 | **通过** |

### P1 验收项

| # | 检查 | 方法 | 结果 |
|---|---|---|---|
| 1 | Rust 工具链版本满足 edition 2024 | `rustc --version` | ✅ 1.99.1（≥1.85） |
| 2 | 四 crate 依赖方向正确 | `grep ^ice- crates/*/Cargo.toml` + `cargo tree -p ice-core -e normal` | ✅ app→{core,providers,vision}，providers→core，vision→core，core 无内部依赖；core 依赖树中无 egui/eframe/reqwest/image |
| 3 | 全量类型检查通过 | `cargo check --workspace` | ✅ exit 0，0 warning |
| 4 | 单元测试通过 | `cargo test --workspace` | ✅ 53 passed / 0 failed（ice-core 40 · ice-providers 10 · ice-app 3 · ice-vision 0，视觉层是 P6/P7 的桩） |
| 5 | Lint 无警告 | `cargo clippy --workspace --all-targets -- -D warnings` | ✅ exit 0（修掉 5 处：2 个可 derive 的 Default、2 个 field_reassign_with_default、1 个 assert_eq 与字面量 bool 比较） |
| 6 | 窗口能启动并渲染 | 真机启动 + 全屏截图目测（截图不入库） | ✅ 标题栏、heading、两个按钮、状态行均按 `03-ui-design.md` 的位置渲染；几何日志 `P1-03` 记录到实际 inner/outer 尺寸与缩放 |
| 7 | 置顶 API 生效 | 简洁窗（置顶开）与用户其他应用同时运行 | ✅ 用户实测反馈"置顶简洁窗口现在无法拖动有点影响我干活"——即该无边框窗覆盖在其正在使用的应用之上且可点击；对照：普通级全量窗被同一应用遮挡。命令层面由 `topmost_maps_to_always_on_top_window_level` 断言 |
| 8 | 模式切换改变窗口形态 | 几何日志 `P1-03` + 无头单测 `ice-app::tests` | ✅ 简洁态 inner==outer==460×96（无标题栏）；全量态 outer 比 inner 高 32pt（标题栏回来）。点击路径本身未做自动化（本机无法合成鼠标事件），但按钮回调调用的就是被测的 `apply_mode`/`apply_topmost`，其命令序列已断言 |
| 9 | 配置读写与损坏回退 | `config` 模块单测 12 项（含 `corrupt_config_is_rejected_not_panic`、`load_at_falls_back_to_defaults_with_reason`）+ 真机写入畸形 config.toml 启动 | ✅ 畸形配置：WARN 记录原因 → 默认配置启动（几何日志显示回到默认 1080×720 全量窗）→ 界面顶部橙色告警条渲染可见 |
| 10 | 开发文档命令可原样执行 | 逐条复制执行 `05-dev-guide.md` §1/§3/§4 | ✅ 全部 exit 0；`-- --ignored` 当前匹配 0 条（联网测试在 P3 落地）；`cargo fmt --all -- --check` 与 `--check` 两种写法等价可用 |
| 11 | 发布构建可用 | `cargo build --release -p ice-app` | ✅ exit 0，耗时 1m05s，产物 `target/release/ice-translator` 11 MB（strip 后） |

### P1 过程中确定/修正的事实

- rustup 首次安装出现"部分安装"状态，重新执行安装脚本自动恢复，最终 stable 1.99.1。
- `reqwest` 0.13 的 TLS feature 名已从 `rustls-tls` 改为 `rustls`；`objc2-vision` 无 `VNImageRequestHandler` feature（实际为 `VNRequestHandler`）。因此 OCR 与 AppKit 的 objc 绑定推迟到真正需要它们的 P5/P6 阶段再接，骨架不背未验证的依赖。
- 本机无法连通 `translate.googleapis.com`（连接失败），`api.mymemory.translated.net` 可用（实测 en→zh、zh→en 正常，ja→zh 命中翻译记忆给出劣质结果）。机翻通道据此选型，详见 ADR-004。
- eframe/egui 0.37 与常见教程差异（已按源码修正，不是猜测）：`App::update` → `App::ui(&mut Ui, &mut Frame)`；`on_exit` 不再带 glow 参数（默认渲染器为 wgpu）；`TopBottomPanel` → `Panel::top/bottom(id)`；`ViewportCommand::WithDecorations` → `Decorations`；`Position` → `OuterPosition`；`WindowLevel(Some(..))` → `WindowLevel(..)`；`AppCreator` 需返回 `Ok(Box<dyn App>)`。
- 启动形态此前被忽略：骨架最初无论配置里存的是什么模式都以全量尺寸开窗，已改为按 `config.ui.mode` 决定初始尺寸/边框/窗级（避免"先开大窗再缩"的闪烁）。
- 本会话的 macOS 权限现状：`screencapture` 需要屏幕录制授权（用户重新授权后可用）；`screencapture -R x,y,w,h` 在本机一律返回 "could not create image from rect"，只能全屏采集后用 `sips --cropOffset` 裁剪；AppleScript 取窗口信息超时，因此**无法合成鼠标点击**，UI 交互类走查在 P4/P5 需要用户配合或改用无头断言。
- **全屏截图不能作为入库证据**：唯一可用的采集方式是整屏，画面必然包含用户 IDE 中的项目代码与其他会话窗口。P1 已拍的两张（旧 `P1-01`/`P1-02`）在推送前删除，视觉结论改由几何日志 + 无头命令断言 + 用户实测反馈支撑。P4/P5 需要图像证据时改用"仅含本应用窗口区域"的裁剪，并在提交前逐张检查。
- **屏幕锁定期间截到的是锁屏壁纸**：`caffeinate -u` 只能唤醒显示、不能解锁会话，此时 `screencapture` 返回不含任何应用窗口的壁纸图（表现为文件正常、内容全黑或全壁纸）。补拍视觉证据必须在用户解锁屏幕后进行。
- **SIGTERM 不触发 `on_exit`**：实测 `kill -TERM` 后 config.toml 未被改写，即异常退出/强杀不会保存窗口几何与模式。正常退出（Cmd+Q、关窗）走 eframe 的 `on_exit` 会保存。→ 记为 P5 待办：**几何回写时机**（拖动/缩放结束与形态切换时落盘），否则简洁窗位置在崩溃后丢失。
- 远端推送凭据此前缺失，2026-10-19 用户已配置完成，P1 起恢复"每阶段验收后 commit + push"。


---

## P2 架构与设计 — 2026-10-19

| 项 | 内容 |
|---|---|
| 目标 | 架构/UI/图像流水线设计成文；每条需求映射到具体模块，无孤儿需求；关键取舍有 ADR |
| 验收方法 | ① 用脚本把 `01-requirements.md` 的需求编号集合与 `traceability.md` 的行集合做双向差集；② 检查矩阵中每条 Must 的"实现"列非空；③ 逐 crate 把 `pub mod` 清单与 `02-architecture.md` §2 的模块表对照；④ 核对 §8 自检 7 项；⑤ ADR 是否覆盖 GUI/OCR/擦除/密钥/分层五项取舍 |
| 证据 | 差集结果 44 ↔ 44 双向为空（无孤儿需求，也无凭空多出的行）；Must 行"实现"列 0 空；`02-architecture.md` §8；`docs/adr/ADR-001/002/003/005/006` |
| 结论 | **通过（签发）** |
| 遗留 | ADR-004（机翻端点选型）依计划待 P3 实测后补；`ice-vision` 四个模块与 `ice-app` 六个模块按 §8 的落地表在 P4~P7 填充 |

### P2 过程中确定/修正的事实

- **模块表漏登记**：`ice-providers::request`（`TranslationRequest`/`TranslationResult`）在骨架里已存在并被 `dispatcher` 使用，但 §2 的表没有这一行。已补登记。这是逐 `pub mod` 对照才暴露的偏差，说明"设计文档 vs 实际代码"必须机器核对，不能凭印象。
- **需求统计此前是错的**：矩阵与需求文档写的 Must 27 / Should 16 / Could 2，逐行重算实为 **Must 30 / Should 12 / Could 2**（共 44 条）。原因是 P0 手工计数即有误，FR-3.9 加入时又只把 26 改成 27 而没有重算。两处统计已按脚本结果更正。
- **矩阵漏一行 Could**：`NFR-8 可访问性`（字号缩放、键盘可达）在需求里有、矩阵里没有，已补，并挂到 P4 走查（`03-ui-design.md` §5 第 5 步的 Tab 顺序检查正是它的验证方法）。
- NFR-6（分层依赖）在矩阵里原记"待 P8"，实际 P1 已用 `cargo tree` 核过，改为"部分 ✅，P8 三目标复核"。
- ADR-001 的成立不再只是纸面判断：单窗口双形态所需的 `Decorations/InnerSize/OuterPosition/Resizable/WindowLevel` 组合在 macOS 真机按预期改变窗口形态（`evidence/P1-03-window-geometry.txt`），这是 P2 能签发的直接依据。

---

## P3 引擎层 — 待记录
## P4 全量界面 — 待记录
## P5 简洁悬浮模式 — 待记录
## P6 OCR — 待记录
## P7 译文回填 — 待记录
## P8 收口 — 待记录

## 变更登记

| 日期 | 变更 | 原因 |
|---|---|---|
| 2026-10-19 | 无 | 基线建立 |
| 2026-10-19 | P1 通过；`ice-app` 启动形态改为读取 `config.ui.mode`（原实现固定按全量尺寸开窗） | 骨架自测发现保存的模式不生效，属 FR-3.1 范围内的修正，不改需求 |
| 2026-10-19 | 新增 P5 待办：窗口几何与形态切换的**即时落盘**（不依赖 `on_exit`） | 实测 SIGTERM/强杀不触发 `on_exit`，异常退出会丢失简洁窗位置 |
| 2026-10-19 | 提交节奏恢复为"每阶段验收后 commit + push" | 用户已配置 GitHub 推送凭据 |
| 2026-10-19 | 证据规范变更：`docs/evidence/` 只允许"裁剪到仅含本应用窗口"的截图，P1 两张整屏截图删除；视觉结论改由几何日志 + 无头断言支撑 | 整屏截图含用户 IDE 代码与其他会话窗口，不应进入远端仓库。规则写入 `03-ui-design.md` §5 |
| 2026-10-19 | P2 签发；需求统计更正为 Must 30 / Should 12 / Could 2（共 44 条），矩阵补登记 `ice-providers::request` 模块与 `NFR-8` 行 | 逐行脚本重算发现 P0 的手工统计有误、矩阵漏一行 Could；模块表漏登记已存在的 `request` 模块。均为文档与代码对齐修正，不改需求内容 |
| 2026-10-19 | 需求基线变更 3 项：FR-3.2 增补"无边框不得导致窗口不可移动"；新增 **FR-3.9 形态切换形变动画**（M）；NFR-4 由"提供暗色主题"改为"**默认跟随系统深浅色 + 三态开关**" | 用户实测 P1 骨架窗口后反馈：置顶简洁窗拖不动影响干活、切换太硬、要求配色跟随系统。三项均已确认 egui/eframe 0.37 有对应能力（`StartDrag`、逐帧 `InnerSize`、`ThemePreference::System`），落在 P4/P5 实现 |
| 2026-10-19 | 引入 SDD 工作流引擎（P2.5，变更包 `CHG-001`）：根目录新增 `sdd`/`tools/`/`templates/`/`skills/`/`features/`/`project/`/`codebase/`/`tests/`，P3 起的阶段变更走 `features/CHG-0NN/` 的批准与证据门禁；`docs/00~07` 基线继续作为需求事实源 | 用户要求把改动纳入结构化记录。来源为其本地工作区的通用模板层，公司项目内容一律不取；本仓库公开，提交前跑 `./sdd scan` 与 `tools/check_staged.py` 两道门禁 |
