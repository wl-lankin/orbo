//! Orbo core: detects files, suggests conversions and runs them locally.

pub mod ai;
pub mod analyze;
pub mod catalog;
pub mod chains;
pub mod engines;
pub mod format;
pub mod jobs;
pub mod output;
pub mod rules;
pub mod tools;
pub mod watch_rules;

pub use analyze::{analyze, Analysis};
pub use catalog::{Catalog, Target};
pub use format::{Format, Kind};
pub use jobs::{convert_blocking, JobEvent, JobQueue, JobSpec, Options};
pub use rules::{Rules, Suggestions};

/// Analyzes a file and returns ring suggestions using the built-in catalog and rules.
pub fn suggest(path: &std::path::Path) -> anyhow::Result<(Analysis, Suggestions)> {
    let a = analyze(path)?;
    let s = Rules::builtin().suggest(&Catalog::current(), &a);
    Ok((a, s))
}
