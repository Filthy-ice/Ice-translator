Change: CHG-016
Version: v0.1
Spec-Hash: 0162d45dab71a15d7703004377ba211ed109de04b696c3b20ece8a1309afb4e2

# CHG-016 · Design

## 实现方案

- 仓库：`ice-translator`（分支 main，基线 0a86a9c，橡皮图标 patch128；CHG-014 v0.2 / CHG-015 v0.1 均 IMPLEMENTING）。
- **架构改判**：标注事实源从"一层像素"（CHG-014 annotate.rs 光栅引擎）换成"一组对象"。像素层降级为**按对象列表重渲染的派生缓存**。CHG-015 的脏矩形前像撤销（T-001/T-002 已落）由对象快照撤销取代。两包已批的**视觉输出与入口承诺**承接，**内部引擎**替换。
- 涉及文件：
  - `crates/ice-app/src/system/annotate.rs`（**重写核心**：AnnoObject 枚举（每件带 erased 擦除痕迹）+ ObjectList + render 复用既有光栅基元 + 对象快照撤销/重做 + 命中/控制点/橡皮绑定对象擦除）；
  - `crates/ice-app/src/system/anno_store.rs`（**新增**：侧车读写 shots/anno/ 子目录，serde_json + image encode）；
  - `crates/ice-app/src/system/capture.rs`（list 补 is_file 过滤跳子目录；clear_all 补清 anno 子目录；retain/age_shots 侧车同步删）；
  - `crates/ice-app/src/system/selection.rs`（遮罩工具态改建对象；工具条/手势抽公共供编辑器复用）；
  - `crates/ice-app/src/system/pin.rs`（displayed anno 档取渲染缓存 + revision 缓存键；按钮表 3→4/4→5；Pin.editor 编辑态）；
  - `crates/ice-app/src/system/pill.rs`（四颗按钮）；
  - `crates/ice-app/src/state.rs`（Anno{base,objects,x0,y0} 取代 Anno{base,layer}；anno_from_capture 建 ObjectList；成品折叠；提交重写历史 PNG + bump recognize_gen）；
  - `crates/ice-app/src/app.rs`（confirm_capture 传 ObjectList；编辑意图分派；copy/save 取合成；age_shots 侧车同步）；
  - `crates/ice-app/Cargo.toml`（加 `serde_json.workspace = true`——既有 workspace crate，零新增 crate 名）；
  - `docs/06`、`docs/07`（收口回写）。
- 总原则：**对象列表是唯一事实源**。显示/复制/保存 = 底图 + render(objects)（合成永远只走 `composite_into` 一处）；烧录只发生在三个出口——确认裁图（crop_selection 既有）、编辑提交重写历史 PNG（调同一个 composite_into）、复制/保存（displayed 已含合成）。**光栅基元一字不动**（paint_segment/arrow_head/stamp_line/draw_text/composite_into/blend_straight/for_each_covered_pixel/mosaic 盒式平均），只是调用方从"手势直接落像素"变成"render 遍历对象落像素"。
- 前置：CHG-014 八件工具视觉基线先锁定（H1 逐像素回归才有比对对象）。CHG-015 T-003 停车的四件脏文件（Anno{base,layer}/crop_base/anno_from_capture/displayed/visual_of）在其上重做：留底图接缝照用，layer 换 objects。

## REQ-001 实现与验证

