# 开发文档 — 环境、结构与规范

## 1. 环境要求

| 项 | 要求 | 校验命令 |
|---|---|---|
| 系统 | macOS 12+（目标 14+），Apple Silicon 或 Intel | `sw_vers` |
| C 工具链 | Xcode Command Line Tools（提供 ld、sdk） | `xcode-select -p` |
| Rust | stable ≥ 1.85（edition 2024） | `rustc --version` |
| 包管理 | cargo，随 rustup 安装 | `cargo --version` |

安装：

```bash
# 1) 工具链
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
. "$HOME/.cargo/env"

# 2) 命令行工具（若 xcode-select -p 报错）
xcode-select --install
```

> 首次 `cargo check` 会编译 eframe/wgpu 依赖树，Apple Silicon 上约 3~6 分钟，属正常现象，后续增量编译在秒级。

## 2. 目录结构

```
Ice-translator/
├── Cargo.toml                 # workspace：依赖版本集中在 [workspace.dependencies]
├── crates/
│   ├── ice-core/              # 领域层：语言、图片文档模型、配置、历史、术语表
│   ├── ice-providers/         # 引擎层：Translator 调度、缓存、机翻/AI 通道、分段并行窗口（parallel.rs，crate 内私有）
│   ├── ice-vision/            # 视觉层：OCR、原文擦除、译文回写
│   └── ice-app/               # 界面层：eframe 应用、双模式窗口、状态机（二进制名 ice-translator）
│      ├── src/
│      │   ├── main.rs          # 启动时序 + `prepare()`（可在测试里跑的那一段）
│      │   ├── app.rs           # `IceApp`：eframe::App 实现、副作用执行、钥匙串探测回收
│      │   ├── state.rs         # `AppState`：唯一一份界面状态
│      │   ├── mode.rs theme.rs jobs.rs stub.rs
│      │   ├── system/          # clipboard、hotkeys、screen、secrets、single_instance、alpha（整窗不透明度）
│      │   └── views/           # mod（工具条+状态条）、full、compact、history、settings、image
│      └── examples/
│          └── viewport_probe.rs  # 真机几何探针（`cargo run -p ice-app --example viewport_probe`）
├── docs/                      # 本文档所在目录（顶层 document/ 是它的软链）
│   ├── adr/                   # 不可逆决策记录
│   ├── evidence/              # 各阶段验收证据：截图、样图、命令输出
│   └── workflow.md            # SDD 状态机与机器记录格式
├── sdd                        # SDD 命令行入口（./sdd --help）
├── tools/                     # SDD 引擎与门禁脚本（Python 3.11+，仅标准库）
├── features/ quick/ incidents/  # 变更包：spec/design/tasks/traceability/evidence
├── project/                   # 工作区配置、项目边界、路线图、生成的 STATE
├── codebase/                  # 代码地图与仓库登记（本机绝对路径在 .sdd/，不入库）
├── templates/                 # 变更包模板
├── skills/                    # 五个 SDD 技能入口
├── tests/                     # SDD 引擎回归测试
└── assets/                    # 图标、测试样图
```

发布打包时 `sdd`、`tools/`、`features/`、`quick/`、`incidents/`、`project/`、`codebase/`、`templates/`、
`skills/`、`tests/` 属于开发工作区，不进入 .app 产物（P8 的打包脚本按此排除）。

`examples/viewport_probe.rs` 不是产品代码，是**实测工具**：它自己开一个窗口、把每帧的实时几何与 viewport 命令打进日志，
用来验证"无边框能不能靠原生边缘缩放""逐帧 `InnerSize` 补间会不会闪""窗口级半透明有没有黑底"这类只有真机才能定论的问题。
CHG-004 的三轮探针（结论与日志留档在 `features/CHG-004/evidence/`）就是它跑的——遇到"egui 文档没写、单测又问不出答案"的窗口行为问题，先在这里加一次探针，别猜。
`examples/screen_probe.rs` 是同一定位的第二个探针：把 `system::screen::probe()` 读到的**每一块屏**的原生坐标与翻转后的矩形打进日志，用来核"吸附的边界矩形取的是不是整块桌面的并集、两块屏中间那条接缝有没有被当成边"（缺陷 D-8 就是这么排掉的——这类问题在无头测试里只能断言纯函数的输入输出，屏的真实排布只有本机有）。

