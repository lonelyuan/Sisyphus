//! Sisyphus Core —— 唯一事实来源。
//!
//! 感知平面（Tauri App）与反思平面（MCP server）都依赖本 crate，
//! 保证"怎么写/读一条事件"只有一份逻辑（见 docs/spec/architecture.md）。
//!
//! **依赖卫生铁律**：本 crate 只放跨端（含 Android NDK）能编译的依赖。
//! 异步运行时（tokio）、MCP（rmcp）只属于 mcp crate，绝不进这里。

pub mod db;
pub mod rule_engine;
pub mod rules;
pub mod ingest;
pub mod context;
pub mod category;
pub mod artifacts;
pub mod vault;
pub mod knowledge;
pub mod sources;
pub mod intervention;
pub mod timeline;
pub mod scheduler;
pub mod lifeindex;

pub use ingest::{ingest_event, capture_text, NewEvent};
pub use rule_engine::{DailyGoal, Finding, RuleContext, RuleEngine};
