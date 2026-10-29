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
│      │   ├── fonts.rs          # 黑体-简字体表装配（`definitions()`/`install()`），建窗第一帧之前调用
│      │   ├── system/          # clipboard、hotkeys、screen、secrets、single_instance、alpha（整窗不透明度）、mouse（左键按着没有）、window/titlebar（原生标题栏那颗绿灯）、dblclick（标题栏双击 → 铺满/还原）——图钉不在这里：2026-10-29 变更 K 起它由 `views::pin_toggle` 画在界面里，那个原生模块已删
│      │   └── views/           # mod（工具条+状态条）、full、compact、history、settings、image
│      └── examples/
│          ├── viewport_probe.rs  # 真机几何探针（`cargo run -p ice-app --example viewport_probe`）
│          └── heiti_probe.rs     # 字体缺字探针：逐面打印候选字体缺哪些常用汉字（`cargo run -p ice-app --example heiti_probe`）
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
cargo test  --workspace                 # 全部单元测试（2026-10-28 任务 #25 之后 440 项通过 + 5 项 ignore）
cargo test  -p ice-core                 # 只测领域层（58 项）
cargo test  -p ice-providers            # 只测引擎层（62 项，HTTP 走本地假服务，不联网）
cargo test  -p ice-app                  # 只测界面层（305 项，无头跑帧，不碰网络/剪贴板/钥匙串）
cargo run   -p ice-core --example detect_probe
                                        # 中英混排检测探针：14 条实例逐条判方向，判错即非零退出
cargo test  --workspace -- --ignored    # 跑需要真实网络/真机的集成测试
cargo test  -p ice-providers --test live -- --ignored --nocapture machine
                                        # 只验机翻真实端点（MyMemory，无需 Key）
cargo test  -p ice-providers --test live -- --ignored --nocapture machine_latency_p50
                                        # NFR-1 判定：冷 20 次 + 缓存命中 10 次，约 40s，见下方限流提醒
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

