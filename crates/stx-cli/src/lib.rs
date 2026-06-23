//! stx orchestration, exposed as a library so both the CLI (`main.rs`) and the
//! HTTP service (`stx-server`) drive the exact same engine: submit, race, and
//! the transaction autopsy.

pub mod agent_tools;
pub mod config;
pub mod diagnose;
pub mod engine;
