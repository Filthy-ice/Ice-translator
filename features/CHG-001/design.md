Change: CHG-001
Version: v0.1
Spec-Hash: 213335f8c4be153a769af710f4e6d51127c1876ccf53684f68924c80823b8b3f

# CHG-001 · Design

## 实现方案

仓库：`ice-translator`（本仓库自身），目标分支 `main`，基线为移植提交 `524943f`。
本变更包不改 `crates/` 的 Rust 代码，只涉及工作区层文件与文档；实现动作已经在 `524943f` 落地，
下面的验证是对已提交内容取证，而不是先批准后补做。

复用点：引擎、模板、技能、流程文档按来源原样保留，便于将来与来源工作区对账；
差异集中在三处（工程名、`scan` 跳过 `.DS_Store`、新增 `check_traceability.py`），
逐条记录在 `docs/verification.md`。

## REQ-001 实现与验证

入口：`sdd`（launcher，把 `tools/` 加入 `sys.path` 后调 `tools/sdd.py::main`）。
工作区定位：`tools/sdd.py::root_for` 从当前目录向上找 `project/config.json`，或接受 `--root`；
本仓库根目录存在该文件，因此根目录与任意子目录均可运行。
初始化产物：`./sdd init .` 生成 `project/`、`codebase/`、`features/`、`quick/`、`incidents/`、`templates/`，
`./sdd repo ice-translator --path . --branch main` 写入 `codebase/repositories.json`（入库）
与 `.sdd/repos.local.json`（被忽略）。
验证：`./sdd validate --offline` 退出码 0，证明工作区结构完整且不依赖仓库外路径。
边界：`.sdd/` 缺失时 `root_for` 仍能定位工作区，只有需要本机路径的命令会要求重新 `repo` 绑定。

同仓布局的指纹边界（实施中发现并修实的缺陷）：来源工作区把规格放在代码仓库之外，`check` 前后对登记仓库
整棵树取内容哈希；本包把规格与代码放在同一仓库，于是账本自身的写入——`features/**` 的证据与状态、
`project/STATE.md`、`.sdd/` 写锁（内容是 PID）——会改变那棵树，使刚记录的证据立刻过期，
`transition VERIFIED` 被"检查后代码改变，证据过期"阻塞。
修法：`fingerprint(path, exclude)` 与 `ledger_for(root, repo)`，仅当登记仓库就是工作区根时排除 `LEDGER` 路径；
分开布局下排除集为空，与来源行为一致。排除的是证据存放处，不是被测代码——
`crates/`、`docs/`、`tools/`、`templates/` 的变动仍会使证据过期，由 `SameRepoWorkspace` 4 项测试锁住。

## REQ-002 实现与验证

`tests/test_workflow.py` 覆盖批准、哈希漂移、计划封存、追踪映射、证据过期、写锁、Quick 风险评估、
同仓布局证据保鲜等 36 项，
每项在系统临时目录建立独立 Git 仓库与独立工作区，结束即清理，不读写 `crates/`、`docs/` 或本工作区规格目录。
验证：`python3 -B tests/test_workflow.py` 全绿。`-B` 阻止生成 `__pycache__`，避免污染仓库。
异常路径：任一测试失败即视为引擎被改坏，相关改动不得提交。

## REQ-003 实现与验证

两道自动门禁 + 一道人工核查：

- `./sdd scan`（`tools/sdd.py::scan`）遍历规格目录、`skills/`、`docs/`，按 `SECRETS` 规则匹配私钥、令牌、
  明文凭据、内网 IP、邮箱、手机号、内部域名；命中只报文件、行号与规则名。
- `python3 -B tools/check_staged.py` 读 `git show :path` 的**暂存区字节**，因此"提交前从工作树删掉密钥"也拦得住。
- 人工关键字核查：其他项目的名称、业务术语、内部主机名不在 `SECRETS` 规则覆盖范围内，
  引入外部文件时必须单独扫一遍（本包移植时即按此法确认，结论见 `docs/verification.md`）。

`.DS_Store` 处理：`scan` 对非 UTF-8 文件报"需人工检查"，而 `.DS_Store` 是 Finder 自动生成的二进制垃圾，
每次都会误报。已在 `scan` 中按文件名跳过，不改其他判定规则。
边界：这是基线规则不是完整 DLP；真实工单 ID、自定义内部域名、分段凭据仍需人工判断。

## REQ-004 实现与验证

`tools/check_traceability.py` 用同一条正则从 `docs/01-requirements.md` 与 `docs/traceability.md`
的表格首列提取需求编号，做双向差集；不一致时打印缺失/多余编号并以退出码 2 失败。
它从当前目录向上定位工作区根，因此在子目录执行结果一致。
约定：需求语义变化先改 `docs/` 基线并在 `docs/07-acceptance.md` 变更登记，Feature 的 spec 只引用编号，不复制需求文本。

## 影响与回滚

影响文件：新增 `sdd`、`tools/`、`templates/`、`skills/`、`features/`、`project/`、`codebase/`、`tests/`、
`.githooks/`、`.gitattributes`、`AGENTS.md`、`docs/{workflow,verification,design-decisions}.md`；
修改 `.gitignore`、`README.md`、`docs/05-dev-guide.md`、`docs/07-acceptance.md`。
不影响运行时行为：不新增 Rust 依赖，不改 `crates/`，不改配置格式与磁盘布局。
兼容性：`.gitattributes` 对规格目录关闭换行转换，因为批准哈希按原始字节计算；Windows 上 checkout 也不会改写。
回滚：`git revert 524943f` 即移除整个工作区层，代码与基线文档不受影响；
`.sdd/` 是本机数据，回滚不涉及。回滚后需删除 `.githooks` 配置（若用户已设置 `core.hooksPath`）。

## 验证层次

本地即可全部证明：结构校验、引擎回归、敏感信息扫描、编号一致性，均为 `VERIFIED_CODE` + `environment=local`。
不需要 Mock、沙箱或生产环境——本变更包没有外部契约。
业务验收对应"用户确认这套记录方式可用并继续用它做 P3"，在 VERIFIED 之后进行。
外部授权需求：仅 `git config core.hooksPath .githooks`（改本机 git 配置，由用户自行执行，工具不代做）。