**跑 live 之前先看这条（2026-10-28 量出来的）**：MyMemory 免费端点对同一 IP 的急促调用会**拉黑一段时间**，不是"每秒几次"的短窗口限速。当晚实测顺序是 20/20 → 16/20 → 0/20（全 HTTP 429），此后即使每次间隔 1.5s 也仍然整批被拒，恢复要等更久。所以：一次会话里别把 `-- --ignored` 全表反复跑；`machine_latency_p50_over_twenty_samples` 自身已带 `PACING_MS = 1500` 的采样间隔，成功样本不足 15 条时它会**判"本次测不出"而红**，那不是回归缺陷，等配额回来再跑即可。429 在应用侧一律不自动重试（`error.rs` 的 `should_retry`，由 `rate_limit_is_user_retryable_but_never_auto_retried` 钉住），自动重试只会把封禁拖得更长。

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
  - **"一行装不装得下"不许无头量**（2026-10-28 定，`the_action_row_stays_on_one_line_at_the_minimum_width`）。egui 自带字体里没有中文字形（`epaint/src/text/text_layout.rs` 就写着 "No bundled font has CJK glyphs"），真机的汉字由 `fonts::install` 装进去的黑体-简负责（2026-10-28 变更 H 之前是 eframe 现找的系统字体，本机解析到苹方）：**无头的 `Context::default()` 不跑这一步**，量一个汉字仍是 6.5px，真机 13.0px，在窄窗里照样"一行装得下"，断言锁不住任何东西。写这条测试时不要顺手在测试里调 `fonts::install`——量出来的数会跟着变，判据就不再是可复算的度量了。判据改成按度量算——字号、`button_padding`、`item_spacing` 全取当帧 `Style` 真值，汉字 `1em`、其余 `0.6em`（往宽里偏），每颗按钮**还要抬到 `interact_size.x` 这个下限**（`Button::min_size` 与样式下限取大，"AI" 那颗实际 40px 而不是算出来的 31.6px），再加上面板左右各 8px 的 `Margin::symmetric(8,2)`（egui 写死，不在 `Spacing` 里）与把手两头各 `GAP_PADDING`。按钮清单从**这一帧真的画出来的** `Shape::Text` 里取（`!galley.text().is_empty()`——空内容的按钮也会留下一个空 galley，不过滤会把图标位当成第 0 颗按钮），文案一改这条跟着变。**判据只能有一个方向：`needed ≤ MIN_COMPACT_W`，外加"那一行实测仍是一行"**（2026-10-28 变更 I 把反向那条撤了，原因写在下面）。曾经写过、现在**不许再写**的那条是 `needed > 曾经会换行的那个宽度`（当时是 320，用来证明"这套度量解释了用户看到的那次换行"）：① 置顶那颗退出动作行后度量只剩 **314px < 320**，再断言 `> 320` 就是在写假话；② 更根本的是**这套度量是往宽里偏的估计**（汉字算满 1em、拉丁 0.6em，每颗还要抬到 `interact_size.x`），它只能证明"合法下限放得下"，**不能反过来说"比它窄一像素就该换行"**——真布局在 313px 那一档照样装得下一行。换行机制本身由 `a_row_that_cannot_fit_wraps_instead_of_overflowing` 锁住，**那一档随度量一起挪**：变更 I 之后是 200px，变更 J 把需求宽度降到 262px 后 200px 已经装得下一行，判据留在原处就成永真的空话，于是移到 **150px**——不要拿度量断言去替它。**下限不是只往上走的，也不许自动往下收**：变更 G 砍掉「全量」后同一套度量给出 364px，下限随之 420 → 400；变更 I 砍掉「置顶」给出 314px，下限**留在 400**——400 是用户已经走过查的体量，为省 86px 再动一次属于"没被要求就别改"（默认宽 460 两步都没动）；**变更 J 砍掉「退出」并把行首那颗主题框换成右下角那颗定值滑块，给出 262px，下限第三次仍留 400**（同一条理由，且那 138px 的余量正是留给真机中文字形比度量估计更宽的那一档）；**2026-10-29 变更 K 把图钉从原生标题栏挪进这一行右下角，需求宽度回到 296px（一枚 `PIN_TOGGLE_W` 26 + 一道 `item_spacing` 8），下限第四次仍留 400**——**图钉与滑块同一条理由，见下一句**。**滑块不进字宽算式**：它不像那颗切换框要跟两个汉字取大，`THEME_SWITCH_W` 是定值 40px，字号缩放不动它——所以这一项贡献的是实测宽度，不是字形估计。
  - **滚动容器必须断言"可视区不越出给它的矩形"**（P5 缺陷 D-9 定的硬规则）。只断言"滚轮 N 帧后偏移增长"测不出越界：`ScrollArea` 的 `min_scrolled_size` 默认 64px，矮栏里它把可视区抬到 64px 并画到框外，而最大偏移按这个虚高视口计算，**最后十几像素怎么滚都滚不进来**——偏移在涨、内容也在动，测试全绿，用户看不见最后一行。写法：`full::record_scroll(ctx, 栏名, &output)` 记下（可视矩形、内容尺寸、当前偏移）三个数，断言 `inner.max.y <= 栏矩形.max.y`、`inner.max.y <= 压在它下面的那条带.min.y`、且 `offset >= content - inner.height`（真能滚到底）。`min_scrolled_height(0.0)` 是修法，这四条断言是防它被悄悄改回去的锁。
  - **凡是不参与布局的覆盖式浮层，优先考虑改成"自占一条带"**（P5 v0.7 / REQ-013 定的第二条硬规则）。全量窗右下角的动作条原本是浮层，正文末尾要按 `bar_cover()` 预留一条与它等高的空白带；预留公式与绘制公式分处两地，一边改了另一边没改，D-9 就以同样的症状复发一次。改成 `Panel::bottom` 自占一行之后，两栏的可用高在构造时就被扣掉，"盖住"在结构上不再可能，断言也从"预留量 ≤ 浮层高 +1px"换成一条更简单的"可视区底边在底栏上沿之上"（`the_last_line_stops_above_the_bottom_bar`）。同一条原则也用在简洁窗：动作行先建成 `Panel::bottom`，两带再分 `CentralPanel` 的剩余高度，分摊由纯函数 `views::compact::band_heights()` 决定，所以 1:1 与"矮窗口回落到输入带一行"两套结果都能脱离字体度量单独断言。
  - **"自占一条带"必须建在窗口层，不能建在带内边距的 `CentralPanel` 里面**（P5 v0.7 落地时的缺陷 D-11，2026-10-28 用户"按钮上下留白距离不同"）。`egui::CentralPanel::default()` 用的框架是 `Frame::central_panel`，四边各 8px 内边距（egui `containers/frame.rs:191`）；把 `Panel::bottom` 建在它内部，这条栏就同时吃到"自己的留白 + 那层的 8px"，下边被顶离窗口底边 8px、左右也各少 8px，而**上边只少一份留白**——留白常量写得再对称也没用。正确做法：像工具条、状态条一样由 `views::show` 建（顺序 toolbar → status → 这条栏 → `CentralPanel`，`CentralPanel` 会吃掉剩余矩形，排到它后面的 `Panel` 收不到位置，P4 缺陷 D-1 就是这么把整条动作行裁出窗口的）。写无头骨架测试时也要照这个顺序排——只调页面函数（`full::show`）的骨架里根本没有那 8px，缺陷测不出来。**这条对四个页签一视同仁**：`settings::tests::Harness` 2026-10-28 之前也是直接调 `settings::show`，白送约 120px（工具条 + 状态条 + `CentralPanel` 内边距），换成 `crate::views::show` 后 640×420 下「退出程序」当场测出下沿压出可视区 1px。判据因此分两档，别写成一条：默认尺寸（`UiConfig::default()`）断言**不滚动就看见**（`panel.contains_rect`，可视矩形由 `settings::show` 开头那行 `record(..., "panel", ui.max_rect())` 记下），最小尺寸只断言**仍在窗口内、允许滚到**——把折叠区内够得着的控件也当缺陷锁死，就会诱使去做用户没要的改版（比如把「窗口」组挪到首组）。**并且这条栏必须自己画底色**：`Frame::NONE` 挪到窗口层就是在窗口上开个洞，eframe 默认 `App::clear_color` 是 `rgba(12,12,12,180)`（半透明深灰，`eframe/src/epi.rs:251`），没被任何 `Frame` 画到的区域直接透出桌面（2026-10-28 用户第二次报的"这颜色怎么回事"）。工具条与状态条看不出来，是因为它们用的 `Frame::side_top_panel` 自带 `panel_fill`；`full::bottom_bar` 因此显式 `.fill(ui.visuals().panel_fill)`，并把底色与矩形一起记进 `bottom_bar_id`，由 `the_text_page_bottom_bar_hugs_the_status_bar` 断言两者相等。
  - **正文框里不画状态，一行都不画**（P5 第三条硬规则，缺陷 D-15，2026-10-28 用户"状态怎么又放译文框里了？里面非要有东西你才不难受吗？"）。耗时/缓存/分段/检测这类元信息一律进底栏那一行（纯函数 `full::bar_meta` 给片段 + 颜色，视图只负责拼一条可截断的文字）。这条与 D-13 是**同一类错的两次复发**：为了消掉一处观感毛病（框外空白、界限不明显）往正文区里加东西或给整栏套边框，改完没人问用户要的是不是这个。规则：**先判断"多余的东西该不该存在"，再判断"它放哪"**——D-15 的空白本来就不该存在（E 自占一行紧贴框底，页脚纯属多余），把它删掉而不是搬来搬去。回归锁是 `the_last_line_stops_above_the_bottom_bar` 里那条收紧后的预算（只剩两份框架内边距 + 分隔线），页脚以任何形式复发都会立刻失败。
  - **自占高度面板里的两个 egui 坑**（v0.7 踩过）：① 手动 `allocate` 出来的行必须 `ui.advance_cursor_after_rect(rect)`，不然 `Panel` 量到的还是它自己那一小条，整条栏会塌成 26×10；② `Ui::columns` 完全不看光标，按整行宽度均分，和上面那种定高行叠在一起就重合 5px——要左右两半各自对齐就用 `horizontal_top` + `allocate_ui_with_layout`。
  - **窗口行为也要能无头接线**：抓原生标题栏那条拖动路径不产生 `StartDrag`（AppKit 自己搬窗口），吸附的**开始**只能靠"几何跳变后归于静止"来认。测法是喂一串 `outer_rect`（静止若干帧 → 跳变 → 再静止）断言 `DragSettler` 被 arm 并完成吸附，再用同一串几何配合 `commanded` 目标点断言"自己下发的移动不 arm"——这两条把"程序把用户的拖动判错了"这类只有真机才看得见的问题锁在了单测里。
  - **松手判定不许只看几何静止：能问硬件就问硬件，问不到才退回时间**（2026-10-28 缺陷 D-16 定的硬规则）。几何静止 150ms 是"用户松手了"的**代理指标**，不是松手本身：原生拖拽（`StartDrag` 或标题栏）进的是 AppKit 的**嵌套事件循环**，鼠标抬起那一下不投递给本应用的 view（winit 0.30.13 `platform_impl/macos/view.rs:630` 的 `mouse_dragged` 只转发 `mouse_motion`），所以 `ctx.input(...).pointer.any_down()` 在整段拖动期间是**旧值**，egui 这条路根本判不出松手。代理指标失效的那一幕就是用户报的"拖到边上停住但手指没抬，窗口自己藏到边外，鼠标和界面越拉越远"。四条写法约束：① 问的是 `+[NSEvent pressedMouseButtons]` 的位 0——窗口服务器维护的硬件按键掩码，与"事件有没有投递给我们"无关，公开 API 且不需要辅助功能/屏幕录制权限；② **`None` 的含义是"不知道"，不是"没按着"**：非 macOS、不在主线程、无头测试都拿不到，这些路径 SHALL 原样走旧的时间判据，否则等于悄悄废掉吸附（NFR-7 与既有的静止判定测试都会红）；③ `postpone` 只在 `arm` 之后动计时——空闲帧也去刷新的话窗口永远判不出静止；④ 按住那一帧**不调 `feed`**，所以松手那一帧只用来立参照、下一个阈值才给结论，延迟上界约 150ms，这条上界由 `postponing_without_feeding_baselines_on_release` 锁住，别把它当成"吸附变慢了"顺手改掉。测法沿用 `area_for` 那条注入闭包的路子：`settle_drag(ctx, area_for, held_for)`，同一段"几何连续静止"只换 `held_for` 的返回值，**两个方向都要断言**——`Some(true)` 那几帧不改 `docked_edge`、不发 `OuterPosition`、判定保持 armed；改成"已抬起"之后必须照常吸附（`a_held_drag_stays_put_until_the_button_is_released`）。硬件那一半无头测不到，真机判据在 W-2.33。
  - **改原生控件的行为：每帧从控件现状读，不缓存"我接过了"**（2026-10-28 变更 G 定的硬规则，`system::titlebar::sync`）。接管绿灯那颗 `NSButton` 的 target/action 时，"我是不是改过它"这个状态**不能只存在自己的字段里**：eframe / winit 会在运行期重设标题栏按钮（红绿灯位置随 `always_on_top`、全屏态、缩放变化重建），也可能被用户在系统里触发重建。做法是每帧先 `target()` / `action()` 读**控件上当前的值**，和"应当是什么"比：该挂而没挂就挂、该还而没还就还，两边都幂等。三条派生约束：① `NSControl.target` 是 `unsafe_unretained`， objc2 交给你的是裸指针语义，**必须自己把 `Retained` 存在线程局部（`WIRED`）里持有**，否则闭包对象一释放，绿灯点下去就打到了野地址；② 原始的 target/action 要和改道前那一对一起存，`release()` 时**原样交回**——两种形态共用同一个 `NSWindow`，回到全量窗还挂着就是界面在撒谎（绿灯在全量里仍是原生缩放；任务 #25 的"双击标题栏铺满"**不靠它**——那是标题栏区域的双击手势，走 `system::dblclick` 的本地事件监视器自己下发几何，见 §8 那一行）；③ 悬停提示（`setToolTip`）跟着行为一起改，那颗按钮的 hover 原本是唯一教用户 `Cmd+Shift+M` 的地方，删了可见按钮不能把知识一起删。这条路径**无头测不到**（拿不到真 `NSWindow`），能锁的只有"一次点击只交接一次"（`one_click_is_handed_over_exactly_once`）和"帧循环把点击认成同一条 `Effects::toggle_mode`"（`a_title_bar_click_toggles_through_the_same_effect_as_the_shortcut`）；系统到底听不听，只有走查 W-2.31 判得了，日志里那条 `标题栏绿键改道` 只证明我们发过命令。