## 3. 依赖方向（硬约束，NFR-6）

```
ice-app  ──►  ice-providers  ──►  ice-core
   │              ▲                  ▲
   └──────►  ice-vision ────────────┘
```

- `ice-core` 不依赖任何其它 crate，不出现 `eframe`/`egui`/`reqwest`/`image`；
- `ice-providers` 与 `ice-vision` 只依赖 `ice-core`；
- 只有 `ice-app` 可以依赖 UI。
- 校验：`cargo tree -p ice-core -e normal` 输出中不得出现 egui/reqwest。

这条约束的意义：引擎和视觉层可以脱离窗口单独测试，UI 换库不影响业务逻辑。

## 4. 常用命令

```bash
cargo check --workspace                 # 快速全量类型检查
cargo test  --workspace                 # 全部单元测试（2026-10-28 起 399 项通过 + 4 项 ignore）
cargo test  -p ice-core                 # 只测领域层（53 项）
cargo test  -p ice-providers            # 只测引擎层（61 项，HTTP 走本地假服务，不联网）
cargo test  -p ice-app                  # 只测界面层（284 项，无头跑帧，不碰网络/剪贴板/钥匙串）
cargo test  --workspace -- --ignored    # 跑需要真实网络/真机的集成测试
cargo test  -p ice-providers --test live -- --ignored --nocapture machine
                                        # 只验机翻真实端点（MyMemory，无需 Key）
cargo test  -p ice-providers --test live -- --ignored --nocapture ai
                                        # 只验 AI 真实通道，需先在钥匙串存 Key（见 §6 末）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check              # 格式检查
cargo run  -p ice-app                   # 开发运行
cargo run  -p ice-app --example viewport_probe
                                        # 真机几何探针（见 §2 末说明）
cargo build --release -p ice-app        # 发布构建
RUST_LOG=debug cargo run -p ice-app     # 带调试日志运行
```

SDD 工作区（规格、批准、追踪、证据）：

```bash
./sdd status CHG-001                    # 恢复现场：状态、批准哈希、任务与证据
./sdd new CHG-002 --title "…" --repos ice-translator
./sdd validate --offline                # 只校验存档结构与附件完整性（CI/换机用）
./sdd scan                              # 敏感信息基线扫描，只报位置与规则
./sdd index                             # 刷新 project/STATE.md 导航
python3 -B tools/check_traceability.py  # 需求基线 ↔ 追踪矩阵 编号一致性
python3 -B tools/check_staged.py        # 暂存区规格/文档敏感信息门禁
python3 -B tests/test_workflow.py       # SDD 引擎自身回归（36 项）
```

可选：`git config core.hooksPath .githooks` 让 `.githooks/pre-commit` 自动跑上面两道门禁。
该配置改的是本机仓库设置，需你确认后执行；未配置时门禁靠手工跑。

日志：`tracing_subscriber` 按 `RUST_LOG` 环境变量决定级别，默认 `info`。日志只记录文本长度与哈希前 8 位，不记录源文全文（NFR-3）；API Key 永远不落日志。

## 5. 编码规范

1. **注释只写约束和原因**，不复述代码。引用需求用 `FR-x.y` / `NFR-x` 标记，便于追踪矩阵反查。
2. **错误处理**：每个 crate 有自己的错误枚举（`CoreError`/`ProviderError`/`VisionError`），用 `thiserror`，向上传播；只有 `ice-app` 的边界处允许 `anyhow`。禁止 `unwrap()` 出现在生产路径——`Mutex` 中毒场景统一用 `unwrap_or_else(|e| e.into_inner())`。
3. **异步边界**：网络用 tokio；OCR 与图像合成是阻塞调用，必须走 `tokio::task::spawn_blocking`，不得占用 UI 线程。
4. **不在 UI 视图里写业务判断**：翻译调度、语种检测、术语注入在 core/providers，OCR 与合成在 vision；`views/*` 只做展示并把副作用排进 `Effects`。窗口几何这一类是例外，也是同一思路的延伸——**模式切换、吸附/停靠/唤出的几何计算必须是 `ice-app::mode` 里的纯函数**（输入矩形与阈值，输出目标矩形与停靠状态），不读全局、不碰窗口，这样单测不用真拖动窗口就能覆盖；`app.rs` 只负责把纯函数的结论下发成 viewport 命令。
5. **公共 API 最小化**：能用 `pub(crate)` 的不用 `pub`。
6. 数值上限集中定义（`HISTORY_CAPACITY`、`max_prompt_terms`、`ai_chunk_chars`），不散落魔法数字。

