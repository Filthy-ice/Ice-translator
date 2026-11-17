# CHG-015 · Design

## 实现方案

- 仓库：`ice-translator`（`/Users/filthy-ice/Desktop/my-translator/Ice-translator`），分支 main，基线 97c4c70（CHG-014 实施中，T-003 粗细色板与 T-008 收口未交）。
- 涉及文件：`crates/ice-app/src/system/annotate.rs`（撤销栈）、`system/selection.rs`（遮罩撤销键 + 工具条/手势抽公共）、`system/pin.rs`（显示优先级 + 编辑态 + 按钮表）、`system/pill.rs`（四颗按钮）、`state.rs`（Pin.anno 字段 + 提交重写 + 成品折叠）、`app.rs`（confirm_capture 传底图、编辑意图分派、copy/save 取合成）。
- 总原则：**层是唯一事实源**。显示 = 底图 + 层（双纹理叠画或合成一份，见 REQ-002）；烧录只发生在三个出口——确认裁图（crop_selection，既有）、编辑提交重写历史 PNG（新增，调同一个 composite_into）、复制/保存（displayed 已含合成）。合成实现永远只有 `composite_into` 一处。
- 前置：CHG-014 的八件工具与药丸先齐活（T-003/008 收口），本包的工具条抽公共才有完整对象可抽。

## REQ-001 实现与验证

**撤销栈（annotate.rs）**：
- `Layer` 加 `undo: Vec<UndoEntry>`；`UndoEntry { parts: Vec<(Rect, RgbaImage)> }`——一笔的脏矩形前像集合。单击类工具一个 part；铅笔/橡皮一次拖拽累积多个 part（每段落像素前先抄该段包围盒的前像），松手时整段算一条入栈。
- 落层入口统一走"先抄前像再落像素"的包装（八件工具 + erase 都经它），**不在每个工具里各写一份快照逻辑**。预览矢量不落层、不入栈。
- `Layer::undo() -> bool`：弹出栈顶，把各 part 前像逐像素贴回，bump revision；空栈返回 false。
- 双上限：入栈时若 `len >= 30` 或 `总字节 + 新条 > 64MB`，从栈首丢弃直到放得下。字节数按 part 像素数 ×4 累计。
- 文字输入开着时遮罩/编辑器不截获 Cmd/Ctrl+Z（egui TextEdit 自带撤销）；判据与 commit_text_input 同一格。

**接线（selection.rs / pin.rs）**：工具态下读 `Key::Z + (command|ctrl)`，pressed 且非 repeat，调 layer.undo()；成功则 request_repaint。遮罩与编辑器各一处调用、同一个 Layer 方法。

**验证**：无头——画一笔撤销后逐像素等于落笔前（H1）；铅笔拖三段撤销一次全没；橡皮擦半条框撤销后另半条与擦痕都恢复原样；31 笔后栈长 30；构造大包围盒使字节超限验证丢最旧；空栈 undo 返回 false 不动 revision。

## REQ-002 实现与验证

**状态（state.rs）**：`Pin` 加 `anno: Option<Anno>`，`Anno { base: RgbaImage, layer: Layer }`。`confirm_capture`（app.rs:1560）在裁图烧录之外，当层 revision>0 时把**未烧录裁图**（crop_selection 不传 layer 的那一份）与层 Arc 的快照交给 `pin_confirmed_capture` → Pin.anno。层为空不建 Anno（回归边界）。
- 注意确认时遮罩即关、层 Arc 随遮罩清：Anno 持有的是层的**所有权转移**（clone 一份像素与撤销栈），不借遮罩的 Arc。

**显示（pin.rs）**：`displayed()` 优先级改为 **anno(底图+层合成) > product > source > file**。anno 档返回 Owned(合成图)；`visual_of` 缓存键加 anno revision（kind 加一档或键里带 revision），层变即重建纹理。复制/保存已走 displayed（app.rs:1809），自动一致，不开新接缝。
- 合成用 composite_into 一份份出（显示档每次 revision 变才算一次，不是每帧）；与遮罩的双纹理叠画不同——贴图窗要的是"复制/保存/显示同一份"，合成一份最稳。

**折叠（state.rs）**：翻译成品回信落 Pin.product 时，若 anno 存在则置 None（成品已含烧录标注，事实 12）。识别底图（source）回信**不折叠**——source 只是翻译链的中间物，显示仍走 anno。

**验证**：无头——空层确认不建 Anno；带层确认后 displayed == 底图+层合成且与盘上烧录 PNG 逐像素同；revision 变缓存键变；成品落位折叠 anno；copy_pin 取到的与显示一致。

## REQ-003 实现与验证

