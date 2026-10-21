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
│   ├── ice-providers/         # 引擎层：Translator 调度、缓存、机翻/AI 通道
│   ├── ice-vision/            # 视觉层：OCR、原文擦除、译文回写
│   └── ice-app/               # 界面层：eframe 应用、双模式窗口、状态机（二进制名 ice-translator）
│      └── src/
│         ├── main.rs          # 启动时序 + `prepare()`（可在测试里跑的那一段）
│         ├── app.rs           # `IceApp`：eframe::App 实现、副作用执行、钥匙串探测回收
│         ├── state.rs         # `AppState`：唯一一份界面状态
│         ├── mode.rs theme.rs jobs.rs stub.rs
│         ├── system/          # clipboard、hotkeys、secrets、single_instance
│         └── views/           # mod（工具条+状态条）、full、history、settings、image
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
cargo test  --workspace                 # 全部单元测试（P4 后当前 246 项通过 + 3 项 ignore）
cargo test  -p ice-core                 # 只测领域层（49 项）
cargo test  -p ice-providers            # 只测引擎层（53 项，HTTP 走本地假服务，不联网）
cargo test  -p ice-app                  # 只测界面层（131 项，无头跑帧，不碰网络/剪贴板/钥匙串）
cargo test  --workspace -- --ignored    # 跑需要真实网络/真机的集成测试
cargo test  -p ice-providers --test live -- --ignored --nocapture machine
                                        # 只验机翻真实端点（MyMemory，无需 Key）
cargo test  -p ice-providers --test live -- --ignored --nocapture ai
                                        # 只验 AI 真实通道，需先在钥匙串存 Key（见 §6 末）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check              # 格式检查
cargo run  -p ice-app                   # 开发运行
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
4. **不在 UI 层写业务判断**：模式切换、吸附计算、字号自适应等逻辑放在 core/vision，UI 只做展示与事件转发。这样同一逻辑可被单测覆盖。
5. **公共 API 最小化**：能用 `pub(crate)` 的不用 `pub`。
6. 数值上限集中定义（`HISTORY_CAPACITY`、`max_prompt_terms`、`ai_chunk_chars`），不散落魔法数字。

## 6. 测试规范

- 单元测试与被测代码同文件，放 `#[cfg(test)] mod tests`，函数名用 `描述性行为`（如 `second_identical_request_hits_cache_and_skips_engine`）。
- 涉及文件读写的用 `tempfile::tempdir()`，禁止写真实的用户配置目录。
- 需要真实外部服务的测试单独放 `crates/<crate>/tests/live.rs`，标 `#[ignore]`，命名 `<通道>_live_<场景>`（如 `machine_live_short`、`ai_live_deepseek`），通过 `-- --ignored <通道>` 触发。原规范的 `net_` 前缀没采用：同一通道往往要跑两三个场景，把通道名放在前面才能用一条过滤命令选中整组。
- 真实端点的契约测试**必须能被本地假服务替代**：引擎构造接受端点注入（`with_endpoint` / `from_config`），默认测试全部走 `fake_server`，联网用例只是补充，不是唯一证据。
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
| OCR | Vision 框架，本地零下载 | v1 不提供，界面明确提示 |
| 置顶窗口 | `NSWindow.setLevel` / egui `always_on_top` | 各自 API，egui 已封装 |
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
| 机翻报"源语言与目标语言相同" | 源文语种与目标一致（中文源文 + 目标简体中文最常见） | 这不是故障：机翻端点对同语言对必拒，本地提前拦下省一次往返。换目标语言即可（CHG-004 起：源语言为"自动检测"时改为自动翻转目标语言，见 `06-test-cases.md` TC-1.2-c） |
| 第二次启动直接退出 | 单实例锁被占（这是预期行为） | 若确认没有另一个实例在跑，是上次被 `kill -9` 留下的陈旧锁；重启会自动接管（PID 已死即删），也可手动 `rm ~/Library/Application\ Support/IceTranslator/ice-translator.lock` |
| 失败日志只有 `error="Status"` | 变体名之外还需要状态码才能定位 | 已补 `status=` 字段（`jobs::status_code`）；状态码不是秘密，响应体已截断、查询串已洗掉 |
| **应用跑着却一行日志都没有** | `RUST_LOG` 的 target 写错：二进制 crate 名是 **`ice_translator`**，不是目录名 `ice-app`。`RUST_LOG=ice_app=debug` 会把全部日志过滤掉，且进程照常运行、看不出异常 | 用 `RUST_LOG=ice_translator=debug,info ./target/debug/ice-translator > /tmp/ice.log 2>&1`。**真机取证前先看一眼日志有没有行**——P4 第四轮就因为这一条丢了钥匙串写入/清除的日志证据，只能改用用户口述 + 指纹变化间接印证 |
| **`pbcopy` 灌中文样本，用户粘出来是空的** | 无 locale 的 shell 里 `pbcopy` 遇到非 ASCII 会**静默失败**（exit 0、剪贴板为空），ASCII 内容不受影响 | `LC_ALL=en_US.UTF-8 pbcopy < /tmp/sample.txt`，再用 `LC_ALL=en_US.UTF-8 pbpaste \| wc -m` 回验非零，才交给用户粘贴 |
| 大文本走 AI 通道报"被限流（HTTP 429）" | 18,000 字符会切成 6 段连发，撞供应商配额；不是缺陷 | 换机翻引擎跑同一份样本（实测 6 段 11.85s 出结果），或减小样本、隔几分钟再试。日志只记 `error="RateLimited" status=429`，不含源文 |
