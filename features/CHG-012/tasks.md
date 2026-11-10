Change: CHG-012
Version: v0.1
Spec-Hash: 505a688625fb1b9ec48ee2b9b7763460ab3ebbb0087fbd861caccabed758d24a

# Tasks（由 tasks.json 生成，勿直接修改）
- [ ] T-001 capture::next_older 纯函数（复用 sorted，锚点用路径）：空锚点=最新、有锚点=下一张更早、最早=None、锚点消失=回退最新 | ice-translator | REQ-001
- [ ] T-002 app.rs ReopenLast 三档分流：空历史/最早/钉满各有提示，最早与钉满都不推进游标 | ice-translator | REQ-002
- [ ] T-003 Pin.history_browse 标记 + AppState.browse_anchor 运行态游标 + 只给 F2 的 browse_older 薄包装 + close_pin 关到最后一张才重置（F1/F3 不计入） | ice-translator | REQ-003
- [ ] T-004 docs/06 加 W-12.1 与无头 TC + docs/07 登记改口 CHG-009 REQ-006/REQ-008/Q-011 与落地次序（不动 CHG-009 任何文件） | ice-translator | REQ-004