## 6. 测试规范

- 单元测试与被测代码同文件，放 `#[cfg(test)] mod tests`，函数名用 `描述性行为`（如 `second_identical_request_hits_cache_and_skips_engine`）。
- 涉及文件读写的用 `tempfile::tempdir()`，禁止写真实的用户配置目录。
- 需要真实外部服务的测试单独放 `crates/<crate>/tests/live.rs`，标 `#[ignore]`，命名 `<通道>_live_<场景>`（如 `machine_live_short`、`ai_live_deepseek`），通过 `-- --ignored <通道>` 触发。原规范的 `net_` 前缀没采用：同一通道往往要跑两三个场景，把通道名放在前面才能用一条过滤命令选中整组。
- 真实端点的契约测试**必须能被本地假服务替代**：引擎构造接受端点注入（`with_endpoint` / `from_config`），默认测试全部走 `fake_server`，联网用例只是补充，不是唯一证据。
- **分段并行的测试只能用 `serve_parallel`，不能用 `serve`**（2026-10-28 加）。`serve` 按到达顺序发固定响应，而并行之后**到达顺序 ≠ 段序**，照旧写法会把内容发错段，断言变成猜谜（`machine` 那条老测试就是这样侥幸绿过一次）。`serve_parallel` 的两件事都是必需的：① 应答由**这条连接自己收到的请求**决定（`reply(到达序号, 原始请求)`）；② 记录**同时在飞的连接数峰值** `Fake::peak`，"真的并行"是量出来的而不是掐总耗时猜的。两个坑：栅栏最多白等 `BARRIER_HOLD`=250ms，所以引擎的测试超时必须明显大于它（本项目两条通道都改成 3s），否则"栅栏没放行"会被误报成"网络超时"；串行回归的表现是 `peak == 1`，不是报错。
- AI 联网用例从 macOS 钥匙串读 Key（ service `IceTranslator` / account `ai-api-key`），生产代码不依赖 `keyring`（只作为 `ice-providers` 的 dev-dependency，见 `ADR-005`）：
  ```bash
  security add-generic-password -a ai-api-key -s IceTranslator -w   # -w 隐藏输入，不落终端记录
  ```
  测试失败信息会直接提示这条命令；Key 不进仓库、不进日志、不进对话。
- UI 无法单测的部分走"人工走查表"，见 `06-test-cases.md`，走查结论与截图存进 `docs/evidence/`。
- **界面层也能单测**，P4 起固定这套写法（`cargo test -p ice-app`，全部不碰网络/剪贴板/钥匙串）：
  - 用 `egui::Context::default()` + `ctx.run_ui(...)` 无头跑一帧，断言渲染后的状态而不是像素；
  - 视图函数一律收 `&mut Effects` 而不是直接动手：副作用（发请求、写剪贴板、存 Key、切页签）先在 `Effects` 里排队，测试直接断言这个队列，`app.rs` 才在帧末真执行；
  - 引擎用 `stub::StubEngine`（可指定回什么译文或抛哪个 `ProviderError` 变体），`StubError::all()` 覆盖全部 11 个变体，加变体时必须同步加进去，`every_error_variant_renders_a_message_and_never_panics` 会自动兜住；
  - 钥匙串相关测试注入读取函数（`spawn_probe_with(runtime, ctx, read)`）或给一个已关闭的通道模拟"探测未回"，**绝不**碰真条目——真条目会让测试挂在系统授权框上；
  - 涉及 tokio 的用 `new_current_thread` runtime 即可，被测代码不该依赖多线程调度。
