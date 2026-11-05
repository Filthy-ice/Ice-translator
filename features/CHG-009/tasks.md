Change: CHG-009
Version: v0.1
Spec-Hash: UNAPPROVED

# Tasks（由 tasks.json 生成，勿直接修改）
- [ ] T-001 真机取证两条假设，证完再写代码：H1（`screencapture -i` 被 esc 取消时到底写不写文件、退出码是什么）与 H5（框选期间我们自己的浮窗会不会进图、以什么状态进图）。取消判据 SHALL NOT 写死「退出码 0 即成功」——那是没证过的猜测 | ice-translator | REQ-002, REQ-004
- [ ] T-002 新建 `crates/ice-app/src/system/capture.rs`：mac 分支在 `spawn_blocking` 里跑 `/usr/sbin/screencapture -i -x -o -t png <唯一路径>`，返回 `Captured(PathBuf) | Cancelled | Failed(类别)`；非 mac 是**同签名**恒 `Unsupported` 的门控版本（视图里不许散写 `#[cfg]`）。纯函数段单独成块、可无头测：临时路径生成（pid + 自增，两次连点不互相覆盖）、取消判定（按「文件是否出现」）、失败类别 → 那句能行动的话 | ice-translator | REQ-001, REQ-002, REQ-003, REQ-005, REQ-008
- [ ] T-003 第四条通道：`Job::ScreenCapture` + `CaptureStage{Idle,Capturing,Done,Failed,Cancelled}` + `capture_request_id`（**与识别/翻译/合成三条互不作废**，形状与 CHG-006 那条一致）+ `capture_gate()`——那颗按钮灰不灰与状态行说哪句话读同一个来源；在飞时不许发起第二次 | ice-translator | REQ-001, REQ-002
- [ ] T-004 `jobs.rs` 的分岔与回传：`spawn_blocking` → `try_send` 一条 `Outcome::ScreenCaptured` + `request_repaint`（有界通道、满了就丢，不许 `send().await`）；回信落帧循环后走既有 `ImageInput::Path` 派发识别，并按 REQ-003 **即刻删除临时文件**（像素已进内存），删除失败只记 debug 且不影响结果；日志与提示条**一个文件名都不出现** | ice-translator | REQ-001, REQ-002, REQ-003, REQ-007
- [ ] T-005 界面与快捷键：E 区第一排加一颗「截图翻译」（非 mac 在原地但禁用 + 一句「这台机器……」，延续 FR-2.1 那条「禁用而不是消失」），I 区多一格「截图」段；快捷键走既有 `Action` 枚举的同一个分派（与按钮同一条路径，不开第二条实现），且 SHALL NOT 与 `Cmd+Shift+3/4/5` 相撞；hover 说清三件事（会框选、esc 取消、截完直接进翻译） | ice-translator | REQ-001, REQ-006
- [ ] T-006 自家窗不许进截图（REQ-004）：按 T-001 的实测结论落地 Q-003 选定的那一档——默认「发起前临时取消置顶、结束原样回来」，恢复路径**不许新增第三种唤回方式**（与 CHG-008 的托盘/唤回语义对齐）；真机若「闪一下」不可接受则退回登记已知限制 + 界面一句说明 | ice-translator | REQ-004
- [ ] T-007 三平台分档写进编译面与文档：`capture.rs` 那一处唯一判定 + `docs/05` §8 新增一行（**「截图这一格」与「识别这一格」并排写**：Linux/Windows 即便截到也没有本地 OCR，这条天花板不许被「跨平台」三个字盖住）+ 一条静态核查测试钉住「`screencapture` 字面量与 `#[cfg(target_os)]` 不散落在视图里」 | ice-translator | REQ-005
- [ ] T-008 文档回写与验收口径：`docs/01` 的 FR-2.10 那一行从「列入延期项」转为本包承担、`docs/02` 模块表加 `system::capture`、`docs/03` E 区那一排与 I-01 系列、`docs/06` 新 TC 与走查 W-5.1~W-5.4、`docs/07` 变更登记与计数；跑一次 `nettop -p <pid>`（截图与识别期间零出站、点翻译前也零）与 `cargo tree` 零依赖增量的复核 | ice-translator | REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008