**对象模型（annotate.rs）**：
- `AnnoObject` 枚举（derive Clone/Debug/Serialize/Deserialize/PartialEq）：`Rect{a,b,width,color}`、`Arrow{from,to,width,color}`、`Line{a,b,width,color}`、`Pencil{points:Vec<(f32,f32)>,width,color}`、`Serial{center,radius,number,color}`、`Text{at,text:String,font_size,color}`、`Mosaic{points:Vec<(f32,f32)>,width,block}`（涂抹笔迹，2026-11-18 改）——**每件再带一个 `erased:Vec<EraseStamp>` 字段**（橡皮对它划过的轨迹段/矩形，存**对象相对坐标**，默认空；EraseStamp 画笔轨迹/矩形区两型，对应 某截图工具 PathMode）。坐标物理像素（事实 13），样式存值不是档索引（档索引是工具态运行态，落对象时换成 THICKNESS/BADGE_RADIUS/TEXT_SIZE/PALETTE 的具体值）。**橡皮不是独立对象类型**，只是往被划过的对象的 erased 里追加（REQ-005）。
- `ObjectList{objects:Vec<AnnoObject>, revision:u64, undo:Vec<Vec<AnnoObject>>, redo:Vec<Vec<AnnoObject>>, next_serial:u32}`——唯一事实源。`next_serial` 序号自增计数（默认 1 起，CHG-014 行为）。
- `render(objects:&[AnnoObject], background:&RgbaImage, w,h) -> RgbaImage`：新建透明层，**按列表序**遍历对象，每件先把几何画进一个临时 tile（Rect→四边 paint_segment；Arrow→杆+arrow_head 两斜线；Line→paint_segment；Pencil→逐段 paint_segment；Serial→实心圆 paint_segment+stamp_line 白数字；Text→stamp_line；Mosaic→涂抹笔迹经过的方块用 background 盒式平均不透明写（mosaic_brush_paint）），再用该件的 `erased` 对**这块 tile 自身** destination-out 减 alpha（复用 CHG-014 `erase` 原语 for_each_covered_pixel，**只擦这个对象的像素不擦底图**），最后把 tile 合成进层。**擦除痕迹存在对象相对坐标里，故随对象平移/变换一起走**（事实 18）。**叠放序=列表序=落笔序**，落墨 alpha-over 与旧引擎逐笔落像素同结果、被擦对象的 destination-out 与旧引擎 erase 同结果（H1 含橡皮）。
- 像素缓存 `Layer{image:RgbaImage, revision:u64}` 降级为 render 的产物：ObjectList 每次突变后重渲染整层、bump revision。`composite_into` 不变（底图＋缓存按 x0/y0 盖合）。
- 旧 `Layer` 的 undo_stack/pending/before_image/stroke 那套脏矩形前像**删除**（被 REQ-003 对象快照取代）；八件旧工具方法（rect_stroke/line/arrow/pencil/number_badge/text/mosaic/erase）改为"建对象"入口，落像素逻辑搬进 render。

**验证**：无头——render 空列表得全透明层；render 单件 Rect 与旧 rect_stroke 落像素逐像素同；composite_into 仍只一处定义；对象坐标物理像素与底图同尺寸。

## REQ-002 实现与验证

**八件工具落对象（selection.rs + annotate.rs）**：遮罩工具态手势不再直接调旧落像素方法，改为建 AnnoObject 推进 ObjectList：
- 框/箭头/直线：拖拽松手建一个对象（几何取按下/抬起两点，样式取当前工具态档换值）；马赛克：涂抹式（像铅笔，拖拽累积轨迹，松手建一个 Mosaic 笔迹对象，2026-11-18 用户要求）；
- 铅笔：一次按下到松开累积 points 建**一个** Pencil 对象（不是每段一个）；
- 序号：单击建 Serial，number=next_serial++（默认 1 起）；
- 文字：单击唤起输入，回车/失焦落 Text 对象（空白不落、next_serial 不动）；
- 橡皮：见 REQ-005（不建对象，把擦除轨迹追加进被划过对象的 erased 字段）。
- 预览矢量（拖拽时看到的框/箭头）仍不落对象、不入栈，松手才建。

**H1 逐像素回归（硬断言）**：同一组笔画，旧引擎按落笔序逐件落像素 vs 新引擎建对象后 render，**七件落墨工具**（框/箭头/直线/铅笔/序号/文字/马赛克）输出逐像素相等，**橡皮部分擦也纳入**（对象 erased 对自身 tile destination-out == CHG-014 `erase`，同笔迹逐像素相等）。马赛克 background 取同一份（遮罩=冻结帧、编辑器=底图）。

**验证**：无头——七件工具各造一组对象，render == 旧引擎落像素（逐像素 assert_eq）；序号自增从 1；文字空白不落；铅笔整段一个对象；无标注（空列表）贴图与今天逐像素一致。

