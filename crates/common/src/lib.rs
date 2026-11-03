//! Shared types, error handling, logging, and IPC definitions.
//!
//! **Ownership**: Jointly maintained by all teams. Changes to public types
//! require review from every affected team plus at least one experienced engineer.
//!
//! See `docs/TECH_ARCHITECTURE.md` §3.1.

#![forbid(unsafe_code)]

pub mod dom;
pub mod error;
pub mod ipc;
