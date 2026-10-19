//! Orbo core: detects files, suggests conversions and runs them locally.

pub mod ai;
pub mod analyze;
pub mod appconfig;
pub mod catalog;
pub mod chains;
pub mod engines;
pub mod format;
pub mod integrations;
pub mod jobs;
pub mod output;
pub mod rules;
pub mod tools;
pub mod watch_rules;

pub use analyze::{analyze, Analysis};
pub use catalog::{Catalog, Target};
pub use format::{Format, Kind};
pub use jobs::{convert_blocking, convert_merge_blocking, JobEvent, JobQueue, JobSpec, MergeSpec, Options};
pub use rules::{Rules, Suggestions};

/// Analyzes a file and returns ring suggestions using the built-in catalog and rules.
pub fn suggest(path: &std::path::Path) -> anyhow::Result<(Analysis, Suggestions)> {
    let a = analyze(path)?;
    let s = Rules::builtin().suggest(&Catalog::current(), &a);
    Ok((a, s))
}

/// Like `suggest` for several files at once; with two or more files, merge targets that fit all of them appear.
pub fn suggest_many(paths: &[std::path::PathBuf]) -> anyhow::Result<(Vec<Analysis>, Suggestions)> {
    anyhow::ensure!(!paths.is_empty(), "Keine Datei angegeben");
    let analyses = paths.iter().map(|p| analyze(p)).collect::<anyhow::Result<Vec<_>>>()?;
    let s = Rules::builtin().suggest_many(&Catalog::current(), &analyses);
    Ok((analyses, s))
}
