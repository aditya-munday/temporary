//! CodeCurrency core engine.
//!
//! Phase 1 proves one claim: a single command can build, test, measure, and
//! explain a solution, and produce a result anyone else can replay to the same
//! answer. Everything here serves that claim.

pub mod capsule;
pub mod engine;
pub mod explain;
pub mod host;
pub mod hotspot;
pub mod latency;
pub mod measure;
pub mod memory;
pub mod problem;
pub mod ranking;
pub mod repeat;
pub mod sandbox;
pub mod scaling;
pub mod throughput;
pub mod toolchain;
pub mod topdown;
pub mod trust;

pub use capsule::*;
