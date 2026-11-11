Change: CHG-013
Version: v0.1
Spec-Hash: PENDING

# CHG-013 · Design

## 实现方案

仓库 `ice-translator`，分支按 `codebase/repositories.json` 登记；基线 commit `ef92028`（本包开工那笔）。入口仍是 `submit_capture`（F1 与图片页「框选贴图」共用，调用点唯一性由 `views/image.rs:1944` 守着，本包不动它），本包只去掉派发前后那一对"收起自家窗 / 原样放回来"。

**核心区分（事实 10，全包的命门）**：`capture::show_again` 现在干两件事——① 对 hide 列表发 `Visible(true)`（可见性还原）；② 按 `wake_commands()` 买一帧（`RequestPaintWhileHidden` + `request_repaint`）。**本包只删 ①，保留 ②**。② 是 CHG-009 REQ-006 隐藏态唤醒的命脉：托盘里按热键时主窗本就收着，子进程回来要靠 `RequestPaintWhileHidden` 那一帧才建得出遮罩/贴图（事实 30/31，`jobs.rs:468-469` 注释明说"隐藏态那一次按下正是这一格"）。`RequestPaintWhileHidden` 在全仓只出现在 `capture.rs:490`（经 `wake_commands`），删了它 REQ-006 就回归 D-21。

模块落点（四处代码 ＋ 两处文档）：

| 落点 | 改什么 | 为什么在这里 |
|---|---|---|
| `app.rs:1798-1801` | 去掉 `capture::hide(ctx, hidden.viewports.clone())` 那一支（连同只为取 hide 列表的解构） | 收起的发起点；`submit_capture` 本身不动 |
| `system/capture.rs` | `show_again` 删掉 `for id in hidden.viewports { Visible(true) }` 那一段，**保留 `wake_commands` 循环**；`hide`（`:462`）与 `Hidden`（`:432`）若再无调用方则删；`wake_commands`（`:456`）/`Wake`（`:440`）**原样保留** | 可见性还原与唤醒本就拧在一个函数里，这里把两件事拆开；唤醒那半是 REQ-006 共用的 |
| `jobs.rs:434`/`:470` | `show_again(&ctx, &hidden)` 改成只叫醒帧循环的那一支（不再传 hide 列表）；`Job::Freeze`/`Job::ScreenCapture` 的 `hidden` 字段随之去掉 | 子进程完成那条线程；叫醒那一段必须留着（建遮罩/贴图） |
| `state.rs:1554` | `capture_hidden_record` 与 `Job` 里的 `hidden` 字段去掉；断言 `hidden.viewports.len()==3`（`:2938`/`:2952`）改判成"不再产生 hide 列表/不再发 `Visible(false)`" | hide 列表的出处；本包以删除为主 |
| `docs/07` | 登记改口 CHG-009 REQ-010/Q-003（(a)→(c)）与落地次序 | 改口关系双向可查 |
| `docs/06` | 加 W-13.1 真机走查与无头 TC | 验证层次落账 |

**关键约束**：`wake_commands()` 与 `RequestPaintWhileHidden` 一行不删（事实 10）；`submit_capture` 派发口不动（`image.rs:1944` 断言）；失焦半透明 `dim_when_unfocused` 不动（非目标，Q-002）。`Job::ScreenCapture` 是 CHG-010 之后留着不接线的旧链（`state.rs:289`），本包顺带把它的 `hidden` 字段一并去掉以保持一致，但它仍不接线。

## REQ-001 实现与验证

调用链（改后）：F1/「框选贴图」→ `submit_capture`（不变）→ `Job::Freeze`（不再带 hidden）→ **不再 `capture::hide`** → `spawn_blocking` 分屏抓屏（不变）→ 完成线程**只叫醒帧循环**（`wake_commands`：`RequestPaintWhileHidden` + `request_repaint`）→ 回信 `Outcome::Froze` → 帧循环建遮罩。

`show_again` 拆成只唤醒（示意，名字落地时定）：

