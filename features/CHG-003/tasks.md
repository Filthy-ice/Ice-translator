Change: CHG-003
Version: v0.1
Spec-Hash: 84a04e7d9c990d045a19e52310be07f856a31b043669a7bf05862269ee111dc7

# Tasks（由 tasks.json 生成，勿直接修改）
- [x] T-001 术语迁入 config.toml（[[term]]）与旧 glossary.toml 只读导入 | ice-translator | REQ-006
  Code/Test: crates/ice-core/src/glossary.rs, crates/ice-core/src/config.rs, crates/ice-core/src/lib.rs
- [x] T-002 ice-app 模块骨架：AppState、四页签、状态条、主题与字号 | ice-translator | REQ-001
  Code/Test: crates/ice-app/src/state.rs, crates/ice-app/src/views/mod.rs, crates/ice-app/src/theme.rs, crates/ice-app/src/app.rs
- [x] T-003 翻译闭环：任务派发、request_id 竞态丢弃、加载态、20k 字符实测 | ice-translator | REQ-002
  Code/Test: crates/ice-app/src/jobs.rs, crates/ice-app/src/views/full.rs, crates/ice-app/src/state.rs
- [x] T-004 剪贴板两个按钮：复制译文 / 复制对照，含空译文保护 | ice-translator | REQ-003
  Code/Test: crates/ice-app/src/system/clipboard.rs, crates/ice-app/src/views/full.rs
- [x] T-005 错误映射与动作按钮（去设置 / 改用机翻） | ice-translator | REQ-004
  Code/Test: crates/ice-app/src/state.rs, crates/ice-app/src/jobs.rs, crates/ice-providers/src/error.rs, crates/ice-providers/src/machine.rs
- [x] T-006 设置页：供应商预设、密钥写钥匙串、主题三态、字号、额外语言 | ice-translator | REQ-005
  Code/Test: crates/ice-app/src/views/settings.rs, crates/ice-app/src/system/secrets.rs
- [x] T-007 历史页与设置页术语分区接线 | ice-translator | REQ-006
  Code/Test: crates/ice-app/src/views/history.rs, crates/ice-app/src/views/settings.rs, crates/ice-core/src/config.rs
- [x] T-008 快捷键映射、关窗常驻与退出落盘 | ice-translator | REQ-007
  Code/Test: crates/ice-app/src/system/hotkeys.rs, crates/ice-app/src/mode.rs, crates/ice-app/src/app.rs
- [x] T-009 单实例互斥：PID 锁、陈旧锁接管、只删自己的锁 | ice-translator | REQ-009
  Code/Test: crates/ice-app/src/system/single_instance.rs, crates/ice-app/src/main.rs
- [x] T-010 文档回写、走查取证与 P4 验收记录 | ice-translator | REQ-008
  Code/Test: docs/02-architecture.md, docs/03-ui-design.md, docs/05-dev-guide.md, docs/06-test-cases.md, docs/07-acceptance.md, docs/traceability.md, docs/adr/ADR-005-keychain-for-secrets.md
