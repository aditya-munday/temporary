//! Shared command helpers: consistent human/JSON rendering and the standard
//! "what went wrong and how to fix it" error shape.

pub mod asm;
pub mod build;
pub mod diff;
pub mod doctor;
pub mod experiment;
pub mod hotspot;
pub mod init;
pub mod latency;
pub mod matrix;
pub mod measure;
pub mod memory;
pub mod pareto;
pub mod principles;
pub mod profiles;
pub mod repeat;
pub mod replay;
pub mod run;
pub mod scaling;
pub mod test;
pub mod throughput;
pub mod topdown;
pub mod validate;

use crate::Format;

/// Print a payload either as a table (already rendered) or as JSON.
pub fn emit<T: serde::Serialize>(
    format: Format,
    table: impl FnOnce(),
    value: &T,
) -> anyhow::Result<()> {
    match format {
        Format::Table => {
            table();
            Ok(())
        }
        Format::Json => {
            println!("{}", serde_json::to_string_pretty(value)?);
            Ok(())
        }
    }
}

/// An error that knows how to fix itself.
///
/// Phase 1 requires errors to say what went wrong *and* how to fix it, so
/// failures are modelled explicitly rather than as bare strings.
#[derive(Debug)]
pub struct Fixable {
    pub what: String,
    pub fix: Vec<String>,
}

impl Fixable {
    pub fn new(what: impl Into<String>) -> Self {
        Fixable {
            what: what.into(),
            fix: Vec::new(),
        }
    }

    pub fn fix(mut self, step: impl Into<String>) -> Self {
        self.fix.push(step.into());
        self
    }

    pub fn into_error(self) -> anyhow::Error {
        let mut msg = self.what;
        if !self.fix.is_empty() {
            msg.push_str("\n\nhow to fix:");
            for (i, step) in self.fix.iter().enumerate() {
                msg.push_str(&format!("\n  {}. {step}", i + 1));
            }
        }
        anyhow::anyhow!(msg)
    }
}
