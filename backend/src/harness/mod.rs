//! Bounded, simulator-backed experiments. Independent of the legacy Agent UI.
pub mod alignment;
pub mod compiler;
pub mod contract;
pub mod equipment;
pub mod http;
pub mod job;
pub mod rotation;
pub mod run_http;
pub mod run_loop;
pub mod run_schema;
pub mod run_store;
pub mod run_tools;

#[cfg(test)]
#[path = "../../tests/harness/run_runtime.rs"]
mod run_runtime_tests;

#[cfg(test)]
#[path = "../../tests/harness/protocol.rs"]
mod protocol_tests;