- **两类"问不出答案"的问题，P5 定了固定解法**：
  - **带时间/外部世界的行为**（剪贴板轮询、停靠的悬停唤出与 500ms 自动收回）不能靠 egui 内省断言：`ctx.run_ui(...).drop_without_applying_deltas()` 不提交这一帧的 repaint 记账，`has_requested_repaint()` 在无头 `Context` 上恒真。做法是把外部世界**注入成闭包**再数调用次数——`poll_clipboard(ctx, read)` 收剪贴板读取函数、`watch_dock(ctx, area_for)` 收可用区探针，测试给计数闭包、真机给 `clipboard::read` 与 `screen::desktop`（吸附/停靠/唤出用的**桌面并集**；`screen::work_area` 只给离屏夹回用，两者按缺陷 D-8 分工）。顺带把隐私要求也一起锁住：`clipboard_watch` 关闭或窗口收起时断言 `reads == 0`。
  - **"画到哪"的问题**（控件溢出、把手压住按钮）靠 `views::compact::record(ctx, salt, rect)` 把控件矩形记进 `Context` 临时数据，然后在**最小窗口尺寸**（`MIN_COMPACT_SIZE`）下断言各带不重叠、不越界、行尾把手不被挤成零宽。P4 的 D-1/D-4 都是"断言 `Effects` 全绿而界面已经烂了"，所以版式测试必须量几何，不能只看副作用队列。
  - **"一行装不装得下"不许无头量**（2026-10-28 定，`the_action_row_stays_on_one_line_at_the_minimum_width`）。egui 自带字体里没有中文字形（`epaint/src/text/text_layout.rs` 就写着 "No bundled font has CJK glyphs"），真机的汉字是 eframe 现找的系统字体（macOS 上 PingFang）：无头量一个汉字 6.5px，真机 13.0px，在窄窗里照样"一行装得下"，断言锁不住任何东西。判据改成按度量算——字号、`button_padding`、`item_spacing` 全取当帧 `Style` 真值，汉字 `1em`、其余 `0.6em`（往宽里偏），每颗按钮**还要抬到 `interact_size.x` 这个下限**（`Button::min_size` 与样式下限取大，"AI" 那颗实际 40px 而不是算出来的 31.6px），再加上面板左右各 8px 的 `Margin::symmetric(8,2)`（egui 写死，不在 `Spacing` 里）与把手两头各 `GAP_PADDING`。按钮清单从**这一帧真的画出来的** `Shape::Text` 里取（`!galley.text().is_empty()`——空内容的按钮也会留下一个空 galley，不过滤会把图标位当成第 0 颗按钮），文案一改这条跟着变。两个方向都要断言：`> 旧下限`（否则"下限要抬"这个结论没被锁住）且 `≤ 新下限`（否则真机又会换行）。
  - **滚动容器必须断言"可视区不越出给它的矩形"**（P5 缺陷 D-9 定的硬规则）。只断言"滚轮 N 帧后偏移增长"测不出越界：`ScrollArea` 的 `min_scrolled_size` 默认 64px，矮栏里它把可视区抬到 64px 并画到框外，而最大偏移按这个虚高视口计算，**最后十几像素怎么滚都滚不进来**——偏移在涨、内容也在动，测试全绿，用户看不见最后一行。写法：`full::record_scroll(ctx, 栏名, &output)` 记下（可视矩形、内容尺寸、当前偏移）三个数，断言 `inner.max.y <= 栏矩形.max.y`、`inner.max.y <= 压在它下面的那条带.min.y`、且 `offset >= content - inner.height`（真能滚到底）。`min_scrolled_height(0.0)` 是修法，这四条断言是防它被悄悄改回去的锁。
  - **凡是不参与布局的覆盖式浮层，优先考虑改成"自占一条带"**（P5 v0.7 / REQ-013 定的第二条硬规则）。全量窗右下角的动作条原本是浮层，正文末尾要按 `bar_cover()` 预留一条与它等高的空白带；预留公式与绘制公式分处两地，一边改了另一边没改，D-9 就以同样的症状复发一次。改成 `Panel::bottom` 自占一行之后，两栏的可用高在构造时就被扣掉，"盖住"在结构上不再可能，断言也从"预留量 ≤ 浮层高 +1px"换成一条更简单的"可视区底边在底栏上沿之上"（`the_last_line_stops_above_the_bottom_bar`）。同一条原则也用在简洁窗：动作行先建成 `Panel::bottom`，两带再分 `CentralPanel` 的剩余高度，分摊由纯函数 `views::compact::band_heights()` 决定，所以 1:1 与"矮窗口回落到输入带一行"两套结果都能脱离字体度量单独断言。
  - **"自占一条带"必须建在窗口层，不能建在带内边距的 `CentralPanel` 里面**（P5 v0.7 落地时的缺陷 D-11，2026-10-28 用户"按钮上下留白距离不同"）。`egui::CentralPanel::default()` 用的框架是 `Frame::central_panel`，四边各 8px 内边距（egui `containers/frame.rs:191`）；把 `Panel::bottom` 建在它内部，这条栏就同时吃到"自己的留白 + 那层的 8px"，下边被顶离窗口底边 8px、左右也各少 8px，而**上边只少一份留白**——留白常量写得再对称也没用。正确做法：像工具条、状态条一样由 `views::show` 建（顺序 toolbar → status → 这条栏 → `CentralPanel`，`CentralPanel` 会吃掉剩余矩形，排到它后面的 `Panel` 收不到位置，P4 缺陷 D-1 就是这么把整条动作行裁出窗口的）。写无头骨架测试时也要照这个顺序排——只调页面函数（`full::show`）的骨架里根本没有那 8px，缺陷测不出来。**这条对四个页签一视同仁**：`settings::tests::Harness` 2026-10-28 之前也是直接调 `settings::show`，白送约 120px（工具条 + 状态条 + `CentralPanel` 内边距），换成 `crate::views::show` 后 640×420 下「退出程序」当场测出下沿压出可视区 1px。判据因此分两档，别写成一条：默认尺寸（`UiConfig::default()`）断言**不滚动就看见**（`panel.contains_rect`，可视矩形由 `settings::show` 开头那行 `record(..., "panel", ui.max_rect())` 记下），最小尺寸只断言**仍在窗口内、允许滚到**——把折叠区内够得着的控件也当缺陷锁死，就会诱使去做用户没要的改版（比如把「窗口」组挪到首组）。**并且这条栏必须自己画底色**：`Frame::NONE` 挪到窗口层就是在窗口上开个洞，eframe 默认 `App::clear_color` 是 `rgba(12,12,12,180)`（半透明深灰，`eframe/src/epi.rs:251`），没被任何 `Frame` 画到的区域直接透出桌面（2026-10-28 用户第二次报的"这颜色怎么回事"）。工具条与状态条看不出来，是因为它们用的 `Frame::side_top_panel` 自带 `panel_fill`；`full::bottom_bar` 因此显式 `.fill(ui.visuals().panel_fill)`，并把底色与矩形一起记进 `bottom_bar_id`，由 `the_text_page_bottom_bar_hugs_the_status_bar` 断言两者相等。
  - **正文框里不画状态，一行都不画**（P5 第三条硬规则，缺陷 D-15，2026-10-28 用户"状态怎么又放译文框里了？里面非要有东西你才不难受吗？"）。耗时/缓存/分段/检测这类元信息一律进底栏那一行（纯函数 `full::bar_meta` 给片段 + 颜色，视图只负责拼一条可截断的文字）。这条与 D-13 是**同一类错的两次复发**：为了消掉一处观感毛病（框外空白、界限不明显）往正文区里加东西或给整栏套边框，改完没人问用户要的是不是这个。规则：**先判断"多余的东西该不该存在"，再判断"它放哪"**——D-15 的空白本来就不该存在（E 自占一行紧贴框底，页脚纯属多余），把它删掉而不是搬来搬去。回归锁是 `the_last_line_stops_above_the_bottom_bar` 里那条收紧后的预算（只剩两份框架内边距 + 分隔线），页脚以任何形式复发都会立刻失败。
  - **自占高度面板里的两个 egui 坑**（v0.7 踩过）：① 手动 `allocate` 出来的行必须 `ui.advance_cursor_after_rect(rect)`，不然 `Panel` 量到的还是它自己那一小条，整条栏会塌成 26×10；② `Ui::columns` 完全不看光标，按整行宽度均分，和上面那种定高行叠在一起就重合 5px——要左右两半各自对齐就用 `horizontal_top` + `allocate_ui_with_layout`。
  - **窗口行为也要能无头接线**：抓原生标题栏那条拖动路径不产生 `StartDrag`（AppKit 自己搬窗口），吸附只能靠"几何跳变后归于静止"来认。测法是喂一串 `outer_rect`（静止若干帧 → 跳变 → 再静止）断言 `DragSettler` 被 arm 并完成吸附，再用同一串几何配合 `commanded` 目标点断言"自己下发的移动不 arm"——这两条把"程序把用户的拖动判错了"这类只有真机才看得见的问题锁在了单测里。

