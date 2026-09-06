//! GA 优化器模块
//!
//! 模块结构：
//! - `loop_config` — 前后端共享的循环配置 JSON schema（LoopConfig）
//! - `analyze`     — 宏文本分析器：抽取可调阈值 (TunableParam)
//!
//! HTTP 接口在 main.rs 中注册：
//! - POST /api/optimizer/analyze  — 解析宏文本并返回可调参数列表

pub mod analyze;
pub mod archive;
pub mod ga;
pub mod loop_config;
pub mod render;
pub mod rule_pool;
pub mod runtime;
pub mod struct_ga;
