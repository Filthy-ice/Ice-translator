Change: CHG-002
Version: v0.1
Spec-Hash: 88f16377d47b5f67e0e99073eb0d02650235d9092e5fbef544656d107f1f487c

# Tasks（由 tasks.json 生成，勿直接修改）
- [x] T-001 实现机翻通道与超长切块 | ice-translator | REQ-001
  Code/Test: crates/ice-providers/src/machine.rs, crates/ice-providers/src/lib.rs, crates/ice-providers/tests/live.rs
- [x] T-002 实现供应商预设与 AI 通道请求 | ice-translator | REQ-002
  Code/Test: crates/ice-core/src/config.rs, crates/ice-providers/src/ai.rs, crates/ice-providers/tests/live.rs
- [x] T-003 实现不可译内容保护与还原 | ice-translator | REQ-003
  Code/Test: crates/ice-providers/src/protect.rs
- [x] T-004 实现 AI 分段与段间上下文 | ice-translator | REQ-004
  Code/Test: crates/ice-providers/src/ai.rs, crates/ice-providers/src/protect.rs
- [x] T-005 实现错误映射与重试上限 | ice-translator | REQ-005
  Code/Test: crates/ice-providers/src/error.rs, crates/ice-providers/src/machine.rs, crates/ice-providers/src/ai.rs
- [x] T-006 落实密钥与日志边界 | ice-translator | REQ-006
  Code/Test: crates/ice-providers/src/ai.rs, crates/ice-core/src/config.rs, crates/ice-providers/Cargo.toml
- [x] T-007 撰写 ADR-004 并回写文档引用 | ice-translator | REQ-007
  Code/Test: docs/adr/ADR-004-machine-translate-endpoint.md, docs/02-architecture.md, docs/07-acceptance.md, docs/traceability.md, docs/05-dev-guide.md, docs/06-test-cases.md