## REQ-003 实现与验证

**对象快照撤销/重做（annotate.rs）**：
- 突变前把 `objects.clone()` 压入 undo（廉价：一组对象，不是像素）；undo 弹栈顶恢复上一份 objects、当前份压 redo、重渲染 bump revision；redo 反之。空栈 undo/redo 返回 false 不动。
- 上限：undo 条数封 30（对象快照廉价，**不再需要 64MB 字节上限**——那是像素前像约束，对象模型下消失，事实 1 那套删除）。redo 栈在新突变时清空（标准语义）。
- 一笔=一次突变：单击类一个对象一条；铅笔/橡皮一次拖拽整段算一条（拖拽期间累积，松手才压一次快照）。撤销橡皮＝恢复被擦对象 erased 追加前的快照、重渲染，被擦的标注复现。
- 键接线（selection.rs 遮罩 / pin.rs 编辑器各一处、同一个 ObjectList 方法）：`Key::Z+(command|ctrl)` pressed 非 repeat → undo；`Key::Z+(command|ctrl)+shift` 或 `Key::Y+ctrl` → redo。文字输入框开着时让位 TextEdit 自带撤销（判据与 commit_text_input 同格）。
- 栈跟着 ObjectList 走：遮罩里画的笔，钉住后编辑器接着撤（ObjectList 所有权转移给 Pin.anno，clone 一份不借遮罩 Arc）。

**验证**：无头——画一笔 undo 后 objects 与渲染逐像素等于落笔前；铅笔拖三段 undo 一次全没；undo 后 redo 恢复；31 笔后 undo 栈长 30；新突变清空 redo；空栈无声不动 revision；橡皮擦除一笔撤销后擦掉的标注逐像素复现。

## REQ-004 实现与验证

**命中＋选中＋全控制点（annotate.rs 纯几何 + pin.rs 手势）**：
- `hit(objects, pt) -> Option<usize>`：从顶（列表尾）往下，按类型**几何**距离判命中（线/箭头/铅笔/马赛克：点到线段距离≤半宽+容差；框：点在边框带内或内部；序号：点在圆内；文字：点在文字包围盒内）——**只看几何不看 erased 缺口，被擦得发虚的部分仍属该对象、照样整条选中**（事实 18）。返回最顶那个索引。
- `control_points(obj) -> Vec<(f32,f32)>`：Rect 四角；Arrow/Line 两端点；Pencil/Mosaic/Serial/Text 无改形控制点（整体移动；马赛克是涂抹笔迹、同铅笔）。
- `drag_control(obj, idx, delta)`：改对应几何点（框拖角改对角、箭头/直线拖端点改向长）；`move_obj(obj, delta)`：整体平移所有几何点（erased 存对象相对坐标故自动跟着走，事实 18）。
- 手势（pin.rs 编辑器，与遮罩同一互斥规则 H2）：有工具时画层/建对象；**无工具时**点对象→选中（出控制点）、拖控制点→改形、拖对象本体→移动、点空白→取消选中、空白处拖→拖窗（既有 movable）。选中时改粗细档/色板→改该对象样式重渲染（不是只改下一笔默认）。Delete/Backspace→删选中对象（压一条撤销快照）。
- 控制点选中即常显（Q-004 一档）；控制点命中半径与画法对齐（既有把手那套读法）。
- 文字再编辑：选中 Text 双击（或选中后按回车）唤起输入框改 text，落对象重渲染。序号改编号：选中 Serial 可改 number。

**验证**：无头——hit 各类型命中/不命中、顶优先；control_points 各类型点数与位置；drag_control 拖角后几何正确；move_obj 平移后所有点同偏移；删选中对象后列表少一件；改样式后对象 color/width 变。真机 W-16.1 验手感（H2：想拖窗却画线/想画却拖窗即停止条件 5）。

## REQ-005 实现与验证

