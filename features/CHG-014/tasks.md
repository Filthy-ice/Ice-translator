Change: CHG-014
Version: v0.2
Spec-Hash: 404ac5357b9002b47583674be4b80008bb38b55de0f7f7f0670ca15ca316ef5e

# Tasks（由 tasks.json 生成，勿直接修改）
- [ ] T-001 新增 system/annotate.rs：透明光栅层 Layer + 落像素纯函数（rect_stroke/line/arrow/pencil）+ erase(destination-out 笔刷) + 预览矢量形状；无头断言落层像素形状与部分擦除（擦半条框另半条仍在、背景不被改） | ice-translator | REQ-001, REQ-002, REQ-004
- [ ] T-002 annotate.rs 补单击/区域类落层：number_badge(实心圆+白数字自增)、text(仓里现成字形光栅落层不联网)、mosaic(拖出区域从冻结背景盒式打码写层)；无头断言序号自增与马赛克块内均值色 | ice-translator | REQ-002
- [x] T-003 粗细档 THICKNESS 与色板 PALETTE 常量一处（默认中档/默认红+红黄蓝绿白黑）+ 药丸下拉选择存运行态不进 config；落层与预览同源读；无头断言换粗细线宽变/换色色变/默认红 | ice-translator | REQ-003
  Code/Test: crates/ice-app/src/system/annotate.rs, crates/ice-app/src/system/selection.rs
- [ ] T-004 selection.rs：标注层按 display+frozen_id 存 IdTypeMap（照 load_selection 那套）+ show 里背景后叠层纹理(脏才重传)与预览矢量 + 工具态 press/drag/release 分派；无头断言空层与现状逐像素同、不串屏、关窗清 | ice-translator | REQ-001, REQ-006
- [ ] T-005 state.rs crop_selection 裁出背景后把标注层对应区域 alpha 合成进裁图（唯一合成点）；app.rs 确认三兄弟取该屏标注层；无头断言三出口带标注一致、空层裁图==现状、合成不越选区界 | ice-translator | REQ-001
- [ ] T-006 selection.rs 药丸改版：BAR_ICONS 扩八工具+Copy+Save+Check(对勾最后)、bar_width 同源扩不铺满锚点宽、icon_button 加八种线稿(①=圆圈+1 不依赖字符)+悬停提示、点工具不算框选手势；改判 the_bar_has_three_icons_in_this_order 成新顺序 | ice-translator | REQ-005
- [ ] T-007 selection.rs 工具态/选区态互斥接线：active_tool 运行态各窗自持；None 走既有 Selection 逐手势不变、Some 走 annotate 不碰 next.rect；再点当前工具或 esc 回 None(esc 先退工具不取消遮罩)；工具态选区仍可见可确认；无头断言三条 | ice-translator | REQ-006
- [ ] T-008 docs/06 加 W-14.x 真机走查与无头 TC + docs/07 登记新增标注层与不变边界（不改 CHG-009/010/011/012/013 快照、零新增 crate/config、旧证据并入合并走查一次重录）；静态 scan/cargo tree 零增量/clippy 全绿 | ice-translator | REQ-007
