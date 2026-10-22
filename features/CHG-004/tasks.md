Change: CHG-004
Version: v0.3
Spec-Hash: 235deaf136a3d2a0f9d8585b72f05f2b8948470f70d9e26629c5f5872091fa9c

# Tasks（由 tasks.json 生成，勿直接修改）
- [x] T-001 三条 HYPOTHESIS 最小实测：无边框边缘缩放、逐帧 InnerSize 补间、with_transparent 副作用（已跑完三轮：H1/H3 成立；H2 不成立；窗内补间只能单向、双向必撞 ~1s 停摆；单发几何命令 20ms 落定 → 用户决定取消动画，FR-3.9 改为瞬时切换） | ice-translator | REQ-001, REQ-004, REQ-007
  Code/Test: crates/ice-app/examples/viewport_probe.rs, features/CHG-004/evidence/T-001-hypothesis-verdicts.md, features/CHG-004/evidence/T-001-probe.log, features/CHG-004/evidence/T-001-probe-v2.log, features/CHG-004/evidence/T-001-probe-v3.log
- [x] T-002 尺寸命令修正：MinInnerSize 按形态取值（不再清成 ZERO）、两种形态都允许缩放、MIN_COMPACT_SIZE 与 sanitize 下限 | ice-translator | REQ-001
  Code/Test: crates/ice-app/src/mode.rs, crates/ice-core/src/config.rs
- [x] T-003 system::screen：AppKit 可用区探针、坐标翻转与选屏逻辑（非 macOS 回退） | ice-translator | REQ-003
  Code/Test: crates/ice-app/src/system/screen.rs, crates/ice-app/Cargo.toml
- [x] T-004 views::compact：把手三处、输入行、译文定高 ScrollArea、动作行（引擎分段/停靠/置顶/设置/Esc） | ice-translator | REQ-002, REQ-005
  Code/Test: crates/ice-app/src/views/compact.rs, crates/ice-app/src/views/mod.rs, crates/ice-app/src/app.rs
- [x] T-005 吸附：snap/cycle_edge/fit_into_area 纯函数 + DragSettler 静止判定 + 松手接线 | ice-translator | REQ-003
  Code/Test: crates/ice-app/src/mode.rs, crates/ice-app/src/app.rs
- [ ] T-006 形态切换瞬时化：commands_for_mode 重写（Decorations 排在 InnerSize 之前、两种形态都可缩放、全量也下发 OuterPosition）、切换后回读真实几何并立即落盘、隐藏态只改配置；无头断言命令条数与顺序 | ice-translator | REQ-004
- [ ] T-007 自动反向：auto_reverse_target 纯函数 + submit 钩子 + 提示条 | ice-translator | REQ-006
- [ ] T-008 失焦半透明：dim_alpha 四组组合 + with_transparent + 设置页开关接线 | ice-translator | REQ-007
- [ ] T-009 剪贴板即翻：Watcher 七例判定、轮询挂点、自写回环防护、空闲 CPU 两次采样 | ice-translator | REQ-008
- [ ] T-010 几何即时落盘、离屏夹回与插拔恢复 | ice-translator | REQ-009
- [ ] T-011 文档回写、走查取证与 P5 验收记录 | ice-translator | REQ-010
