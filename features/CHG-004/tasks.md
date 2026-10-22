Change: CHG-004
Version: v0.1
Spec-Hash: 0a74661dd24107ebe2a576aacde34be301ddfcaa457fd8a54484689f3bd057e2

# Tasks（由 tasks.json 生成，勿直接修改）
- [ ] T-001 三条 HYPOTHESIS 最小实测：无边框边缘缩放、逐帧 InnerSize 补间、with_transparent 副作用 | ice-translator | REQ-001, REQ-004, REQ-007
- [ ] T-002 尺寸命令修正：清 MinInnerSize、允许缩放、MIN_COMPACT_SIZE 与 sanitize 下限 | ice-translator | REQ-001
- [ ] T-003 system::screen：AppKit 可用区探针、坐标翻转与选屏逻辑（非 macOS 回退） | ice-translator | REQ-003
- [ ] T-004 views::compact：把手三处、输入行、译文定高 ScrollArea、动作行（引擎分段/停靠/置顶/设置/Esc） | ice-translator | REQ-002, REQ-005
- [ ] T-005 吸附：snap/cycle_edge/fit_into_area 纯函数 + DragSettler 静止判定 + 松手接线 | ice-translator | REQ-003
- [ ] T-006 形变动画：Morph 补间、35% 那帧切装饰、反向插值、末帧校正与落盘 | ice-translator | REQ-004
- [ ] T-007 自动反向：auto_reverse_target 纯函数 + submit 钩子 + 提示条 | ice-translator | REQ-006
- [ ] T-008 失焦半透明：dim_alpha 四组组合 + with_transparent + 设置页开关接线 | ice-translator | REQ-007
- [ ] T-009 剪贴板即翻：Watcher 七例判定、轮询挂点、自写回环防护、空闲 CPU 两次采样 | ice-translator | REQ-008
- [ ] T-010 几何即时落盘、离屏夹回与插拔恢复 | ice-translator | REQ-009
- [ ] T-011 文档回写、走查取证与 P5 验收记录 | ice-translator | REQ-010