- **往原生标题栏里加一颗可点的控件：只有 `NSTitlebarAccessoryViewController` 这一条公开路**（2026-10-28 变更 I 定的硬规则，与上一条是**两类不同的接线**。**这条路本项目已于 2026-10-29 变更 K 放弃**：用户给图钉指定的两个位置——全量窗「悬浮窗」那颗的左边、简洁窗右下角滑块的左边——**都在窗内**，而 accessory VC 只能钉在标题栏最右端，满足不了这句；`system::pin` 整个模块删除，图钉改由 `views::pin_toggle` 用 egui 现画。规则本身仍然成立，它是"下一次真要把控件放进标题栏"时唯一的一条公开路，四条约束原样留在下面）。先记住"为什么不能用 egui 画"：`with_fullsize_content_view(true)` + `titlebar_transparent(true)` 只是让内容铺到标题栏底下，`NSTitlebarView` 依然持有那块区域的 `mouseDown` 去搬窗口，画上去的东西点不动——**能画 ≠ 能点**。要既在栏内又收得到点击，公开 API 里只有 `window.addTitlebarAccessoryViewController(vc)` + `vc.setLayoutAttribute(NSLayoutAttribute::Right)`（尺寸由容器 view 的 frame 决定，这个 VC 没有 `viewSize` 可设）。四条落地约束：① **纯 additive，所以失败可观测**：绿灯那颗是"改道"（要负责还原、要证明没撒谎），图钉是"新增"（没有原语义要归还），接不上就表现为 `Outcome::Unavailable` + 一次性 WARN，界面少一颗按钮而能力一条没少（`Cmd+Shift+P` 与设置页那格照旧）；② **上色只在 borderless 按钮上生效**：`setContentTintColor` 对带框按钮无效，必须 `setBordered(false)` + `setImagePosition(ImageOnly)`，图本身要 `setTemplate(true)` 才认 tint；③ **别用废弃 API 而不用说明**：把 `pin`（出厂斜 45°）转正是 `lockFocus`/`unlockFocus` + `NSAffineTransform` 重画进新 `NSImage`，这两个函数 Apple 已废弃，这里以**带理由的 scoped `#[allow(deprecated)]`** 保留（22pt 图标按显示倍率栅格化，位图 rep 那套换不掉），并**必须**在 `representations().is_empty()`（画布是空的）时退回未旋转那张——宁可歪着显示也不能显示空白；④ **换符号/换颜色/换旋转都算"外观"，三条通道各自独立**，任何一条失效（比如系统没这个 SF Symbol）界面仍读得懂当前状态，不要三条都压在一个 `setFilled` 上。共享的纪律只有一条，抄上一条的 ①：**每帧从控件现状决定改什么**，`Retained<PinTarget>` 存在线程局部（`NSControl.target` 是 `unsafe_unretained`）。无头能锁的仍然只有"一次点击只交接一次"与"帧循环把它认成 `Effects::toggle_topmost`"，真机判据在 W-2.34。**上面这四条约束已随 `system::pin` 一起封存**（2026-10-29 变更 K：图钉落到 egui 这一侧的 `views::pin_toggle`，样式与命中不再问 AppKit）。落在界面里反而量得到更多：两处位置谁左谁右、命中矩形互不重叠、按图钉不触发拖拽、一次点击只下发一条 `WindowLevel`——分别是 `the_pin_sits_left_of_the_floating_button_and_sets_the_same_intent`、`the_pin_sits_left_of_the_theme_switch_and_shares_the_topmost_channel`、`the_pin_effect_and_the_hotkey_share_one_path`；W-2.34 现在判的是这两格在真机上的观感与可点性。
- **拦原生手势（不是改控件）时：本地事件监视器是唯一"既能看见又能取消"的公开位置**（2026-10-28 任务 #25 定的硬规则，`system::dblclick`）。双击标题栏这件事前三条路都堵：`NSTitlebarView` 的 `mouseDown` 子类化会掐断窗口拖动；`NSWindow.titlebarView` 在 objc2-app-kit 0.3.2 里**没绑定**；`windowWillUseStandardFrame:` 属于 winit 自己的 delegate。剩下的 `+[NSEvent addLocalMonitorForEventsMatchingMask:handler:]` 返回 `nil` 即取消派发。四条写法约束：① **吞的范围要能用纯函数算出来并 fail-closed**——`title_band()` 用 `standardWindowButton(...).frame()` 折红绿灯的最右缘、右端只留一道 `EDGE_PAD`（图钉原先让开的那 `pin::PIN_ZONE_W` 随 2026-10-29 变更 K 一起删掉，栏里右半再没有会收点击的控件），`in_band()` 还额外要求带不退化（`left < right && bottom < top`），任何一项拿不到就**一个像素都不吞**，让系统去做它自己配的双击动作；误吞一次单击 = 用户拖不走窗口，比"双击没反应"难查一个量级。② **这条 API 不 retain 传进去的 block**，`RcBlock` 要自己在线程局部里持有到 `removeMonitor` 之后；它躲在 `objc2-app-kit` 的 `block2` feature 后面（`NSEvent.rs:1174` 那道 cfg），Cargo.toml 因此加 `block2 = "0.6"` + feature `"block2"`——**加 feature 前先读绑定 crate 的 cfg**，别一上来开 default（会拖进 ~283 个模块）。③ **几何命令下发的是内尺寸，目标是外框**：`mode::fill_size` 用当帧实测的 `outer − inner` 减装饰，两轴各 `max(0.0)`；写死 32pt 会在缩放因子或装饰变化后必然溢出。④ **铺满期间必须停掉"实时几何写回配置"那一路**（`mode::capture` 靠 `state.filled` 整帧早退），否则整屏尺寸进槽，还原无源、重启后窗口也不是用户那个大小——这正是 D-6/D-10 的换入口。无头能锁的：带几何三条、"一次双击只交接一次"、帧循环把它认成 `Effects::toggle_fill`、铺/还原各自"两条命令各一条"与护栏上膛（`a_double_click_fills_the_work_area_and_arms_the_guard` / `a_second_double_click_gives_the_remembered_rect_back`）、以及"铺到槽里已有的尺寸就拒绝"（`filling_to_the_size_already_recorded_is_refused`）。**监视器本身听不听、菜单栏与 Dock 那圈的观感是不是用户要的"铺满整屏"，只有走查 W-2.37 判得了**；日志里那条 `双击标题栏：下发铺满/还原的几何命令` 只证明我们发过命令。
- **覆盖式浮层如果是纯提示，SHALL 排除在命中测试之外**（2026-10-28 变更 I 定的第二条硬规则，`views::toasts`）。提示条从右下角搬到窗口正中，代价不是"挡视线"而是**挡点击**：正中那块压住的恰好是最常被点的按钮。做法是 `Area::interactable(false)`——egui 直接把这一 layer 整块排除在 hit-test 之外，下面的按钮照常可点，同时也就**没有"点提示条关掉它"这回事**（用户要的正是"不应该有关闭的操作和按钮"）。三条派生：① 淡出要有帧，`request_repaint_after(TOAST_FADE_STEP)`（16ms）**只在屏幕上还有提示条时**续，一空就回到 NFR-2 的空闲策略，否则 TC-N-1 那条 30s cputime 采样会红；② 透明度只乘**框**（`Frame::multiply_with_opacity` 一次处理 fill / stroke / shadow）和**文字色**（`gamma_multiply`），别去乘整窗 alpha——那是 `system::alpha` 的活；③ **要用户做决定的那条不套这条规则**：`confirm()` 的失败提示留着两颗按钮，自动消失等于替用户答"否"。谁将来补一颗 `×`，`a_click_on_a_toast_does_not_dismiss_it` 会立刻红。