## 7. 提交规范

- Conventional Commits：`docs(P0): ...`、`feat(P3): ...`、`fix(P5): ...`、`test(P7): ...`
- 每个阶段验收通过后至少一个提交；提交信息正文说明**为什么**这么改，而不是改了哪些文件。
- 阶段验收未通过不进入下一阶段，问题以 `fix(Px)` 回到对应阶段修。
- 由变更包驱动的开发，提交正文引用 `CHANGE-ID@version` 与 Spec SHA-256（规格与代码同仓，可同一次提交）。
- **本仓库是公开的**：从外部引入文件前必须过 `./sdd scan`、`tools/check_staged.py`，
  并按关键字核查不含其他项目的标识、内部域名、真实账号与未脱敏日志；任一不过即不提交。

## 8. 平台差异备忘

| 主题 | macOS | Windows / Linux |
|---|---|---|
| 应用图标（Dock / 任务栏） | macOS **没有"窗口图标"**：winit 的 `set_window_icon` 是空实现（`platform_impl/macos/window_delegate.rs:1541`）。能改 Dock 磁贴的只有 `NSApplication.setApplicationIconImage`，而 eframe 0.37 已经帮你调了——前提是 `NativeOptions::viewport.icon` 有值；**不设就退回 eframe 自带的 egui logo**（`native/epi_integration.rs:506`），这就是"图标定稿了但 Dock 还是不对"的根因。见 `main::app_icon`：`include_bytes!` 编进二进制 + 启动时降采样到 256（Dock 磁贴最大约 128pt，视网膜 2x 到顶；不解这一下会在启动帧多占约 8MB RGBA）。**Dock 上那行名字改不了**（eframe 注释明确），要连名字、Finder 图标、`.icns` 一起解决就得做真正的 `.app` bundle。 | Windows 走同一项：eframe 用 `WM_SETICON` 设大小两枚 HICON；Linux 靠窗口图标 |
| OCR | Vision 框架，本地零下载 | v1 不提供，界面明确提示 |
| 置顶窗口 | `NSWindow.setLevel` / egui `always_on_top` | 各自 API，egui 已封装 |
| 监视器可用区 | egui 的 `ctx.available_rect` 不够（不含菜单栏/Dock 的扣除），走 `system/screen.rs`：objc2 直取 `NSScreen.visibleFrame` 再翻坐标。依赖刻意关掉 default features（`objc2-app-kit` 的 default 会打开 ~283 个模块、拖进六个 Apple 绑定 crate），只开 `NSScreen`/`NSGeometry`。**两条口径不要用混**：停靠/吸附/唤出要的是所有屏可见区的**并集**（`desktop()`，桌面的外缘才是"边"，两块屏中间的接缝不是——按单屏算会把窗口藏进接缝，缺陷 D-8）；离屏夹回要的是窗口所在**单屏**（`work_area()`）。 | 非 macOS 编译不到这段：`screen.rs` 有回退路径，取不到可用区时**不做夹回**（宁可不动，也不把窗口甩到别处） |
| 窗口的缩放与拖动 | 两种形态都带原生标题栏（2026-10-27 Q-013 起简洁窗不再无边框），因此边缘缩放是系统默认能力，不用自绘手柄。曾经的实测结论仍然成立并留着备用：`Decorations(false)` + `Resizable(true)` + 按形态取 `MinInnerSize` 时 AppKit 的边缘缩放热区仍在（拖角期间 inner 以 ~16ms 连续变化 20+ 台阶）。代价是抓标题栏那条拖动路径不产生 `StartDrag`，吸附必须由几何跳变观测补（见 §6 最后一条） | 各窗口管理器行为不一，P8 跨平台复核 |
| 窗口级半透明 | `with_transparent(true)` + 失焦时给窗体底色乘 alpha（`views::compact::apply_dim`）；surface 不透明时再怎么乘都透不出来 | 同理，但透明区是否吞点击需各自实测 |
| 钥匙串 | Security.framework（`keyring` crate） | Windows 凭据管理器 / Linux Secret Service |
| 钥匙串授权框 | 条目不是本程序写入的（例如用 `security add-generic-password` 手工加的）且本程序未签名时，`get_password()` 会**阻塞**到用户点完授权框。所以读取必须离开建窗路径、放 `spawn_blocking` | 无此行为，但读取仍按同一异步路径走，省得分平台两套时序 |
| 窗口隐藏 | eframe 0.37 在 macOS 上**不投递** `Reopen`，`Visible(false)` 之后点 Dock 图标叫不回来；改用 `CancelClose` + `Minimized(true)` | 各自的"最小化到托盘"语义不同，P5 复核 |
| 时区 | `libc::localtime_r` 拿本地偏移 | 同一份代码可用；`time_t`/`tm_gmtoff` 在 32 位平台是 `i32`，转换要留着（对本平台的 `useless_conversion` 告警放行） |
| 字体 | 系统 PingFang.ttc 等，Core Text 枚举 | 需自带字体文件，v1 不覆盖 |
| 应用形态 | 需打包 `.app` 才有 Dock 图标与正常生命周期 | 直接 exe |

