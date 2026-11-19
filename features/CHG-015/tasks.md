Change: CHG-015
Version: v0.1
Spec-Hash: ac7c69999e971ee5f3cbcc613df42e632e783b47f2f115541dcd2eeef9b3286f

# Tasks（由 tasks.json 生成，勿直接修改）
- [x] T-001 annotate.rs 撤销栈：UndoEntry 脏矩形前像（单击一 part、拖拽整段累积）、落层统一先抄前像、undo() 逐像素贴回 bump revision、30 条+64MB 双上限丢最旧；无头断言恢复逐像素精确（H1）、铅笔三段一笔撤、橡皮擦痕可撤、双上限、空栈不动 | ice-translator | REQ-001
  Code/Test: crates/ice-app/src/system/annotate.rs
- [x] T-002 selection.rs 遮罩撤销键：工具态 Cmd+Z/Ctrl+Z 调 Layer::undo，文字输入开着让位 TextEdit，空栈无声；无头断言键位分派与输入让位 | ice-translator | REQ-001
  Code/Test: crates/ice-app/src/system/selection.rs
- [ ] T-003 state.rs Pin.anno（base+layer 所有权转移）：confirm_capture 层非空才建、displayed 优先级 anno>product>source>file、visual_of 缓存键带 revision、成品回信折叠 anno、copy/save 走 displayed 自动一致；无头断言空层不建 Anno、显示==烧录 PNG、折叠、缓存刷新 | ice-translator | REQ-002
- [ ] T-004 pin.rs/pill.rs 标注入口：PinAction::Annotate、BAND_ACTIONS 3→4 末位追加、ACTIONS 4→5 插 Close 前、exactly_five_actions 改判断言、PinIntent::Annotate、药丸宽度同源；无头断言表长顺序文案映射 | ice-translator | REQ-003
- [ ] T-005 工具条与手势抽公共 + 贴图编辑态：selection.rs 八工具条/手势抽共享函数（遮罩逐项不变）、Pin.editor、药丸收起、工具条浮图片下缘内侧翻上缘、有工具画层无工具拖窗、esc 输入→工具→提交关闭、提交烧录重写历史 PNG（不新增文件）、在飞识别/翻译 generation 作废、编辑期翻译禁用；无头断言提交重写/作废/esc 层级/遮罩形状保持 | ice-translator | REQ-004, REQ-001
- [ ] T-006 烧录图新建层：anno 为空时取 displayed 像素为 base 建空层（F2 历史/成品折叠后/重启后同路），橡皮只擦新层；无头断言旧烧录像素不变、新笔可撤、提交后 F2 再钉见新笔 | ice-translator | REQ-005
- [ ] T-007 docs/06 加 W-15.1 真机走查与无头 TC + docs/07 登记撤销栈/编辑态/改判（CHG-011 Q-006 按钮数、CHG-014 Q-004/Q-005 另包兑现）+ 静态 scan/cargo tree 零增量/clippy 全绿；旧证据并入合并走查一次重录 | ice-translator | REQ-006