**橡皮＝绑定对象的透明擦除（annotate.rs 对象 erased 字段 + render 对自身减 alpha，Q-001 更正：对齐 某截图工具＋CHG-014 已批行为）**：
- 橡皮是工具、不是对象。一次擦除（画笔轨迹/矩形区两模式，宽取粗细档）SHALL 命中它划过的每个对象，把这段轨迹转成**对象相对坐标**追加到那些对象的 `erased` 字段——不新增对象、不改任何对象的几何。
- render 逐件先画几何进 tile、再用该件 `erased` 对 tile 自身 destination-out 减 alpha（复用 CHG-014 `erase` 原语 for_each_covered_pixel，减到 0 变透明露底图），最后合成。**擦的是这个对象的像素、不是底图**；划过空白无对象则什么都不做。
- 因 erased 在对象相对坐标里，**拖走/改形被擦对象时擦除跟着一起走**（事实 18 用户实测"跟着挪动了"），不是留在原地的全局洞。缩放时被擦处默认按相对坐标一起变换（未单独实测，W-16.1 复核）。
- 一次按下到松开整段橡皮＝一次突变（动了若干对象的 erased）＝一条撤销快照（REQ-003）；撤销恢复这些对象擦除前状态、被擦处逐像素复现（用户点名"擦除的部分还能撤销"）。
- 被擦对象几何一字不改，仍完整可选中/移动/拖控制点（REQ-004 hit 只看几何）。
- 烧录图新建的空对象层上（REQ-009），橡皮只擦得掉新画对象（旧标注是底图像素、无 erased 可记），擦不掉烧录进底图的旧标注。

**验证**：无头——对象带一条 erased 擦半条箭头，另半条逐像素还在、被擦段变透明露底图；erase 后 move_obj 平移，被擦段跟着平移（相对坐标不脱开）；一次拖擦多对象撤销后全部逐像素复现；矩形 erased 区内擦干净、区外一字不动；erased 对自身 tile destination-out == CHG-014 `erase` 同笔迹逐像素相等（H1 含橡皮）；划过空白无对象 erased 不新增；烧录图新建层底图旧烧录像素逐像素不变。

## REQ-006 实现与验证

**侧车持久化（anno_store.rs 新增 + capture.rs 清理同步）**：
- 布局：`shots/anno/<shot-stem>.json`（ObjectList 序列化，serde_json）＋ `shots/anno/<shot-stem>.base.png`（未烧录底图，image encode PNG）。`<shot-stem>` = 历史 PNG 文件名去 .png（`<pid>-<seq>-<epoch>`）。子目录避开 list 误读。
- `save_sidecar(dir, shot_path, base, objects)`：写 json + base.png（原子替换：先写临时再 rename）。`load_sidecar(dir, shot_path) -> Option<(RgbaImage, ObjectList)>`：读回，损坏/缺失/版本不符返回 None（优雅退回 REQ-009，停止条件 7）。json 带 schema_version 字段便于将来迁移。
- **list 过滤（capture.rs:402）**：`list()` 补 `.filter(|e| e.path().is_file())`——子目录 anno/ 不被当成 Shot（事实 10：现 list 读每个条目含目录）。这是侧车放子目录的硬前置。
- **清理同步（事实 11）**：`age_shots`（app.rs:1337）删 retain 返回的每个 PNG 后，补删其侧车对（anno_store::remove_sidecar）；`clear_all`（capture.rs:382）现只删文件不递归子目录→补清 anno/ 子目录（删里面文件再删目录，或递归）。**不留孤儿**。
- 接线：确认钉住带标注时 save_sidecar；编辑提交重写历史 PNG 时同步 save_sidecar；pin_from_history/F2 钉出来时 load_sidecar，有则建 Pin.anno（base+objects）可续编辑，无则走 REQ-009 烧录图新建。
- 依赖：ice-app/Cargo.toml 加 `serde_json.workspace = true`（既有 workspace crate，事实 12，零新增 crate 名）；AnnoObject/ObjectList derive Serialize/Deserialize。
- 隐私：侧车只落用户截图底图＋标注几何，不落密钥/译文内容，日志不落用户内容（NFR-3）。