- **一处观感设置要同时喂给 egui 的三个不同入口**（2026-10-28 变更 H 定的硬规则，`theme::PaneText`）。字号是 `FontId`；行间距在 `Spacing::extra_text_line_spacing`，**只有 TextEdit 那条路会自动吃**（`text_edit/builder.rs:563`），`Label` 完全不认，得换成每个 section 上的**绝对** `format.line_height`（`text_layout.rs:534` 是 `unwrap_or(row_height)`，不取较大值，所以传小了会切字）；字间距两边都要手工挂 `format.extra_letter_spacing`——`Spacing` 里根本没有这一项，TextEdit 只能通过 `.layouter(...)` 这个自定义排版器塞进去。四个调用点（全量源文、全量译文、简洁输入带、简洁译文带）都走同一个 `PaneText`，就是为了不让"一个框松一个框挤"这种半改完的状态出现。回归锁分两层：`theme::tests` 量 `PaneText` 自己（字号是 `font_scale × 分栏倍率`、行高正好多出设置值、字间距让同一句话变宽），`views::full::tests` 量**真的跑完两帧之后栏目内容的高度**——只有后者能证明 view 真的用了它。**跑帧必须补 `theme::apply`**：`spacing` 那一步是 `app.rs` 每帧做的，测试里不补就永远读到 egui 出厂的 0，正文再松也量不出来（第一版就栽在这儿，源文栏 629→629 一动不动）。还有一个同类陷阱：`full` 的源文框被 `min_size` 拉到满栏高，字数太少时量到的是栏目高不是内容高，测行距要用 2400 字以上。
- **默认档不许是"用户抱怨的那个样子"**。变更 H 之前字距/行距是 0，用户抱怨"太挤"；只把滑杆加上而默认仍是 0，等于把同一个毛病留给用户自己去设置里翻。`config::DEFAULT_LETTER_SPACING`（0.5px）与 `DEFAULT_LINE_SPACING`（3px）因此写死成非零，并由 `letter_and_line_spacing_act_on_both_body_columns` 的第一条断言锁住"默认必须比全 0 松"。**但默认值不许顺手扩成配色改动**（用户 2026-10-28 说"发灰"，根因是中文落到了偏细的苹方，改的是字体族，一处颜色都没动）。

