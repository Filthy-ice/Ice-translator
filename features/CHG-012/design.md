# CHG-012 · Design

## 实现方案

仓库 `ice-translator`，分支按 `codebase/repositories.json` 登记；基线 commit `58926a9`（本包开工那笔）。入口仍是 Carbon F2 → `global_hotkeys` → `Action::ReopenLast`（这一段照 CHG-009 REQ-006 不变），本包只改 `ReopenLast` 之后"取哪一张、怎么记游标"。

模块落点（三处代码 ＋ 两处文档）：

| 落点 | 改什么 | 为什么在这里 |
|---|---|---|
| `system/capture.rs` | 加 `next_older(shots, anchor) -> Option<&Shot>` 纯函数，复用 `sorted`；`recent` 保留不动 | 排序唯一出处在这（事实 1），"下一张更早"与老化共用一个序 |
| `state.rs` | `Pin` 加 `history_browse: bool`（`Pin::new` 默认 false）；`AppState` 加 `browse_anchor: Option<PathBuf>`（`new` 默认 None，**不进 `Config`**）；加一个**只给 F2** 的薄包装 `browse_older`；`close_pin` 末尾接重置规则 | 标记与游标都是状态；薄包装避开共用的 `pin_from_history`（事实 2） |
| `app.rs` | `ReopenLast` 那一支：读 shots → 空/最早/钉满三档分流 → 调 `browse_older` | 读盘在帧末这一层，状态层不碰 fs（CHG-009 REQ-005 分层纪律） |
| `docs/07` | 登记改口 CHG-009 REQ-006/REQ-008/Q-011 与落地次序 | 改口关系要双向可查 |
| `docs/06` | 加 W-12.1 真机走查与无头 TC | 验证层次落账 |

**关键约束**：`pin_from_history` 被 F2、遮罩确认 `pin_confirmed_capture`、单测三处共用（事实 2），**本包不改它一行**；F2 的来源标记与游标推进只落在新包装 `browse_older` 里，F1/F3 走原路、不带标记。

## REQ-001 实现与验证

调用链：Carbon F2 → `Action::ReopenLast`（不变）→ `app.rs` 读盘（`capture::list`）→ `capture::next_older` → `state.browse_older` → `pin_from_history`（复用，不改）→ `push_pin` ＋ `pins_new` 派后台只识别。

`next_older` 形状（纯函数，不碰 fs，锚点用路径不用下标，事实 6）：

```rust
pub fn next_older<'a>(shots: &'a [Shot], anchor: Option<&Path>) -> Option<&'a Shot> {
    let order = sorted(shots);
    match anchor {
        None => order.first().copied(),
        Some(path) => match order.iter().position(|s| s.path == path) {
            Some(i) => order.get(i + 1).copied(), // i 是最早那张时 get(i+1)=None
            None => order.first().copied(),       // 锚点被老化/清除：回退到最新重新起翻
        },
    }
}
```

`browse_older`（只给 F2 的薄包装，复用同一条钉图路，不开第二条）：

```rust
pub fn browse_older(&mut self, path: PathBuf) -> Option<RequestId> {
    let id = self.pin_from_history(path.clone(), None)?; // 满 6 时这里就 None，锚点不动
    if let Some(pin) = self.pins.iter_mut().find(|p| p.id == id) {
        pin.history_browse = true;
    }
    self.browse_anchor = Some(path);
    Some(id)
}
```

复用点：`sorted`（与老化同源）、`pin_from_history` / `push_pin` / `pins_new`（钉图与派识别那一条路）。每按一次新钉一张、累积；不派翻译（`pins_new` 只派"只识别"，CHG-009 REQ-001 主干）。
验证：无头 TC 锁 `next_older`（空锚点＝最新、有锚点＝下一张、锚点是最早＝None、锚点消失＝回退最新、同分按路径）；真机 W-12.1 锁"每按一次新钉一张更早的、累积"。

## REQ-002 实现与验证

三档分流都在 `app.rs` 帧末 `ReopenLast` 那一支（读盘在这层）：

```rust
Action::ReopenLast => {
    let shots = self.shots_dir.clone().map(|d| capture::list(&d)).unwrap_or_default();
    if shots.is_empty() {
        self.state.toast(Tone::Info, "还没有截图记录");
    } else {
        match capture::next_older(&shots, self.state.browse_anchor.as_deref()) {
            None => self.state.toast(Tone::Info, "没有更早的截图了"),
            Some(shot) => {
                if self.state.browse_older(shot.path.clone()).is_none() {
                    self.state.toast(Tone::Info, "屏幕上已经钉满 6 张，先 esc 关掉一张");
                }
            }
        }
    }
}
```