```rust
// 删掉 visible 还原那一段，只留唤醒；hidden 参数随之去掉
pub fn wake_after_capture(ctx: &egui::Context) {
    for command in wake_commands() {
        match command {
            Wake::PaintWhileHidden => ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::RequestPaintWhileHidden,
            ),
            Wake::Repaint => ctx.request_repaint(),
        }
    }
}
```

为什么主窗可见时 `RequestPaintWhileHidden` 也无害：它只保一帧"隐藏中也画"，主窗本就可见时这一帧照常画，不产生副作用；托盘隐藏时它正是建遮罩的那一帧（事实 30/31）。所以**两种情形统一保留唤醒**，不必按"主窗开没开"分流（那正是被事实 5 否掉的旧判据 `root_was_hidden`）。
验证：无头 TC 锁"`submit_capture` 之后不产生 hide 列表、不再发 `Visible(false)`"＋"唤醒仍发 `RequestPaintWhileHidden` + `request_repaint`"＋既有全量不回退；真机 W-13.1 锁"聚焦全量窗发起，定格不再有半透明背景框、还原不再白 1~3 秒"。

## REQ-002 实现与验证

行为上"什么都不做"即是实现：不再收起 ⇒ 自家窗按当前不透明状态进冻结帧（正常像）。已知限制登记在 `docs/07` 与本包 spec（Q-003 (c)）。
是否加界面提示见 Q-001（默认不加，沿用 #88 首用提示那一包）；本包不新增提示控件、不新增配置字段。
验证：真机 W-13.1（用户看自家窗可见时进图是不是正常不透明像、能不能接受）；静态确认没有新增"替用户隐藏"的写法。

## REQ-003 实现与验证

改口登记：`docs/07` 加一行"CHG-013 改口 CHG-009 REQ-010/Q-003：由 (a) 发起前收起自家窗、结束原样回来，退到 (c) 不收起、登记已知限制；真机依据是半透明背景框（异步收起 race）与还原 1~3s 白屏"；本包 spec「原始意图与来源」已双向指认。**不动 CHG-009/CHG-010 任何文件**。
次序：CHG-009 与 CHG-010 都 `ACCEPTED` 之后才 `transition CHG-013 IMPLEMENTING` 并改代码（事实 9：`docs/` 与 `crates/` 都在树指纹内，先改会冲掉两者已录证据）。本包批准只封规格/设计/计划，实施那一步等两者收口。
不变边界：不加配置字段、不加 crate（`cargo tree` 比对基线 `ef92028`）、`./sdd scan` 通过；**grep 确认 `RequestPaintWhileHidden` 仍在**（REQ-006 唤醒，事实 10）；文案不含文件名（沿用 CHG-009 既有断言）。
回滚：改动集中在四个文件、以删除为主；回滚＝revert 本包那笔 commit，CHG-009/CHG-010 行为自动恢复（其快照未动）。

## 影响与回滚

涉及仓库 `ice-translator`；文件 `app.rs` / `jobs.rs` / `system/capture.rs` / `state.rs` ＋ `docs/06` / `docs/07`。兼容性：无配置/数据迁移（hide 列表是运行态，去掉后旧状态构造照跑）。回滚条件：W-13.1 仍白 1~3s（停止条件 4，说明白屏另有根 #107）、或用户不接受自家窗进图（停止条件 5）、或实施会冲掉 CHG-009/CHG-010 证据（停止条件 1）；回滚方法：revert 本包 commit。

## 验证层次

本地无头（`cargo test -p ice-app` / `--workspace`）锁"不再收起/不再发 `Visible(false)`"与"唤醒仍发 `RequestPaintWhileHidden`"、且既有全量不回退；静态（`./sdd scan` / `cargo tree` / grep `RequestPaintWhileHidden`）锁边界与 REQ-006 不回归；真机 **W-13.1**（`BUSINESS_CONFIRMED`）锁"聚焦全量窗发起一轮：背景框消失、还原不白、自家窗可见时进图是正常像"——观感只有用户眼睛能判，**不许拿日志冒充**。遮罩首帧那 ~0.1s 白（#107）不在本包验收范围，分开记。