- **统计口径必须与被比较的对象同量纲，先问单位再调常数**（2026-10-28 复核待办第 5 条定的硬规则，`ice-core::detect`）。中英混排"按主导一方判定"这条规则本身没错，错的是两边不是同一个东西：汉字按**字**计、拉丁按**字母**计，一个 `CrashLoopBackOff` 就顶 16 个字，中文句子被判成英语（实测 14 条实例判反 2 条，置信度 0.56/0.77）。改成按翻译单元计（汉字一字一单元、一段连续拉丁字母一单元、`k8s`/`P99` 各算一个术语、纯数字不算词）之后 14 条全对，**而改前那 5 条既有测试一条没动、也没重调阈值**——这是"改的是量纲而不是把常数挪到刚够过"的判据。凡是"按数量比较两边谁占主导"的启发式，动手之前先问一句两边的单位是否可比；复核用的实例表留在 `crates/ice-core/examples/detect_probe.rs`，一条命令重跑、判错非零退出。

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
| 监视器可用区 | egui 的 `ctx.available_rect` 不够（不含菜单栏/Dock 的扣除），走 `system/screen.rs`：objc2 直取 `NSScreen.visibleFrame` 再翻坐标。依赖刻意关掉 default features（`objc2-app-kit` 的 default 会打开 ~283 个模块、拖进六个 Apple 绑定 crate），只开真正用到的那几个模块 feature：`NSScreen`/`NSView`/`NSWindow`/`NSButton`/`NSControl`/`NSEvent` + `objc2-core-foundation`（`NSEvent` 自己带上它需要的 `objc2-foundation` 子 feature，不会多出第三个绑定 crate；`objc2-core-foundation` 不是可选项，带 `CGFloat` 的方法整体关在它后面）。**两条口径不要用混**：停靠/吸附/唤出要的是所有屏可见区的**并集**（`desktop()`，桌面的外缘才是"边"，两块屏中间的接缝不是——按单屏算会把窗口藏进接缝，缺陷 D-8）；离屏夹回要的是窗口所在**单屏**（`work_area()`）。 | 非 macOS 编译不到这段：`screen.rs` 有回退路径，取不到可用区时**不做夹回**（宁可不动，也不把窗口甩到别处） |
| 窗口的缩放与拖动 | 两种形态都带原生标题栏（2026-10-27 Q-013 起简洁窗不再无边框），因此边缘缩放是系统默认能力，不用自绘手柄。曾经的实测结论仍然成立并留着备用：`Decorations(false)` + `Resizable(true)` + 按形态取 `MinInnerSize` 时 AppKit 的边缘缩放热区仍在（拖角期间 inner 以 ~16ms 连续变化 20+ 台阶）。代价是抓标题栏那条拖动路径不产生 `StartDrag`，吸附必须由几何跳变观测补（见 §6 那两条与拖动有关的硬规则）；**"松手"这一侧既读不到事件、也不许只看几何静止**，见下一行 | 各窗口管理器行为不一，P8 跨平台复核 |
| 拖动期间的鼠标按键状态 | 原生拖拽走的是 AppKit 的**嵌套事件循环**，鼠标抬起那一下不投递给本应用的 view（winit 0.30.13 `platform_impl/macos/view.rs:630` 的 `mouse_dragged` 只转发 `mouse_motion`），所以 egui 的 `pointer.any_down()` 在整段拖动期间是**旧值**，判不出松手（缺陷 D-16）。只能问窗口服务器：`+[NSEvent pressedMouseButtons]` 的位 0（`system::mouse::left_button_down`）——公开 API、与事件投递无关、不需要辅助功能或屏幕录制权限、只在主线程调 | 非 macOS 没有等价且免权限的入口，`left_button_down()` 恒返回 `None`；调用方 SHALL 把 `None` 当"不知道"、退回纯时间判据（§6 的 D-16 那条），**不许因为问不到就永远不吸附**，非 macOS 的构建与无头测试因此行为不变（NFR-7） |
| 窗口级半透明 | **2026-10-28 起走原生整窗路径**：`system::alpha::set` → `NSWindow.setAlphaValue`，边框、标题栏、红绿灯一起淡（代价是文字一并变淡，所以"透多少"做成设置页滑杆 `ui.unfocused_alpha`，`0.30..=1.00`，默认 0.85，`sanitized()` 双向夹取且 NaN 退回默认值）。拿窗口句柄共用 `system::window::ns_window`（与 eframe 定位红绿灯同一路数，只在主线程）。**接不住时退回 `views::compact::apply_dim`**（只乘窗内底色 alpha，标题栏不跟）——非 macOS / 拿不到句柄 / 第一帧还没有句柄都算接不住，缓存写 `None` 下一帧重试，一次失败不能永久退回。`with_transparent(true)` 仍在创建期开着（H3：透明区不吞点击、底色不透明时观感不变） | 同理，但透明区是否吞点击需各自实测；`setAlphaValue` 这条是 AppKit 专有，其余平台只剩 `apply_dim` |
| 接管原生标题栏按钮 | 公开 API 够得到**行为**，够不到**目标**：`NSWindow::standardWindowButton(ZoomButton)` 拿到那颗绿灯后可以覆盖它的 `target`/`action`（`system::titlebar`），点下去就走我们的闭包；但它的**平铺/全屏目标**不受控——macOS 26 在绿灯上还叠了悬停平铺选择器，若实测发现它绕过 target/action，本条判为"公开 API 够不到行为"，回退是把那颗可见按钮做回动作行（**由用户裁定**，见 `06-test-cases.md` W-2.31 的停止条件、`07-acceptance.md` P5 遗留①）。另三条见 §6 那条硬规则：每帧读控件现状、`unsafe_unretained` 的 target 要自己持有、交回时原样还 | 无对应概念，P8 跨平台复核；届时切回全量的可见入口只能落在窗内自绘 |
| 往原生标题栏里**加**一颗控件 | 与上一行是两类接线：上一行是改道已有按钮（要还原），这一行是纯新增。公开 API 里唯一"既在栏内又收得到点击"的槽位是 `NSWindow::addTitlebarAccessoryViewController` + `NSTitlebarAccessoryViewController::setLayoutAttribute(NSLayoutAttribute::Right)`；**egui 画不进标题栏**——`fullSizeContentView` 只让内容铺到栏底，`NSTitlebarView` 仍持有那块 `mouseDown` 去搬窗口，画上去的东西点不动。图标用 SF Symbol（`NSImage::imageWithSystemSymbolName`，`pin`/`pin.fill`），要 `setTemplate(true)` + **borderless** 按钮才吃 `setContentTintColor(systemBlue)`；"摆正"是把出厂斜 45° 的 `pin` 用 `NSAffineTransform` 重画进新 `NSImage`，用的 `lockFocus`/`unlockFocus` 是 Apple 已废弃 API（scoped `#[allow(deprecated)]` + 空位图退回未旋转那张，理由写在 `system::pin` 里）。**这一颗是两种形态共用的**：`apply_pin` 挂在 `run` 的形态分支之外，因为两种形态是同一个 `NSWindow`（用户"全量界面也要有"）。feature 清单为此扩到 `NSTitlebarAccessoryViewController`/`NSViewController`/`NSResponder`（父链，`setView:` 才存在）/`NSImage`/`NSGraphics`（重画）/`NSColor`/`NSCell`（`ImageOnly`）/`NSAffineTransform`/`objc2-core-foundation`，`objc2-foundation` 侧加 `NSAffineTransform` 与 `NSArray`（`representations()` 的返回类型，用它判画布是否空白）；`NSPoint/NSRect/NSSize` 从 **objc2_foundation** 导出而不是 app-kit。**这一段已于 2026-10-29 变更 K 作废**：用户把图钉要的位置指到了窗内两处（全量窗「悬浮窗」左边、简洁窗滑块左边），accessory 只能钉标题栏最右端满足不了；`system::pin` 模块连同上面那批 feature 与 `apply_pin` / `poll_pin_click` 两个调用点一起删除，`NSTitlebarAccessoryViewController` 那套机制在本项目里不再留存。图钉现在是 `views::pin_toggle` 的矢量线稿 | 上面那半是"原生才做得到"的代价，这一半是"不再需要原生"的好处：图钉画在 egui 里，任何平台都画得出也点得动，置顶不再有"非 macOS 少一个入口"这回事（NFR-7）——`Cmd+Shift+P` 与设置页那一格仍是同一条 `Effects::toggle_topmost` 通道 |
| 拦下**标题栏双击**（任务 #25） | 公开 API 里既能看见又能取消派发的那一个位置：`+[NSEvent addLocalMonitorForEventsMatchingMask:handler:]`（`system::dblclick`）。返回 `nil` 即吞掉这一次事件，系统"双击标题栏"的默认动作（通常是最小化）因此不执行。**三条纪律**：① 只吞 `clickCount() >= 2` 且落进 `title_band()` 那条带的——单击、内容区、红绿灯一律放行，带边界由 `standardWindowButton(...).frame()` 折出实测宽度、右端只留一道 `EDGE_PAD`（**沿革**：变更 K 之前右端还要让开图钉那 `pin::PIN_ZONE_W`，那颗于 2026-10-29 挪进窗内，让位区随之删除；`title_band()` 的入参也从五个收成四个）（`NSWindow.titlebarView` 在 objc2-app-kit 0.3.2 里**没有绑定**，拿不到栏矩形，所以只能这样拼）；② **判据 fail-closed**：拿不到窗口、算不出带、带退化（`left >= right` 或 `bottom >= top`）时一个像素都不吞，宁可由系统去做它默认动作——误吞一次单击就等于把窗口拖不走（那条路比"双击没反应"难查得多）；③ 这条 API **不 retain** 传进去的 block，`RcBlock` 必须由我们自己在线程局部里持有，出了作用域就是野指针；它躲在 `objc2-app-kit` 的 **`block2` feature**（`NSEvent.rs:1174` 那道 cfg）后面，Cargo.toml 因此加 `block2 = "0.6"` 并在 feature 清单里补 `"block2"`。**已知代价**：双击之后立刻按住拖动失效（第二次按下的那一下被吞，AppKit 的拖拽循环没开始）。**只在简洁窗之外安装**：简洁窗的位置归吸附/停靠，整屏化之后那 3px 热区就没了。**与绿灯那条是两件事**：绿灯改的是**单击**那颗按钮的 target/action，本条拦的是**标题栏区域的双击**，两者互不影响，别把它们当成一处接线 | 非 macOS 分支恒返回 `Outcome::Unavailable`，什么都不改，系统默认双击行为原样保留；铺满/还原只剩 `Cmd+Shift+M` 与手动缩放（NFR-7 的 macOS 优先纪律） |
| 钥匙串 | Security.framework（`keyring` crate） | Windows 凭据管理器 / Linux Secret Service |
| 钥匙串授权框 | 条目不是本程序写入的（例如用 `security add-generic-password` 手工加的）且本程序未签名时，`get_password()` 会**阻塞**到用户点完授权框。所以读取必须离开建窗路径、放 `spawn_blocking` | 无此行为，但读取仍按同一异步路径走，省得分平台两套时序 |
| 窗口隐藏 | eframe 0.37 在 macOS 上**不投递** `Reopen`，`Visible(false)` 之后点 Dock 图标叫不回来；改用 `CancelClose` + `Minimized(true)` | 各自的"最小化到托盘"语义不同，P5 复核 |
| 时区 | `libc::localtime_r` 拿本地偏移 | 同一份代码可用；`time_t`/`tm_gmtoff` 在 32 位平台是 `i32`，转换要留着（对本平台的 `useless_conversion` 告警放行） |
| 字体 | **界面正文不再走系统解析**（2026-10-28 变更 H）：`fonts::install` 在建窗闭包里把 `/System/Library/Fonts/STHeiti Medium.ttc` 读进内存、`ctx.set_fonts` 装成 `Proportional` 族第 0 位，中文才落到黑体-简上。三条不显然的约束：① 这个 `.ttc` 有两面，**第 1 面才是黑体-简**（第 0 面是黑体-繁，两面都覆盖常用简体字，选错不崩不缺字，只有肉眼看得出）；② `Monospace` 族里黑体排在 Hack **之后**，否则对照页的拉丁列不再等宽；③ `set_fonts` 是整体替换，只在启动调一次（每帧调会把字形图集作废），eframe 的 `egui_system_font_provider` 仍挂在 provider 上兜底生僻字（它排在我们的清单之后）。代价：55MB 字体常驻内存（`FontData` 没有文件映射入口）。文件读不到就什么都不做、中文回落系统解析并记一条 debug——不能因为缺字体让界面空白 | Windows / Linux 无这一份文件：`install` 返回 `false`，沿用 eframe 的系统字体回退；真要跨平台一致需自带字体文件，v1 不覆盖 |
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
| 中文还是"发灰"、没变成黑体（变更 H） | `fonts::install` 没生效：文件不在那个路径、或面索引被改回 0（黑体-繁）、或 `install` 调用被挪到了第一帧排版之后 | 看日志有没有 `没找到黑体字体文件`；`cargo test -p ice-app fonts::` 两条断言（族顺序 + 汉字覆盖）会指出是哪一种；`cargo run -p ice-app --example heiti_probe` 直接打印每个候选面缺了哪些字 |
| 拖动/缩放后位置丢了（尤其被强杀） | `on_exit` 只在正常退出跑，`kill -TERM` 不触发它 | 已修：拖动定案、形态切换定案、关窗/退出三处各强制写一次（`app.rs::flush_config(true)`），几何变化本身还会续一个 500ms 的重绘闹钟。若又丢，查这三处调用点 |
| 切到简洁窗后仍是全量大小（缺陷 D-6） | `InnerSize` 是**异步请求**（winit `request_inner_size`），切换后头几帧 `mode::capture` 量到的还是旧几何，会把过渡尺寸写进目标形态的配置槽，而尺寸命令只发一次、没人重发 | 已修：`PendingGeometry` + `MODE_GEOMETRY_GRACE`（1s）——宽限期内 capture 只写位置轴，实测尺寸命中目标 ±2px 才恢复写尺寸。若再出现，查是否有代码绕过 `pending_geometry` 直接回写 `compact_rect` |
| 盘上 `full_rect` 与 `compact_rect` 四项全等，简洁窗缩不回去（缺陷 D-10） | D-6 的修法只做了"到期即放行"：原生那次没跟上的切换，到期时把大窗尺寸照抄进 `compact_rect`，而 `measured == target` 此后每帧命中，坏状态**自我维持**——程序自己再也不会改它 | 已修成**三段护栏**：第一段到期由 `mode::resend_commands` 只补发一组 `InnerSize`+`OuterPosition`（`take_retry` 保证恰好一次）；第二段到期什么都不采纳，把当时实测尺寸存为 `baseline` 并 WARN（只报两个尺寸）；此后尺寸轴只在与 `baseline` 的欧氏距离 >2px（用户真的手动缩放过）时才写回。**已经写坏的用户配置怎么救**：设置页「窗口」分区的「恢复默认窗口尺寸」按钮（`app::reset_window_sizes`，不走 `mode::apply`——它开头先 `capture`，会把正要清掉的坏值再抄一遍） |
| 停靠之后"窗口找不到了"（缺陷 D-7） | 藏出去只剩 `dock_sliver_px`（默认 3px）一条边，层级若是普通窗口就会被其他应用盖住，指针也收不到 | 已修：停靠态一律浮在其他应用之上（egui 侧是 `WindowLevel::AlwaysOnTop`，AppKit 落为 `.floating`）。召回动作是把鼠标推到屏幕那条边上，窗口整条唤出；要彻底解除停靠，唤出后向屏幕内拖 ≥40px 松手 |
| 拔掉外接屏后窗口在屏幕外 | 记录的位置在新 available 区之外 | 已修：启动后第一帧做一次离屏校验（`rescue_offscreen`），整块在区外就 `fit_into_area` 夹回并落盘，日志留一条 warn（只有位置，没有文本）。**停靠态不夹**——左/右停靠本来就有 `w − 3px` 在区外，那 3px 还在区内就不算越界 |
| 机翻报"源语言与目标语言相同" | 源文语种与目标一致（中文源文 + 目标简体中文最常见） | 这不是故障：机翻端点对同语言对必拒，本地提前拦下省一次往返。换目标语言即可（CHG-004 起：源语言为"自动检测"时改为自动翻转目标语言，见 `06-test-cases.md` TC-1.2-c） |
| 想退出程序但找不到入口（用户原话"我没有办法重启软件"） | 关窗（红灯、`Cmd+W`）按 FR-5.2 的定义是**收起不退出**，进程一直在；`Cmd+Q` 能用但界面里没有它 | v0.7（REQ-011）补的可见入口现在**只剩一颗**：设置页「窗口」分区那颗 `退出程序`（简洁窗动作行那颗 `退出` 已于 2026-10-28 变更 J 由用户删除——"悬浮时右下角也没必要有退出按钮，我既然用悬浮窗了，大概率是长期使用了"，那一格让给了右下角的主题滑块）。它只立 `Effects::quit` 意图，与 `Cmd+Q` 汇到同一条 `Action::Quit` 路径（先落盘再释放单实例锁，不绕过任何收尾）。**悬浮形态下要点设置页那颗得先切回全量**（标题栏绿灯或 `Cmd+Shift+M`）；收起态与悬浮态的快捷路仍是 Dock 右键退出或 `Cmd+Q` |
| 右下角那颗滑块看不出哪边是深色哪边是浅色（变更 J） | 它按用户要求**没有任何文字与悬停提示**（"不需要提示，看到的人都懂"），含义全靠"滑块盖住哪一侧、露出哪一侧"；左端=盖住太阳=深色，右端=盖住月牙=浅色，中间=自动 | 先核 `theme_slot`/`theme_at` 那一对纯函数（0.0/0.5/1.0 与 0.25/0.75 分界）——映射错会整条都反；再看 `KNOB_RADIUS`(8.0) 是否还大于太阳那圈 3.6 与月牙外弧 7.0，滑块盖不住图案就等于含义丢失。真机上若用户仍读不懂，回退方案是两端各加一个文字标签，**由用户裁定，不自行加**（`06-test-cases.md` W-2.35） |
| 悬浮窗点标题栏绿灯没反应，或点下去仍是最大化/平铺（变更 G） | 绿键的 `target`/`action` 由 eframe/winit 在运行期重设过，而我们只在切换那一刻挂过一次；或是 macOS 26 的悬停平铺选择器直接绕过了 target-action | 先看 `/tmp/ice.log` 里有没有 `标题栏绿键改道：点击切到全量窗`（`apply_zoom_hook` 只在结果变化时记一行）——**这条只证明我们发过命令，不证明系统听了**。没这行 = 挂不上，查 `system::titlebar::sync` 的每帧读现状分支；有这行但点了没反应 = 公开 API 够不到，按 `06-test-cases.md` W-2.31 的停止条件回退成动作行那颗可见「全量」按钮，**由用户裁定** |
| 图钉**没出现、点不动，或两处样子不一致**（变更 K 之后它是窗内控件，不再问原生） | 两种形态各由 `views::pin_toggle` 在那条 `right_to_left` 带里 `allocate_space` 一格；看不见通常是**那一格被压窄或整条带没建起来**（工具条/动作行换行），样子不对则是 `draw_pin` 里 `pinned` 分支与 `PIN_TILT_DEG` 的符号问题 | 先跑 `cargo test -p ice-app pin`——`the_pin_sits_left_of_the_floating_button_and_sets_the_same_intent` 与 `the_pin_sits_left_of_the_theme_switch_and_shares_the_topmost_channel` 直接量两处矩形的左右关系与宽度（宽度不足当场红），`the_pin_effect_and_the_hotkey_share_one_path` 量"一次点击只下发一条 `WindowLevel`"。两处不一致在结构上不可能：同一个函数、同一个 `config.ui.always_on_top`。**不再有"非 macOS 就看不到图钉"这一档**（原生那颗才有） |
| 提示条来不及读就没了（变更 I 的既定代价） | 居中淡出把总时长从"停留数秒 + 可点掉"压成一条 `TOAST_LIFETIME` 直线（2026-10-28 变更 I 定 500ms），用户原话要求的就是这个 | 这不是缺陷，是需求（"所有的提示类弹窗都是中心弹窗500ms内淡化消失，不应该有关闭的操作和按钮。"）。需要用户决策的走 `confirm()`，它有两颗按钮且不会自动消失；纯提示读不过来就先改**文案**（缩短）。**"不要在提示条上加回关闭按钮"这一条到今天没松**；松的是"不许给它续命"——2026-10-29 变更 L 用户直接判"500ms太短了"，改的就是那**一个常量**（现 1s），`opacity()` / `expired()` / 重绘帧数全部跟着它推导，没有第二处数值需要同步改，这正是当初把时长与淡出合成一个数的回报 |
| 拖到边上**手指还按着**，窗口就自己藏到边外、鼠标与界面越拉越远（缺陷 D-16） | 松手判定只看"几何连续静止 150ms"，而原生拖拽期间应用收不到鼠标抬起（AppKit 的嵌套事件循环，winit 只转发 `mouse_motion`），"停住但没松手"于是被当成松手 | 已修：`app::settle_drag` 每帧先问 `+[NSEvent pressedMouseButtons]`（`system::mouse::left_button_down`），还按着就 `DragSettler::postpone` 把计时重新起算、这一帧一个结论都不下。**反向症状**（松手了也一直不收）说明这台 macOS 上那条询问没如实反映硬件状态，或调用点跑到了非主线程（`MainThreadMarker::new()?` 挡下时返回 `None`，按设计应当**退回时间判据**而不是永不吸附）；真机判据与回退方案写在 `06-test-cases.md` W-2.33 |
| 第二次启动直接退出 | 单实例锁被占（这是预期行为） | 若确认没有另一个实例在跑，是上次被 `kill -9` 留下的陈旧锁；重启会自动接管（PID 已死即删），也可手动 `rm ~/Library/Application\ Support/IceTranslator/ice-translator.lock` |
| 失败日志只有 `error="Status"` | 变体名之外还需要状态码才能定位 | 已补 `status=` 字段（`jobs::status_code`）；状态码不是秘密，响应体已截断、查询串已洗掉 |
| **应用跑着却一行日志都没有** | `RUST_LOG` 的 target 写错：二进制 crate 名是 **`ice_translator`**，不是目录名 `ice-app`。`RUST_LOG=ice_app=debug` 会把全部日志过滤掉，且进程照常运行、看不出异常 | 用 `RUST_LOG=ice_translator=debug,info ./target/debug/ice-translator > /tmp/ice.log 2>&1`。**真机取证前先看一眼日志有没有行**——P4 第四轮就因为这一条丢了钥匙串写入/清除的日志证据，只能改用用户口述 + 指纹变化间接印证 |
| **`pbcopy` 灌中文样本，用户粘出来是空的** | 无 locale 的 shell 里 `pbcopy` 遇到非 ASCII 会**静默失败**（exit 0、剪贴板为空），ASCII 内容不受影响 | `LC_ALL=en_US.UTF-8 pbcopy < /tmp/sample.txt`，再用 `LC_ALL=en_US.UTF-8 pbpaste \| wc -m` 回验非零，才交给用户粘贴 |
| 大文本走 AI 通道报"被限流（HTTP 429）" | 18,000 字符会切成 6 段连发，撞供应商配额；不是缺陷。机翻侧同样会撞：2026-10-28 不限速连发 20 次就把本机 IP 打进一段时间的封禁（见 §4 的 live 提醒） | 换机翻引擎跑同一份样本（实测 6 段 11.85s 出结果），或减小样本、隔几分钟再试。日志只记 `error="RateLimited" status=429`，不含源文 |