## 9. 常见故障

| 症状 | 原因 | 处理 |
|---|---|---|
| `linker 'cc' not found` | 未装 CLT | `xcode-select --install` |
| 启动后窗口秒退且无日志 | panic 在 init 前 | `RUST_LOG=trace cargo run -p ice-app` 看第一行 |
| `crates.io 403` | 网络代理拦截 | 检查 `~/.cargo/config.toml` 的 source 替换 |
| 白屏/黑屏 | GPU 后端与远程桌面/虚拟显卡组合 | 切换 `ice-app` 里 eframe 的后端 feature（glow ↔ wgpu） |
| 启动后没有窗口，日志停在 `取得单实例锁` | 钥匙串授权框把 `get_password()` 阻塞住了，而读取排在建窗之前 | 已修：读取挪到 `app::spawn_key_probe`（建窗之后、`spawn_blocking` 里）。若复现，先确认 `main.rs` 的时序没被改回去 |
| 配置改了不生效 | 落盘是脏标记 + 退避 500ms，不是逐帧写 | 等半秒再看 `config.toml`；Cmd+Q 会强制落盘。真不落盘时查 `config_dirty` 有没有被置上 |
| 拖动/缩放后位置丢了（尤其被强杀） | `on_exit` 只在正常退出跑，`kill -TERM` 不触发它 | 已修：拖动定案、形态切换定案、关窗/退出三处各强制写一次（`app.rs::flush_config(true)`），几何变化本身还会续一个 500ms 的重绘闹钟。若又丢，查这三处调用点 |
| 切到简洁窗后仍是全量大小（缺陷 D-6） | `InnerSize` 是**异步请求**（winit `request_inner_size`），切换后头几帧 `mode::capture` 量到的还是旧几何，会把过渡尺寸写进目标形态的配置槽，而尺寸命令只发一次、没人重发 | 已修：`PendingGeometry` + `MODE_GEOMETRY_GRACE`（1s）——宽限期内 capture 只写位置轴，实测尺寸命中目标 ±2px 才恢复写尺寸。若再出现，查是否有代码绕过 `pending_geometry` 直接回写 `compact_rect` |
| 盘上 `full_rect` 与 `compact_rect` 四项全等，简洁窗缩不回去（缺陷 D-10） | D-6 的修法只做了"到期即放行"：原生那次没跟上的切换，到期时把大窗尺寸照抄进 `compact_rect`，而 `measured == target` 此后每帧命中，坏状态**自我维持**——程序自己再也不会改它 | 已修成**三段护栏**：第一段到期由 `mode::resend_commands` 只补发一组 `InnerSize`+`OuterPosition`（`take_retry` 保证恰好一次）；第二段到期什么都不采纳，把当时实测尺寸存为 `baseline` 并 WARN（只报两个尺寸）；此后尺寸轴只在与 `baseline` 的欧氏距离 >2px（用户真的手动缩放过）时才写回。**已经写坏的用户配置怎么救**：设置页「窗口」分区的「恢复默认窗口尺寸」按钮（`app::reset_window_sizes`，不走 `mode::apply`——它开头先 `capture`，会把正要清掉的坏值再抄一遍） |
| 停靠之后"窗口找不到了"（缺陷 D-7） | 藏出去只剩 `dock_sliver_px`（默认 3px）一条边，层级若是普通窗口就会被其他应用盖住，指针也收不到 | 已修：停靠态一律浮在其他应用之上（egui 侧是 `WindowLevel::AlwaysOnTop`，AppKit 落为 `.floating`）。召回动作是把鼠标推到屏幕那条边上，窗口整条唤出；要彻底解除停靠，唤出后向屏幕内拖 ≥40px 松手 |
| 拔掉外接屏后窗口在屏幕外 | 记录的位置在新 available 区之外 | 已修：启动后第一帧做一次离屏校验（`rescue_offscreen`），整块在区外就 `fit_into_area` 夹回并落盘，日志留一条 warn（只有位置，没有文本）。**停靠态不夹**——左/右停靠本来就有 `w − 3px` 在区外，那 3px 还在区内就不算越界 |
| 机翻报"源语言与目标语言相同" | 源文语种与目标一致（中文源文 + 目标简体中文最常见） | 这不是故障：机翻端点对同语言对必拒，本地提前拦下省一次往返。换目标语言即可（CHG-004 起：源语言为"自动检测"时改为自动翻转目标语言，见 `06-test-cases.md` TC-1.2-c） |
| 想退出程序但找不到入口（用户原话"我没有办法重启软件"） | 关窗（红灯、`Cmd+W`）按 FR-5.2 的定义是**收起不退出**，进程一直在；`Cmd+Q` 能用但没有任何看得见的按钮 | v0.7（REQ-011）补了两颗「退出」按钮——简洁窗动作行末尾、设置页「窗口」分区——都只立 `Effects::quit` 意图，与 `Cmd+Q` 汇到同一条 `Action::Quit` 路径（先落盘再释放单实例锁，不绕过任何收尾）。收起态下界面点不到，那条路径仍然是 Dock 右键退出或 `Cmd+Q` |
| 第二次启动直接退出 | 单实例锁被占（这是预期行为） | 若确认没有另一个实例在跑，是上次被 `kill -9` 留下的陈旧锁；重启会自动接管（PID 已死即删），也可手动 `rm ~/Library/Application\ Support/IceTranslator/ice-translator.lock` |
| 失败日志只有 `error="Status"` | 变体名之外还需要状态码才能定位 | 已补 `status=` 字段（`jobs::status_code`）；状态码不是秘密，响应体已截断、查询串已洗掉 |
| **应用跑着却一行日志都没有** | `RUST_LOG` 的 target 写错：二进制 crate 名是 **`ice_translator`**，不是目录名 `ice-app`。`RUST_LOG=ice_app=debug` 会把全部日志过滤掉，且进程照常运行、看不出异常 | 用 `RUST_LOG=ice_translator=debug,info ./target/debug/ice-translator > /tmp/ice.log 2>&1`。**真机取证前先看一眼日志有没有行**——P4 第四轮就因为这一条丢了钥匙串写入/清除的日志证据，只能改用用户口述 + 指纹变化间接印证 |
| **`pbcopy` 灌中文样本，用户粘出来是空的** | 无 locale 的 shell 里 `pbcopy` 遇到非 ASCII 会**静默失败**（exit 0、剪贴板为空），ASCII 内容不受影响 | `LC_ALL=en_US.UTF-8 pbcopy < /tmp/sample.txt`，再用 `LC_ALL=en_US.UTF-8 pbpaste \| wc -m` 回验非零，才交给用户粘贴 |
| 大文本走 AI 通道报"被限流（HTTP 429）" | 18,000 字符会切成 6 段连发，撞供应商配额；不是缺陷 | 换机翻引擎跑同一份样本（实测 6 段 11.85s 出结果），或减小样本、隔几分钟再试。日志只记 `error="RateLimited" status=429`，不含源文 |