异常传播：`push_pin` 满返回 `None` 是既有形状；`browse_older` 在 `pin_from_history` 返回 `None` 时**直接 `?` 退出**，因此不置 `history_browse`、不推进 `browse_anchor`（先钉成功再记游标）。最早那一档 `next_older` 返回 `None`，压根不调 `browse_older`，锚点自然不动。`newest_shot`（`app.rs:1278`）F2 不再用，可删可留（留着不影响行为）。
验证：无头 TC 锁三档 ＋"满/最早时锚点不变"；真机复核三句文案读不读得懂。

## REQ-003 实现与验证

状态流：

- `Pin.history_browse`：只有 `browse_older` 成功那一支置 true；`push_pin` 被 F1（遮罩确认 `pin_confirmed_capture` → `pin_from_history`）/ F3（`pin_last_capture`）调用时走 `Pin::new` 默认 false（事实 2 的隔离由此兑现）。
- `AppState.browse_anchor`：运行态，`new()` 默认 None；**不进 `Config`、不进 `config.toml`、不持久化**（与 `hotkey_capture` 同族）。
- `close_pin` 末尾接重置：

```rust
pub fn close_pin(&mut self, id: RequestId) -> bool {
    let before = self.pins.len();
    self.pins.retain(|pin| pin.id != id);
    if !self.pins.iter().any(|p| p.history_browse) {
        self.browse_anchor = None; // 再没有一张 F2 翻出来的图，才重置
    }
    self.pins.len() != before
}
```

锚点那张关了、但别的 `history_browse` 还开 → `any` 为真 → 不重置（下一次接着往更早翻）。F1/F3 的图不带标记 → 它们开着不算、关掉也不触发重置。
幂等/并发：`close_pin` 按号摘、号不复用；重置判据是"当前集合里有没有标记"，与关的顺序无关。重启＝`AppState::new` → `browse_anchor=None`。
验证：无头 TC 锁"关到最后一张历史贴图才重置""F1/F3 不计入""锚点那张关了别的还开→不重置""新建 AppState 游标空"；静态 V-003 grep `config.rs` 无 `browse_anchor`/`history_browse`。

## REQ-004 实现与验证

改口登记：`docs/07` 加一行"CHG-012 改口 CHG-009 REQ-006 末句 / REQ-008 第 4 句的'打开上一次＝最近那张'为'依次往回翻'，Q-011 形态（钉成贴图）不变"；本包 spec「原始意图与来源」已双向指认。**不动 CHG-009 任何文件**。
次序：CHG-009 `ACCEPTED` 之后才 `transition CHG-012 IMPLEMENTING` 并改代码（事实 5：`docs/` 与 `crates/` 都在树指纹内，先改会冲掉 CHG-009 已录证据）。本包批准只封规格/设计/计划，实施那一步等 CHG-009 收口。
不变边界：不加配置字段（V-003）、不加 crate（V-004 `cargo tree` 比对基线 `58926a9`）、`./sdd scan` 通过（V-005）、文案不含文件名（沿用 CHG-009 既有断言）。
回滚：改动集中在三个文件＋一格状态＋一个标记；回滚＝revert 本包那笔 commit，CHG-009 行为自动恢复（其快照未动）。

## 影响与回滚

涉及仓库 `ice-translator`；文件 `capture.rs` / `state.rs` / `app.rs` ＋ `docs/06` / `docs/07`。兼容性：无配置/数据迁移（游标运行态、标记默认 false，旧状态构造照跑）。回滚条件：W-12.1 做不出累积手感，或实施会冲掉 CHG-009 证据（停止条件 1）；回滚方法：revert 本包 commit。

## 验证层次

本地无头（`cargo test -p ice-app` / `--workspace`）锁纯函数与状态规则、且既有全量不回退；静态（`./sdd scan` / `config.rs` grep / `cargo tree`）锁边界；真机 **W-12.1**（`BUSINESS_CONFIRMED`）锁物理 F2 的累积手感——注入键不触发 Carbon 热键（事实 4），这一格只有用户按得出，**不许拿日志冒充**。CHG-009 的物理键命中（W-5.1）已证，本包不重证热键送达，只证"翻历史"这一层新行为。