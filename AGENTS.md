# Ice Translator · 工作区指令

Rust 跨平台翻译客户端。规格与代码同仓：SDD 记录平铺在根目录（`features/`、`quick/`、`incidents/`、
`project/`、`codebase/`、`templates/`），实现在 `crates/`，项目级基线文档在 `docs/`。
`.sdd/` 只存本机绝对路径、写锁与备份，已被忽略。

先读 `project/STATE.md` 与当前 Feature，再读 `codebase/ARCHITECTURE.md` 与实际代码；按需加载设计、任务和证据。

## 仓库是公开的

`github.com/Filthy-ice/Ice-translator` 未登录可访问。**任何提交进去的内容全世界可读。**
禁止放入：其他公司项目的代码、文档、术语、内部域名、真实账号、未脱敏日志或截图。
从别处引入文件前先确认来源与归属，并跑一遍 `./sdd scan` 与 `python3 -B tools/check_staged.py`。

## 工作边界

- 业务行为以批准的 `spec.md` + SHA-256 快照为准；`change.json` 是状态、版本和批准元数据源。
- INTAKE/CLARIFIED/SPEC_REVIEW 可调查、编辑规格，不改业务代码。仅在真实授权确认当前哈希后记录 approve；
  不得把草稿、沉默或旧版本批准当成新批准。
- 已批准行为可自主实施；新增业务判断交回用户；外部副作用按会话实际授权执行。批准 local-delivery 不授予部署或生产写权限。
- APPROVED 可设计与拆任务；`seal-plan` 后 DESIGNED；进入 IMPLEMENTING 才实施映射任务。
- `tasks.json`、`traceability.json`、`evidence.json` 是机器可校验记录；`tasks.md`/`evidence.md` 是生成视图，不手改。
- 验证引用当前批准哈希和 plan_hash；源码变动使旧证据过期。VERIFIED 后记录实际业务验收才能 ACCEPTED。
  本仓库规格与代码同仓，被测哈希已排除账本路径（`features/ quick/ incidents/ project/STATE.md .sdd/`），
  不要再把"证据写完即过期"当成 bug 去改门禁。
- 外部契约推翻前提时记录 SPEC_DEVIATION 并退回 SPEC_REVIEW；改规格用 `revise` 递增版本，仅技术方案变化用 `replan`。
- 删除任何东西（含临时文件、旧证据）前先征得同意；不碰用户自己的文件。

## 工程约定

- 工具链 `rust-toolchain.toml` 锁定；命令与规范见 `docs/05-dev-guide.md`。
- 提交前：`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全绿。
- 依赖方向单向（`ice-app` → `ice-providers`/`ice-vision` → `ice-core`），`ice-core` 不得引入 IO/UI/网络（NFR-6，ADR-006）。
- API Key 只进 macOS 钥匙串（service `IceTranslator` / account `ai-api-key`），不写配置、不打日志（ADR-005、NFR-3）。
- 截图证据必须裁剪到仅含本应用窗口，逐张确认无用户桌面其他内容后才提交（`docs/03-ui-design.md` §5）。
- 每完成一个阶段的验收：`docs/07-acceptance.md` 记录结论，然后 commit + push 到 `origin/main`。

## 快速入口

`./sdd --help` 查看工具；`./sdd status CHG-001` 恢复现场；`./sdd validate` 检查门禁；`./sdd index` 刷新导航。
工具在工程根目录用 `./sdd ...`，从其他目录启动用 `./sdd --root <工程路径> ...`。
流程与数据格式见 `docs/workflow.md`；阶段计划见 `project/ROADMAP.md`；需求基线见 `docs/01-requirements.md`。
技能源在 `skills/`，安装：`python3 tools/install_skills.py --dest <技能目录>`（只新增，不覆盖不同版本）。

## 与 docs/ 基线的关系

`docs/00~07` 是 v1.0 的项目级基线（需求、架构、UI、图像流水线、开发、用例、验收、追踪矩阵），
`features/CHG-*/` 是每个变更包的执行记录与证据。基线变更要同时改 `docs/`，并在对应 Feature 的
`spec.md` 里引用需求编号；两边不一致时以 `docs/01-requirements.md` 为准并修正 Feature。
