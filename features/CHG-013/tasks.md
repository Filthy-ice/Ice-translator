Change: CHG-013
Version: v0.1
Spec-Hash: 3a7a6837d82c1f37bc4a939a72aeb70b0dc7331682e1096a45db189ecfc80060

# Tasks（由 tasks.json 生成，勿直接修改）
- [ ] T-001 去掉收起—还原整对（一笔原子改动，编译一起过）：app.rs:1798-1801 去 capture::hide 那一支；capture.rs show_again 拆成只唤醒（删 Visible(true) 还原段，保留 wake_commands/Wake/RequestPaintWhileHidden），hide/Hidden 若无调用方则删；jobs.rs:434/470 改只叫醒帧循环、Job::Freeze/ScreenCapture 去 hidden 字段；state.rs:1554 去 capture_hidden_record | ice-translator | REQ-001
- [ ] T-002 单测改判 + REQ-006 唤醒不回归守卫：state.rs:2938/2952 两条断言 hidden.viewports.len()==3 改判成「submit_capture 后不再产生 hide 列表/不再发 Visible(false)」；capture.rs:789-790 wake_commands 断言保留并加强「仍发 RequestPaintWhileHidden + request_repaint」；静态 grep 确认 RequestPaintWhileHidden 仍在 capture.rs（事实 10，删了即回归 D-21） | ice-translator | REQ-001, REQ-003
- [ ] T-003 自家窗可见进图＝行为上「不收起」即实现（REQ-002）：不新增任何替用户隐藏的写法、不新增提示控件、不新增配置字段；静态确认无新增隐藏路径、cargo tree 比对基线 ef92028 零增量、config.rs 无新增字段；已知限制文字落 T-004 | ice-translator | REQ-002
- [ ] T-004 docs/06 加 W-13.1 真机走查与无头 TC + docs/07 登记改口 CHG-009 REQ-010/Q-003（(a) 发起前收起 → (c) 不收起、登记已知限制）、真机依据（半透明背景框 + 还原 1~3s 白屏）与落地次序（CHG-009/010 都 ACCEPTED 之后才实施），不动 CHG-009/010 任何文件 | ice-translator | REQ-002, REQ-003