**验证**：无头——save→load 往返 objects 与 base 逐像素/逐字段相等；损坏 json load 返回 None 不崩；list 含 anno 子目录时只返回 PNG 文件不返回子目录；age_shots 删 PNG 后侧车对也没了；clear_all 后 anno 子目录清空；侧车文件不被 next_unpinned 选中。

## REQ-007 实现与验证

**按钮表（pin.rs + pill.rs）**：`PinAction` 加 `Annotate`（label "标注"、menu_label "标注这张图"）；`BAND_ACTIONS` 3→4（末位追加，既有三颗顺序不动）、`ACTIONS` 4→5（Annotate 插在 Close 前）；`exactly_four_actions` 改 `exactly_five_actions` 锁新长度，注释写明改判来源（CHG-011 Q-006 → CHG-015 REQ-003 → CHG-016 REQ-007）。`PinIntent` 加 `Annotate`；pill_buttons 读表自动多一颗，药丸宽度 width_for(4) 同源。编辑态中药丸不 show（REQ-008），翻译禁用走 ui.add_enabled_ui 既有口径。

**验证**：无头——表长 4/5、顺序断言、Annotate 在 Close 前、label 文案、intent 映射。

## REQ-008 实现与验证

**编辑态（pin.rs + state.rs + selection.rs 抽公共）**：`Pin.editor: bool`（状态层）。点 Annotate → editor=true、band_open 收（药丸不 show）。贴图窗 show 里 editor 档：
- 窗尺寸不变（图片尺寸）；工具条以 Area 浮在图片下缘内侧（圆角药丸，与遮罩同风格），下缘放不下翻上缘——复用遮罩那条翻转判据。
- **工具条与手势抽公共**：把 selection.rs 里"八工具图标 + 粗细色板 + 复制/保存/对勾 + 工具态手势分派"抽成共享函数（遮罩闭包与编辑器闭包各调一次），ToolState 按 pin id 存根 ctx data（照 selection 那套键控，D-46 教训：状态各窗自持不绕主窗回传）。**这是本包最大重构面**，抽完遮罩行为必须逐项不变（V-002 兜底）。编辑器手势多了 REQ-004 的选中/控制点分支（无工具时）。
- 手势：有工具画层/建对象（point_to_px 漏斗，编辑器 ppp 取贴图窗当前帧）、无工具选中/拖控制点/拖窗。esc 层级：输入→工具/取消选中→关编辑器（提交）。
- **提交**：对勾或末层 esc → 烧录 base+render(objects)（composite_into）重写 pin.path 那一个 PNG（原子替换，不新增历史 PNG 文件）＋同步 save_sidecar（REQ-006）；editor=false、药丸恢复。提交时若该张识别/翻译在飞：bump recognize_gen 作废回信（既有 generation 机制，事实 7），stage 回 Idle。
- 编辑期间右键菜单「翻译」禁用；复制/保存走 displayed（anno 档，取 base+render 合成）。撤销/重做接 REQ-003 同一套。

**验证**：无头——提交后盘上 PNG == base+render(objects) 烧录；提交不新增历史 PNG 文件数（侧车是 anno 子目录另算）；在飞作废后旧回信被丢（generation 断言）；editor 档不产生选区/认窗手势（形状核查）；esc 三层顺序；工具条抽公共后遮罩形状断言全保持。真机 W-16.1 验手感。

## REQ-009 实现与验证

**显示优先级＋折叠＋烧录新建（pin.rs + state.rs）**：
- `displayed()` 优先级 **anno(base+render(objects) 合成) > product > source > file**（事实 6，承接 CHG-015 T-003 停车那套）。anno 档返回 Owned(合成图)；`visual_of` 缓存键带 objects.revision（kind3），对象变即重建纹理。复制/保存已走 displayed，自动一致不开新接缝。
- 合成用 composite_into 一份份出（显示档每次 revision 变才算一次，不是每帧）。
- **折叠**：翻译成品回信落 Pin.product 时 anno 置 None（成品已含烧录标注，事实 8）。识别底图 source 回信不折叠（source 是翻译链中间物，显示仍走 anno）。
- **烧录图新建**：点 Annotate 时 anno 为 None（F2 无侧车的/成品折叠后的/侧车损坏的）→ 取 displayed() 当前像素 clone 为 base、建**空 ObjectList**（尺寸=底图尺寸），Anno 成立。此后与 REQ-008 同路。橡皮擦除/删除只作用新对象（列表里只有新对象），擦不掉也删不掉烧录进底图的旧标注（旧标注是底图像素、不是对象、无 erased 可记）。文档写明边界。