**按钮表（pin.rs）**：`PinAction` 加 `Annotate`（label "标注"、menu_label "标注这张图"）；`BAND_ACTIONS` 3→4（末位追加）、`ACTIONS` 4→5（Annotate 插在 Close 前）；`exactly_four_actions` 改 `exactly_five_actions` 锁新长度，注释写明改判来源（CHG-011 Q-006 → CHG-015 REQ-003）。`PinIntent` 加 `Annotate`；pill_buttons 读表自动多一颗，药丸宽度 width_for(4) 同源。
- 编辑态中药丸不 show（REQ-004），翻译禁用走 ui.add_enabled_ui 既有口径。

**验证**：无头——表长 4/5、顺序断言、Annotate 在 Close 前、label 文案、intent 映射。

## REQ-004 实现与验证

**编辑态（pin.rs + state.rs）**：`Pin.editor: bool`（状态层，帧循环可见）。点 Annotate → editor=true、band_open 收（药丸不 show）。贴图窗 show 里 editor 档：
- 窗尺寸不变（图片尺寸）；工具条以 Area 浮在图片下缘内侧（圆角药丸，与遮罩同风格），下缘放不下翻上缘——复用遮罩那条的翻转判据。
- **工具条与手势抽公共**：把 selection.rs 里"八工具图标 + 粗细色板 + 复制/保存/对勾 + 工具态手势分派"抽成共享函数（遮罩闭包与编辑器闭包各调一次），ToolState 按 pin id 存根 ctx data（照 selection 那套键控）。这是本包最大的重构面，抽完遮罩行为必须逐项不变（V-002 兜底）。
- 手势：有工具画层（point_to_px 漏斗，编辑器 ppp 取贴图窗当前帧）、无工具拖窗（既有 movable 逻辑）。esc 层级：输入→工具→关编辑器（提交）。
- **提交**：对勾或末层 esc → 烧录 base+layer（composite_into）重写 pin.path 那一个 PNG（capture::save_shot 的覆写档或同目录原子替换，不新增文件）；editor=false、药丸恢复。提交时若该张识别/翻译在飞：bump recognize_gen 作废回信（既有 generation 机制），stage 回 Idle。
- 编辑期间右键菜单「翻译」禁用；复制/保存走 displayed（anno 档）。
- 撤销键接 REQ-001 同一套。

**验证**：无头——提交后盘上 PNG == base+layer 烧录；提交不新增文件数；在飞作废后旧回信被丢弃（generation 断言）；editor 档不产生选区/认窗手势（形状核查）；esc 三层顺序。真机 W-15.1 验手感。

## REQ-005 实现与验证

**烧录图新建层（pin.rs/state.rs）**：点 Annotate 时 anno 为 None → 取 displayed() 当前那份像素 clone 为 base、建空 Layer（尺寸=底图尺寸），Anno 成立。此后与 REQ-004 完全同路。橡皮只作用于新层（层模型天然如此，无需特殊代码）；文档写明边界。
- 成品折叠后的贴图：base=成品像素（displayed 优先级 anno 为空时取 product），新层盖成品。

**验证**：无头——烧录图建层后 displayed == 烧录图+新笔；橡皮擦新笔 alpha=0、旧烧录笔画像素不变；提交重写后 F2 再钉出来看到新笔。

## REQ-006 实现与验证

**回归与静态**：cargo test --workspace 全绿（无标注贴图逐手势不变）；clippy -D warnings；cargo tree -p ice-app -e normal 与基线逐行同（零新增 crate）；./sdd scan 过；不新增 config 键（grep 配置结构断言）。工具条抽公共后遮罩形状断言（写回顺序、像素漏斗、认窗门）全部保持。

## 影响与回滚

- 涉及仓库：仅 ice-translator。文件：annotate.rs / selection.rs / pin.rs / pill.rs / state.rs / app.rs + docs/06、docs/07。
- 兼容性：历史 PNG 格式不变（仍是烧录 RGBA PNG）；config 不变；旧贴图（无 anno）走原路径。
- 数据影响：编辑提交**覆写**既有历史 PNG（同路径同文件名），不新增落盘面；7 天/200 张老化与一键清除不受影响。
- 回滚：本包各任务独立 commit，回滚即 revert 对应提交；anno 字段去掉后 Pin 回到只持 path 的现状，无迁移。
- 改判登记：CHG-011 Q-006 按钮数（3→4/4→5）在 docs/07 与本包 spec 写明，合并走查时 revise CHG-011；CHG-014 Q-004/Q-005 的"另包"由本包兑现，不 revise。

## 验证层次

- 本地无头：V-001（ice-app 定向）、V-002（workspace 全量）、V-003（clippy）、V-004（依赖零增量）、V-005（scan）、V-006（形状 grep）。
- 业务验收：V-007 / W-15.1 用户真机——遮罩画几笔 ctrl+z 逐笔退、钉住后点标注续画、橡皮擦新笔、esc 关闭保留改动、F2 历史图标注只擦得掉新笔、翻译链不回退。观感手感只有用户眼睛能判。
- 无沙箱/生产操作；不 push（待批队列纪律）。