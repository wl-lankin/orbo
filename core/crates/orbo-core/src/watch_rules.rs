//! Decision logic for watched folders: which new files get converted, and when.
//! No file system watching here; the app feeds in paths and submits the returned jobs.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::analyze::analyze;
use crate::catalog::Catalog;
use crate::output;

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchRule {
    pub id: String,
    pub folder: PathBuf,
    pub target: String,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub include_subfolders: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WatchJob {
    pub input: PathBuf,
    pub target: String,
    pub preset: Option<String>,
}

pub const POLL: Duration = Duration::from_millis(500);
pub const STABLE_FOR: Duration = Duration::from_millis(1500);
pub const MAX_WAIT: Duration = Duration::from_secs(600);
const HANDLED_TTL: Duration = Duration::from_secs(600);
const LEDGER_CAP: usize = 2000;

/// Partial downloads, temp files, lock files and hidden files are never converted.
pub fn ignored_name(name: &str) -> bool {
    if name.starts_with('.') || name.starts_with("~$") {
        return true;
    }
    let lower = name.to_lowercase();
    if matches!(lower.as_str(), "thumbs.db" | "desktop.ini") {
        return true;
    }
    const EXT: [&str; 9] = ["crdownload", "part", "download", "tmp", "partial", "temp", "opdownload", "filepart", "swp"];
    Path::new(&lower).extension().and_then(|e| e.to_str()).is_some_and(|e| EXT.contains(&e))
}

#[cfg(windows)]
fn hidden_attr(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::metadata(p).map(|m| m.file_attributes() & 0x2 != 0).unwrap_or(false)
}
#[cfg(not(windows))]
fn hidden_attr(_: &Path) -> bool {
    false
}

fn strip_counter(s: &str) -> &str {
    match s.rsplit_once(' ') {
        Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => head,
        _ => s,
    }
}

/// True when the file name already ends in one of Orbo's output suffixes (also with the " 2" collision counter).
pub fn carries_suffix(path: &Path, suffixes: &[String]) -> bool {
    let stem = output::stem_of(path);
    let base = strip_counter(&stem);
    suffixes.iter().any(|s| !s.trim().is_empty() && (stem.ends_with(s.as_str()) || base.ends_with(s.as_str())))
}

/// Resolves symlinks in the directory part so event paths and configured folders compare equal.
fn norm(p: &Path) -> PathBuf {
    match (p.parent(), p.file_name()) {
        (Some(dir), Some(name)) => dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).join(name),
        _ => p.to_path_buf(),
    }
}

/// Waits until `len` reports the same non-zero size for `stable_for`. `len` returning None (file gone) aborts.
/// Time is counted in `poll` steps through `sleep`, so tests can run without a clock.
pub fn wait_stable(mut len: impl FnMut() -> Option<u64>, mut sleep: impl FnMut(Duration), poll: Duration, stable_for: Duration, max_wait: Duration) -> bool {
    let mut last = match len() {
        Some(l) => l,
        None => return false,
    };
    let (mut elapsed, mut steady) = (Duration::ZERO, Duration::ZERO);
    while elapsed < max_wait {
        sleep(poll);
        elapsed += poll;
        let Some(now) = len() else { return false };
        if now == last && now > 0 {
            steady += poll;
            if steady >= stable_for {
                return true;
            }
        } else {
            steady = Duration::ZERO;
            last = now;
        }
    }
    false
}

/// Side effects the matcher needs, injectable for tests.
pub struct Io<'a> {
    pub len: &'a dyn Fn(&Path) -> Option<u64>,
    pub sleep: &'a dyn Fn(Duration),
    pub paused: &'a dyn Fn() -> bool,
}

pub fn fs_len(p: &Path) -> Option<u64> {
    // Opening catches files another program still holds exclusively (Windows).
    std::fs::File::open(p).ok()?;
    std::fs::metadata(p).ok().map(|m| m.len())
}

#[derive(Default)]
struct State {
    outputs: VecDeque<PathBuf>,
    output_set: HashSet<PathBuf>,
    handled: HashMap<PathBuf, Instant>,
}

pub struct WatchMatcher {
    rules: Vec<WatchRule>,
    out_folder: Option<PathBuf>,
    /// Output naming "suffix" appends " (Title)" to the file name.
    title_suffix: bool,
    state: Mutex<State>,
}