**验证**：无头——空列表确认不建 Anno；带对象确认后 displayed == base+render 且与盘上烧录 PNG 逐像素同；revision 变缓存键变；成品落位折叠 anno；烧录图建空列表后 displayed == 烧录图+新笔；橡皮擦新对象旧烧录像素不变；copy_pin 取到的与显示一致。

## REQ-010 实现与验证

**回归与静态**：cargo test --workspace 全绿（无标注贴图逐手势不变）；clippy -D warnings；cargo tree -p ice-app -e normal 与基线逐行同（serde_json 是既有 workspace crate，加进 ice-app 不新增 crate 名——tree 里 serde_json 本就经 ice-core 在图内，核对无新名字）；./sdd scan 过；不新增 config 键（grep 配置结构断言，对象列表/编辑态/撤销栈都是运行态，侧车是落盘文件不是 config 键）。工具条抽公共后遮罩形状断言（写回顺序、像素漏斗、认窗门、对勾最后）全部保持。翻译链只动取图优先级+提交重写盘+generation 三处接缝。

## 影响与回滚

- 涉及仓库：仅 ice-translator。文件：annotate.rs（重写）/anno_store.rs（新增）/capture.rs/selection.rs/pin.rs/pill.rs/state.rs/app.rs/Cargo.toml + docs/06、docs/07。
- 兼容性：历史 PNG 格式不变（仍烧录 RGBA PNG）；config 不变；旧贴图（无侧车）走 REQ-009 烧录图新建；**新增落盘面 = shots/anno/ 子目录（侧车 json+base.png），用户已批（Q-002）**。
- 数据影响：编辑提交覆写既有历史 PNG（同路径同文件名）＋同步侧车；7 天/200 张老化与一键清除**同步清侧车**（不留孤儿）。
- 回滚：本包各任务独立 commit，回滚即 revert 对应提交；anno 字段去掉后 Pin 回到只持 path 现状；侧车子目录可整删（历史 PNG 不受影响）。
- 改判登记：CHG-014 像素引擎/CHG-015 脏矩形撤销由本包取代（架构改判，批本包即批改判）；CHG-011 Q-006 按钮数（3→4/4→5）docs/07 与本包 spec 写明，合并走查时 revise CHG-011；橡皮＝**绑定到被擦对象的透明擦除、随对象移动**——先前 spec 写"整对象删"是我对 某截图工具 的**误判**，用户 2026-11-18 真机两次验证纠正（部分擦＋拖走被擦箭头缺口跟着挪），与 CHG-014 已批 `erase` 一致（这是纠错不是改判），docs/07 登记这条更正。

## 验证层次

- 本地无头：V-001（ice-app 定向：对象引擎/H1 逐像素含橡皮/撤销重做/命中控制点/橡皮绑定对象擦除随对象移动/侧车往返/按钮表/显示折叠）、V-002（workspace 全量回归）、V-003（clippy）、V-004（依赖零增量）、V-005（scan）、V-006（形状 grep：composite_into 单一定义/exactly_five_actions/list is_file 过滤/侧车子目录）。
- 业务验收：V-007 / W-16.1 用户真机——遮罩画几笔 cmd/ctrl+z 逐笔退、shift+z 重做；钉住后点「标注」续画、选中上次画的箭头拖端点改向、拖框角改大小、改色板即改选中那笔、Delete 删；橡皮部分擦（擦半条箭头另半条还在、擦掉的能撤销）；重启后/F2 历史图接着选中编辑（侧车）；对勾/esc 关闭改动还在、F2 再钉出来最新；翻译链不回退。观感手感只有用户眼睛能判。
- 无沙箱/生产操作；不 push（待批队列纪律）。