impl WatchMatcher {
    pub fn new(rules: &[WatchRule], out_folder: Option<PathBuf>, title_suffix: bool) -> WatchMatcher {
        let rules = rules
            .iter()
            .filter(|r| r.enabled)
            .map(|r| WatchRule { folder: r.folder.canonicalize().unwrap_or_else(|_| r.folder.clone()), ..r.clone() })
            .collect();
        let out_folder = out_folder.map(|f| f.canonicalize().unwrap_or(f));
        WatchMatcher { rules, out_folder, title_suffix, state: Mutex::new(State::default()) }
    }

    /// Remembers a file Orbo wrote so it is never converted again.
    pub fn note_output(&self, path: &Path) {
        let p = norm(path);
        let mut st = self.state.lock().unwrap();
        if st.output_set.insert(p.clone()) {
            st.outputs.push_back(p);
            if st.outputs.len() > LEDGER_CAP {
                if let Some(old) = st.outputs.pop_front() {
                    st.output_set.remove(&old);
                }
            }
        }
    }

    /// Carries the ledger and the handled files over when the matcher is rebuilt after a settings change.
    pub fn inherit(&self, old: &WatchMatcher) {
        let (outputs, handled) = {
            let o = old.state.lock().unwrap();
            (o.outputs.clone(), o.handled.clone())
        };
        let mut st = self.state.lock().unwrap();
        st.output_set.extend(outputs.iter().cloned());
        st.outputs.extend(outputs);
        st.handled.extend(handled);
    }

    fn is_output(&self, path: &Path) -> bool {
        let st = self.state.lock().unwrap();
        path.ancestors().any(|a| st.output_set.contains(a))
    }

    /// Name and location checks only. Returns the index of the rule that covers `path`.
    pub fn rule_for(&self, path: &Path) -> Option<usize> {
        if let Some(out) = &self.out_folder {
            if path.starts_with(out) {
                return None;
            }
        }
        let idx = self.rules.iter().position(|r| {
            let Ok(rel) = path.strip_prefix(&r.folder) else { return false };
            let depth = rel.components().count();
            depth >= 1 && (r.include_subfolders || depth == 1)
        })?;
        let rule = &self.rules[idx];
        let rel = path.strip_prefix(&rule.folder).ok()?;
        if rel.components().any(|c| ignored_name(&c.as_os_str().to_string_lossy())) {
            return None;
        }
        if let Some(out) = &self.out_folder {
            // A rule on the output folder itself would feed on its own results.
            if &rule.folder == out || rule.folder.starts_with(out) {
                return None;
            }
        }
        if self.is_output(path) {
            return None;
        }
        let catalog = Catalog::current();
        let target = catalog.get(&rule.target)?;
        let mut suffixes = vec![target.suffix.clone()];
        if self.title_suffix {
            suffixes.push(format!(" ({})", target.title));
        }
        if carries_suffix(path, &suffixes) {
            return None;
        }
        if !path.is_file() || hidden_attr(path) {
            return None;
        }
        Some(idx)
    }

    fn claim(&self, path: &Path) -> bool {
        let mut st = self.state.lock().unwrap();
        let now = Instant::now();
        if st.handled.len() > 1024 {
            st.handled.retain(|_, t| now.duration_since(*t) < HANDLED_TTL);
        }
        match st.handled.get(path) {
            Some(t) if now.duration_since(*t) < HANDLED_TTL => false,
            _ => {
                st.handled.insert(path.to_path_buf(), now);
                true
            }
        }
    }

    fn release(&self, path: &Path) {
        self.state.lock().unwrap().handled.remove(path);
    }

    /// Full pipeline for one event path: filters, waits for a stable size, checks compatibility.
    /// Returns the job to submit, or None. Calling it again for the same path yields None.
    pub fn process(&self, path: &Path, io: &Io) -> Option<WatchJob> {
        if (io.paused)() {
            return None;
        }
        let path = norm(path);
        let idx = self.rule_for(&path)?;
        if !self.claim(&path) {
            return None;
        }
        let stable = wait_stable(|| (io.len)(&path), |d| (io.sleep)(d), POLL, STABLE_FOR, MAX_WAIT);
        // Orbo's own result may have been reported while we waited.
        if !stable || (io.paused)() || self.rule_for(&path) != Some(idx) {
            self.release(&path);
            return None;
        }
        let rule = &self.rules[idx];
        let catalog = Catalog::current();
        let target = catalog.get(&rule.target)?;
        let analysis = analyze(&path).ok()?;
        if !target.accepts(&analysis) {
            return None;
        }
        Some(WatchJob { input: path, target: rule.target.clone(), preset: rule.preset.clone() })
    }
}
